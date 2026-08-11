//! Slack incoming-webhook delivery.
//!
//! Fire-and-forget: a delivery failure is logged by the caller and never
//! affects a check cycle. There is no retry queue — the hourly still-down
//! re-alert is the natural retry.

use std::time::Duration;

use crate::error::AppError;

pub const SLACK_TIMEOUT: Duration = Duration::from_secs(5);
pub const WEBHOOK_HOST: &str = "hooks.slack.com";
pub const CONFIGURATION_TEST_MESSAGE: &str = "webhook configured";

/// A webhook URL must be https on hooks.slack.com; anything else is rejected
/// at entry time.
pub fn validate_webhook_url(url: &str) -> Result<(), AppError> {
    let parsed = url::Url::parse(url)
        .map_err(|e| AppError::InvalidUrl(format!("invalid webhook URL: {e}")))?;
    if parsed.scheme() != "https" || parsed.host_str() != Some(WEBHOOK_HOST) {
        return Err(AppError::InvalidUrl(format!(
            "webhook must be an https URL on {WEBHOOK_HOST}"
        )));
    }
    Ok(())
}

pub fn payload(text: &str) -> serde_json::Value {
    serde_json::json!({ "text": text })
}

pub async fn send(webhook_url: &str, text: &str) -> Result<(), AppError> {
    let client = reqwest::Client::builder()
        .timeout(SLACK_TIMEOUT)
        .build()
        .map_err(|e| AppError::Internal(format!("slack client: {e}")))?;
    let response = client
        .post(webhook_url)
        .json(&payload(text))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("slack delivery failed: {}", e.without_url())))?;
    if !response.status().is_success() {
        return Err(AppError::Internal(format!(
            "slack delivery failed: HTTP {}",
            response.status().as_u16()
        )));
    }
    Ok(())
}

pub async fn verify(webhook_url: &str) -> Result<(), AppError> {
    send(webhook_url, CONFIGURATION_TEST_MESSAGE).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn webhook_validation() {
        assert!(validate_webhook_url("https://hooks.slack.com/services/T0/B0/x").is_ok());
        for bad in [
            "http://hooks.slack.com/services/T0/B0/x",
            "https://example.com/services/T0/B0/x",
            "https://hooks.slack.com.evil.example/x",
            "not a url",
        ] {
            assert!(validate_webhook_url(bad).is_err(), "accepted: {bad}");
        }
    }

    #[test]
    fn payload_shape() {
        assert_eq!(
            payload("monitor down").to_string(),
            r#"{"text":"monitor down"}"#
        );
    }

    #[tokio::test]
    async fn send_posts_json_payload() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/hook")
                .header("content-type", "application/json")
                .json_body(serde_json::json!({ "text": "🔴 test" }));
            then.status(200);
        });
        send(&server.url("/hook"), "🔴 test").await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn send_reports_http_failure() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/hook");
            then.status(404);
        });
        assert!(send(&server.url("/hook"), "x").await.is_err());
    }

    #[tokio::test]
    async fn transport_errors_do_not_expose_the_webhook_url() {
        let error = send("http://127.0.0.1:1/services/secret-token", "x")
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("secret-token"));
    }

    #[tokio::test]
    async fn verify_sends_configuration_message() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/hook")
                .json_body(serde_json::json!({
                    "text": CONFIGURATION_TEST_MESSAGE
                }));
            then.status(200);
        });
        verify(&server.url("/hook")).await.unwrap();
        mock.assert();
    }
}
