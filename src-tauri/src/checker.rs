//! Single HTTP check: one request, one outcome.
//!
//! A check succeeds when the response status is 2xx/3xx and, if the monitor
//! has a look-for string, the body contains it. Response time is the full
//! duration from request start to body completion (to headers for HEAD).
//! Connection-level errors get one retry after a short delay; bad statuses
//! and missing strings do not.

use std::time::{Duration, Instant};

use crate::store::CheckMethod;

pub const USER_AGENT: &str = "tauri-uptime-monitor/1.0";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_RETRY_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub success: bool,
    pub http_status: Option<u16>,
    pub response_time_ms: i64,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct CheckConfig {
    pub timeout: Duration,
    pub retry_delay: Duration,
}

impl Default for CheckConfig {
    fn default() -> Self {
        CheckConfig {
            timeout: DEFAULT_TIMEOUT,
            retry_delay: DEFAULT_RETRY_DELAY,
        }
    }
}

/// Shared HTTP client + config, built once at startup and used by both the
/// scheduler and `check_now` so connection pooling is shared.
pub struct CheckContext {
    pub client: reqwest::Client,
    pub config: CheckConfig,
}

impl Default for CheckContext {
    fn default() -> Self {
        let config = CheckConfig::default();
        CheckContext {
            client: build_client(&config),
            config,
        }
    }
}

pub fn build_client(config: &CheckConfig) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(config.timeout)
        .build()
        .expect("reqwest client construction cannot fail with these options")
}

pub async fn run_check(
    client: &reqwest::Client,
    config: &CheckConfig,
    url: &str,
    method: CheckMethod,
    look_for_string: &str,
) -> CheckOutcome {
    // The timeout is a hard cap on the whole check — attempts plus the retry
    // delay — while each attempt is timed separately so a retried check
    // reports the final attempt's duration.
    let overall = Instant::now();
    let attempts = async {
        let mut started = Instant::now();
        let mut attempt = perform(client, url, method, look_for_string).await;
        if let Err(e) = &attempt {
            if e.retryable {
                tokio::time::sleep(config.retry_delay).await;
                started = Instant::now();
                attempt = perform(client, url, method, look_for_string).await;
            }
        }
        (started.elapsed(), attempt)
    };
    let (attempt_elapsed, attempt) = match tokio::time::timeout(config.timeout, attempts).await {
        Ok(result) => result,
        Err(_) => {
            return CheckOutcome {
                success: false,
                http_status: None,
                response_time_ms: overall.elapsed().as_millis() as i64,
                failure_reason: Some("timeout".into()),
            }
        }
    };
    let response_time_ms = attempt_elapsed.as_millis() as i64;
    match attempt {
        Ok(http_status) => CheckOutcome {
            success: true,
            http_status: Some(http_status),
            response_time_ms,
            failure_reason: None,
        },
        Err(failure) => CheckOutcome {
            success: false,
            http_status: failure.http_status,
            response_time_ms,
            failure_reason: Some(failure.reason),
        },
    }
}

struct CheckFailure {
    reason: String,
    http_status: Option<u16>,
    /// Only connection-level errors warrant the single retry.
    retryable: bool,
}

async fn perform(
    client: &reqwest::Client,
    url: &str,
    method: CheckMethod,
    look_for_string: &str,
) -> Result<u16, CheckFailure> {
    let request = match method {
        CheckMethod::GET => client.get(url),
        CheckMethod::HEAD => client.head(url),
        CheckMethod::POST => client.post(url),
    };
    let response = request.send().await.map_err(request_failure)?;
    let status = response.status();
    let code = status.as_u16();

    if !(status.is_success() || status.is_redirection()) {
        return Err(CheckFailure {
            reason: format!("HTTP {code}"),
            http_status: Some(code),
            retryable: false,
        });
    }

    // Read the body to completion so the measured time covers the full
    // response; HEAD has none.
    if method != CheckMethod::HEAD {
        let body = response.bytes().await.map_err(request_failure)?;
        if !look_for_string.is_empty() {
            let text = String::from_utf8_lossy(&body);
            if !text.contains(look_for_string) {
                return Err(CheckFailure {
                    reason: "string not found".into(),
                    http_status: Some(code),
                    retryable: false,
                });
            }
        }
    }

    Ok(code)
}

fn request_failure(e: reqwest::Error) -> CheckFailure {
    if e.is_timeout() {
        CheckFailure {
            reason: "timeout".into(),
            http_status: None,
            retryable: false,
        }
    } else if e.is_connect() {
        let detail = e
            .source_chain_root()
            .unwrap_or_else(|| "unreachable".into());
        CheckFailure {
            reason: format!("connection error: {detail}"),
            http_status: None,
            retryable: true,
        }
    } else {
        CheckFailure {
            reason: format!("request error: {e}"),
            http_status: e.status().map(|s| s.as_u16()),
            retryable: false,
        }
    }
}

trait SourceChainRoot {
    fn source_chain_root(&self) -> Option<String>;
}

impl SourceChainRoot for reqwest::Error {
    /// The innermost error message (e.g. "Connection refused") — the readable
    /// part of reqwest's nested error chain.
    fn source_chain_root(&self) -> Option<String> {
        let mut source: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(self);
        let mut last = None;
        while let Some(err) = source {
            last = Some(err.to_string());
            source = err.source();
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn test_config() -> CheckConfig {
        CheckConfig {
            timeout: Duration::from_millis(500),
            retry_delay: Duration::from_millis(10),
        }
    }

    async fn check(
        server: &MockServer,
        path: &str,
        method: CheckMethod,
        look_for: &str,
    ) -> CheckOutcome {
        let config = test_config();
        let client = build_client(&config);
        run_check(&client, &config, &server.url(path), method, look_for).await
    }

    #[tokio::test]
    async fn success_on_200() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/ok");
            then.status(200).body("hello world");
        });
        let outcome = check(&server, "/ok", CheckMethod::GET, "").await;
        assert!(outcome.success);
        assert_eq!(outcome.http_status, Some(200));
        assert!(outcome.failure_reason.is_none());
    }

    #[tokio::test]
    async fn failure_on_500_records_status() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/err");
            then.status(500);
        });
        let outcome = check(&server, "/err", CheckMethod::GET, "").await;
        assert!(!outcome.success);
        assert_eq!(outcome.http_status, Some(500));
        assert_eq!(outcome.failure_reason.as_deref(), Some("HTTP 500"));
    }

    #[tokio::test]
    async fn string_present_passes_and_absent_fails() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/page");
            then.status(200).body("all systems operational");
        });
        let ok = check(&server, "/page", CheckMethod::GET, "operational").await;
        assert!(ok.success);
        let missing = check(&server, "/page", CheckMethod::GET, "maintenance").await;
        assert!(!missing.success);
        assert_eq!(missing.failure_reason.as_deref(), Some("string not found"));
        assert_eq!(missing.http_status, Some(200));
    }

    #[tokio::test]
    async fn head_succeeds_without_reading_body() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::HEAD).path("/head");
            then.status(204);
        });
        let outcome = check(&server, "/head", CheckMethod::HEAD, "").await;
        assert!(outcome.success);
        assert_eq!(outcome.http_status, Some(204));
    }

    #[tokio::test]
    async fn timeout_is_reported_as_timeout() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/slow");
            then.status(200).delay(Duration::from_secs(2));
        });
        let outcome = check(&server, "/slow", CheckMethod::GET, "").await;
        assert!(!outcome.success);
        assert_eq!(outcome.failure_reason.as_deref(), Some("timeout"));
    }

    #[tokio::test]
    async fn connection_error_is_reported_and_retried() {
        // Nothing listens on this port.
        let config = test_config();
        let client = build_client(&config);
        let started = std::time::Instant::now();
        let outcome = run_check(
            &client,
            &config,
            "http://127.0.0.1:9",
            CheckMethod::GET,
            "",
        )
        .await;
        assert!(!outcome.success);
        assert!(
            outcome
                .failure_reason
                .as_deref()
                .unwrap()
                .starts_with("connection error:"),
            "reason: {:?}",
            outcome.failure_reason
        );
        // The retry delay proves a second attempt happened.
        assert!(started.elapsed() >= config.retry_delay);
    }
}
