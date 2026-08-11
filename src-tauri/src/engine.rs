//! Background check scheduler.
//!
//! A tokio task ticks every 30 seconds; each tick collects the monitors that
//! are due and checks them concurrently (capped in-flight, so one slow site
//! never delays the rest). A monitor is due when uptime checking is enabled
//! and any of these hold:
//!
//! - it is still `not_yet_checked` (including after failures that haven't
//!   crossed the down threshold yet) or has no recorded check time — new
//!   monitors resolve to a visible status quickly,
//! - it is currently `down` — failing monitors are re-checked every tick,
//!   interval ignored, so recovery is detected quickly,
//! - its check interval has elapsed since the last check.
//!
//! All writes funnel through `Store::record_check` (one transaction per
//! check, one connection behind a mutex), so the engine and the command
//! handlers share a single writer story. A failing or panicking check is
//! logged and never kills the scheduler.
//!
//! After each cycle that touched at least one monitor, a `check-completed`
//! event carries the affected monitor ids to the frontend, which invalidates
//! its queries — the UI never polls.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use crate::checker::{self, CheckConfig};
use crate::error::AppError;
use crate::store::{uptime_status, Monitor, Store};

pub const TICK_INTERVAL: Duration = Duration::from_secs(30);
pub const MAX_IN_FLIGHT: usize = 10;

/// Event name the frontend listens for after each completed cycle.
pub const CHECK_COMPLETED_EVENT: &str = "check-completed";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckCompletedPayload {
    pub monitor_ids: Vec<i64>,
}

pub fn is_due(monitor: &Monitor, now: DateTime<Utc>) -> bool {
    if !monitor.uptime_check_enabled {
        return false;
    }
    if monitor.uptime_status == uptime_status::NOT_YET_CHECKED
        || monitor.uptime_status == uptime_status::DOWN
    {
        return true;
    }
    match &monitor.last_check_at {
        None => true,
        Some(last) => match DateTime::parse_from_rfc3339(last) {
            // An unparseable timestamp should never hide a monitor forever.
            Err(_) => true,
            Ok(last) => {
                let elapsed = now.signed_duration_since(last.with_timezone(&Utc));
                elapsed.num_minutes() >= monitor.check_interval_minutes
            }
        },
    }
}

/// Check every due monitor once. Returns the ids that were checked.
pub async fn run_cycle(
    store: &Arc<Store>,
    client: &reqwest::Client,
    config: &CheckConfig,
) -> Result<Vec<i64>, AppError> {
    let now = Utc::now();
    let due: Vec<Monitor> = store
        .list_monitors()?
        .into_iter()
        .filter(|m| is_due(m, now))
        .collect();
    if due.is_empty() {
        return Ok(Vec::new());
    }

    let semaphore = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let mut handles = Vec::with_capacity(due.len());
    for monitor in due {
        let semaphore = semaphore.clone();
        let store = store.clone();
        let client = client.clone();
        let config = *config;
        handles.push(tokio::spawn(async move {
            let _permit = semaphore.acquire_owned().await.ok()?;
            let outcome = checker::run_check(
                &client,
                &config,
                &monitor.url,
                monitor.check_method,
                &monitor.look_for_string,
            )
            .await;
            match store.record_check(monitor.id, &outcome) {
                Ok((_, _event)) => Some(monitor.id),
                Err(e) => {
                    tracing::error!(monitor_id = monitor.id, error = %e, "failed to record check");
                    None
                }
            }
        }));
    }

    let mut checked = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(Some(id)) => checked.push(id),
            Ok(None) => {}
            Err(e) => tracing::error!(error = %e, "check task panicked"),
        }
    }
    Ok(checked)
}

/// Check a single monitor immediately, outside the schedule.
pub async fn check_one(
    store: &Arc<Store>,
    client: &reqwest::Client,
    config: &CheckConfig,
    id: i64,
) -> Result<Monitor, AppError> {
    let monitor = store.get_monitor(id)?;
    let outcome = checker::run_check(
        client,
        config,
        &monitor.url,
        monitor.check_method,
        &monitor.look_for_string,
    )
    .await;
    let (updated, _event) = store.record_check(id, &outcome)?;
    Ok(updated)
}

/// Spawn the scheduler loop. Runs for the lifetime of the app, independent
/// of window visibility.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let store = app.state::<Arc<Store>>().inner().clone();
        let ctx = app.state::<checker::CheckContext>();
        let config = ctx.config;
        let client = ctx.client.clone();
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            match run_cycle(&store, &client, &config).await {
                Ok(ids) if !ids.is_empty() => {
                    if let Err(e) = app.emit(
                        CHECK_COMPLETED_EVENT,
                        CheckCompletedPayload { monitor_ids: ids },
                    ) {
                        tracing::error!(error = %e, "failed to emit check-completed");
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "check cycle failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{CheckMethod, MonitorInput};
    use httpmock::prelude::*;
    use std::time::Instant;

    fn input(url: &str) -> MonitorInput {
        MonitorInput {
            url: url.into(),
            check_interval_minutes: 5,
            check_method: CheckMethod::GET,
            look_for_string: String::new(),
            uptime_check_enabled: true,
            cert_check_enabled: None,
        }
    }

    fn monitor_due_probe(overrides: impl FnOnce(&mut Monitor)) -> Monitor {
        let mut m = Monitor {
            id: 1,
            url: "https://example.com".into(),
            uptime_check_enabled: true,
            check_interval_minutes: 5,
            check_method: CheckMethod::GET,
            look_for_string: String::new(),
            uptime_status: "up".into(),
            uptime_failure_reason: None,
            consecutive_failures: 0,
            status_last_change_at: None,
            last_check_at: None,
            down_alert_sent_at: None,
            cert_check_enabled: false,
            cert_status: "not_yet_checked".into(),
            cert_expires_at: None,
            cert_issuer: None,
            cert_failure_reason: None,
            created_at: String::new(),
            updated_at: String::new(),
            last_response_time_ms: None,
        };
        overrides(&mut m);
        m
    }

    #[test]
    fn due_rules() {
        let now = Utc::now();
        let recent = (now - chrono::Duration::minutes(2)).to_rfc3339();
        let stale = (now - chrono::Duration::minutes(6)).to_rfc3339();

        // Disabled: never due.
        assert!(!is_due(
            &monitor_due_probe(|m| m.uptime_check_enabled = false),
            now
        ));
        // Never checked / no timestamp: due.
        assert!(is_due(
            &monitor_due_probe(|m| m.uptime_status = "not_yet_checked".into()),
            now
        ));
        assert!(is_due(&monitor_due_probe(|m| m.last_check_at = None), now));
        // Down: due every tick even if just checked.
        assert!(is_due(
            &monitor_due_probe(|m| {
                m.uptime_status = "down".into();
                m.last_check_at = Some(recent.clone());
            }),
            now
        ));
        // Up + recent: not due. Up + interval elapsed: due.
        assert!(!is_due(
            &monitor_due_probe(|m| m.last_check_at = Some(recent.clone())),
            now
        ));
        assert!(is_due(
            &monitor_due_probe(|m| m.last_check_at = Some(stale.clone())),
            now
        ));
    }

    #[tokio::test]
    async fn cycle_checks_due_monitors_and_records_results() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/ok");
            then.status(200);
        });
        server.mock(|when, then| {
            when.method(GET).path("/bad");
            then.status(500);
        });

        let store = Arc::new(Store::open_in_memory().unwrap());
        let ok = store.create_monitor(&input(&server.url("/ok"))).unwrap();
        let bad = store.create_monitor(&input(&server.url("/bad"))).unwrap();

        let config = CheckConfig {
            timeout: Duration::from_millis(500),
            retry_delay: Duration::from_millis(10),
        };
        let client = checker::build_client(&config);
        let checked = run_cycle(&store, &client, &config).await.unwrap();
        assert_eq!(checked.len(), 2);

        let ok_after = store.get_monitor(ok.id).unwrap();
        assert_eq!(ok_after.uptime_status, "up");
        assert!(ok_after.last_check_at.is_some());
        assert!(ok_after.last_response_time_ms.is_some());

        let bad_after = store.get_monitor(bad.id).unwrap();
        // One failure: suspicion, not state.
        assert_eq!(bad_after.uptime_status, "not_yet_checked");
        assert_eq!(bad_after.consecutive_failures, 1);

        // Second cycle: both are due again (bad is not_yet_checked, ok not
        // due yet) — only bad runs, and it goes down at two failures.
        let checked = run_cycle(&store, &client, &config).await.unwrap();
        assert_eq!(checked, vec![bad.id]);
        let bad_after = store.get_monitor(bad.id).unwrap();
        assert_eq!(bad_after.uptime_status, "down");
        assert_eq!(bad_after.uptime_failure_reason.as_deref(), Some("HTTP 500"));
    }

    #[tokio::test]
    async fn slow_monitor_does_not_serialize_the_cycle() {
        let server = MockServer::start();
        let delay = Duration::from_millis(300);
        for i in 0..4 {
            let path = format!("/slow{i}");
            server.mock(move |when, then| {
                when.method(GET).path(path.clone());
                then.status(200).delay(delay);
            });
        }

        let store = Arc::new(Store::open_in_memory().unwrap());
        for i in 0..4 {
            store
                .create_monitor(&input(&server.url(format!("/slow{i}"))))
                .unwrap();
        }

        let config = CheckConfig {
            timeout: Duration::from_secs(2),
            retry_delay: Duration::from_millis(10),
        };
        let client = checker::build_client(&config);
        let started = Instant::now();
        let checked = run_cycle(&store, &client, &config).await.unwrap();
        let elapsed = started.elapsed();
        assert_eq!(checked.len(), 4);
        // Concurrent: well under the 4 × 300 ms a serial run would need.
        assert!(
            elapsed < Duration::from_millis(900),
            "cycle took {elapsed:?}"
        );
    }
}
