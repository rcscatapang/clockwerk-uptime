//! TLS certificate inspection and daily scheduling.
//!
//! `native-tls` performs the authoritative handshake against the platform
//! trust store. If verification rejects the peer, a second inspection-only
//! handshake recovers leaf metadata without changing the invalid result. Both
//! handshakes run in `spawn_blocking` and share one timeout budget, keeping the
//! uptime scheduler's async workers free.

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use native_tls::{TlsConnector, TlsStream};
use x509_parser::parse_x509_certificate;

use crate::store::Monitor;

pub const CHECK_EVERY_HOURS: i64 = 24;
pub const EXPIRY_WARNING_DAYS: i64 = 10;
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct CertificateOutcome {
    pub valid: bool,
    pub expires_at: Option<String>,
    pub issuer: Option<String>,
    pub failure_reason: Option<String>,
}

impl CertificateOutcome {
    fn invalid(reason: impl Into<String>) -> Self {
        Self {
            valid: false,
            expires_at: None,
            issuer: None,
            failure_reason: Some(reason.into()),
        }
    }
}

#[derive(Debug)]
struct CertificateMetadata {
    expires_at: String,
    issuer: String,
}

pub fn is_due(monitor: &Monitor, now: DateTime<Utc>) -> bool {
    if !monitor.cert_check_enabled {
        return false;
    }
    let Some(last_check) = monitor
        .cert_last_check_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    else {
        return true;
    };
    now.signed_duration_since(last_check.with_timezone(&Utc))
        >= chrono::Duration::hours(CHECK_EVERY_HOURS)
}

pub async fn run_check(url: &str) -> CertificateOutcome {
    let started = Instant::now();
    let parsed = match url::Url::parse(url) {
        Ok(parsed) if parsed.scheme() == "https" => parsed,
        _ => return CertificateOutcome::invalid("certificate checks require an https URL"),
    };
    let Some(host) = parsed.host_str().map(str::to_string) else {
        return CertificateOutcome::invalid("certificate URL has no host");
    };
    let port = parsed.port().unwrap_or(443);
    let addresses = match tokio::time::timeout(
        CHECK_TIMEOUT,
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(addresses)) => addresses.collect::<Vec<_>>(),
        Ok(Err(error)) => {
            return CertificateOutcome::invalid(format!("could not resolve host: {error}"))
        }
        Err(_) => return CertificateOutcome::invalid("certificate check timed out"),
    };
    if addresses.is_empty() {
        return CertificateOutcome::invalid("host resolved to no addresses");
    }

    let remaining = CHECK_TIMEOUT.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return CertificateOutcome::invalid("certificate check timed out");
    }

    let task = tokio::task::spawn_blocking(move || {
        inspect_with_system_roots(&host, &addresses, remaining)
    });
    match tokio::time::timeout(remaining, task).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => CertificateOutcome::invalid(format!("certificate task failed: {error}")),
        Err(_) => CertificateOutcome::invalid("certificate check timed out"),
    }
}

fn inspect_with_system_roots(
    host: &str,
    addresses: &[SocketAddr],
    timeout: Duration,
) -> CertificateOutcome {
    let deadline = Instant::now() + timeout;
    let verified = TlsConnector::new()
        .map_err(|error| error.to_string())
        .and_then(|connector| connect_tls(&connector, host, addresses, deadline));
    match verified {
        Ok(stream) => match metadata(&stream) {
            Ok(metadata) => CertificateOutcome {
                valid: true,
                expires_at: Some(metadata.expires_at),
                issuer: Some(metadata.issuer),
                failure_reason: None,
            },
            Err(reason) => CertificateOutcome::invalid(reason),
        },
        Err(verification_reason) => {
            let mut builder = TlsConnector::builder();
            builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
            let recovered = builder
                .build()
                .map_err(|error| error.to_string())
                .and_then(|connector| connect_tls(&connector, host, addresses, deadline))
                .and_then(|stream| metadata(&stream));
            let mut outcome = CertificateOutcome::invalid(format!(
                "TLS certificate verification failed: {verification_reason}"
            ));
            if let Ok(metadata) = recovered {
                outcome.expires_at = Some(metadata.expires_at);
                outcome.issuer = Some(metadata.issuer);
            }
            outcome
        }
    }
}

fn connect_tls(
    connector: &TlsConnector,
    host: &str,
    addresses: &[SocketAddr],
    deadline: Instant,
) -> Result<TlsStream<TcpStream>, String> {
    let mut last_error = "connection failed".to_string();
    for address in addresses {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("certificate check timed out".into());
        }
        match TcpStream::connect_timeout(address, remaining) {
            Ok(stream) => {
                stream.set_read_timeout(Some(remaining)).ok();
                stream.set_write_timeout(Some(remaining)).ok();
                return connector
                    .connect(host, stream)
                    .map_err(|error| error.to_string());
            }
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(last_error)
}

fn metadata(stream: &TlsStream<TcpStream>) -> Result<CertificateMetadata, String> {
    let certificate = stream
        .peer_certificate()
        .map_err(|error| format!("could not read peer certificate: {error}"))?
        .ok_or_else(|| "server did not provide a certificate".to_string())?;
    let der = certificate
        .to_der()
        .map_err(|error| format!("could not decode peer certificate: {error}"))?;
    let (_, parsed) = parse_x509_certificate(&der)
        .map_err(|error| format!("could not parse peer certificate: {error}"))?;
    let expires_at = Utc
        .timestamp_opt(parsed.validity().not_after.timestamp(), 0)
        .single()
        .ok_or_else(|| "certificate expiry timestamp is invalid".to_string())?
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    Ok(CertificateMetadata {
        expires_at,
        issuer: parsed.issuer().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{CheckMethod, MonitorInput, Store};

    fn monitor() -> Monitor {
        Store::open_in_memory()
            .unwrap()
            .create_monitor(&MonitorInput {
                url: "https://example.com".into(),
                check_interval_minutes: 5,
                check_method: CheckMethod::GET,
                look_for_string: String::new(),
                uptime_check_enabled: true,
                cert_check_enabled: None,
            })
            .unwrap()
    }

    #[test]
    fn daily_due_rules() {
        let now = Utc::now();
        let mut value = monitor();
        assert!(is_due(&value, now));
        value.cert_last_check_at = Some((now - chrono::Duration::hours(23)).to_rfc3339());
        assert!(!is_due(&value, now));
        value.cert_last_check_at = Some((now - chrono::Duration::hours(24)).to_rfc3339());
        assert!(is_due(&value, now));
        value.cert_check_enabled = false;
        assert!(!is_due(&value, now));
    }

    #[tokio::test]
    #[ignore = "requires internet access"]
    async fn valid_certificate_fixture() {
        let outcome = run_check("https://sha256.badssl.com").await;
        assert!(outcome.valid, "{:?}", outcome.failure_reason);
        assert!(outcome.expires_at.is_some());
        assert!(outcome.issuer.is_some());
    }

    #[tokio::test]
    #[ignore = "requires internet access"]
    async fn invalid_certificate_fixtures_retain_metadata() {
        for url in [
            "https://expired.badssl.com",
            "https://self-signed.badssl.com",
        ] {
            let outcome = run_check(url).await;
            assert!(!outcome.valid, "{url}");
            assert!(outcome.failure_reason.is_some(), "{url}");
            assert!(outcome.expires_at.is_some(), "{url}");
            assert!(outcome.issuer.is_some(), "{url}");
        }
    }
}
