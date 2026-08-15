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

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Semaphore;

use crate::checker::{self, CheckConfig};
use crate::error::AppError;
use crate::store::{uptime_status, Monitor, RecordedCertificateCheck, RecordedCheck, Store};

pub const TICK_INTERVAL: Duration = Duration::from_secs(30);
pub const MAX_IN_FLIGHT: usize = 10;
const RETENTION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETENTION_BATCH_SIZE: usize = 5_000;

/// Event name the frontend listens for after each completed cycle.
pub const CHECK_COMPLETED_EVENT: &str = "check-completed";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckCompletedPayload {
    pub monitor_ids: Vec<i64>,
}

enum SchedulerOutcome {
    Checked(Vec<i64>),
    Complete,
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

/// Check every due monitor once.
pub async fn run_cycle(
    store: &Arc<Store>,
    client: &reqwest::Client,
    config: &CheckConfig,
) -> Result<Vec<RecordedCheck>, AppError> {
    let now = Utc::now();
    let due: Vec<Monitor> = store
        .list_monitors()?
        .into_iter()
        .filter(|m| is_due(m, now))
        .collect();
    Ok(check_concurrently(store, client, config, due).await)
}

/// Check every monitor once, `MAX_IN_FLIGHT` at a time. Shared by the
/// scheduler and by forced checks so both obey the same cap, timeout, and
/// recording path (and therefore the same state machine).
async fn check_concurrently(
    store: &Arc<Store>,
    client: &reqwest::Client,
    config: &CheckConfig,
    monitors: Vec<Monitor>,
) -> Vec<RecordedCheck> {
    if monitors.is_empty() {
        return Vec::new();
    }
    let semaphore = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let mut handles = Vec::with_capacity(monitors.len());
    for monitor in monitors {
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
            match store.record_check(monitor.id, &outcome).await {
                Ok(recorded) => Some(recorded),
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
            Ok(Some(recorded)) => checked.push(recorded),
            Ok(None) => {}
            Err(e) => tracing::error!(error = %e, "check task panicked"),
        }
    }
    checked
}

/// Check exactly these monitors immediately, whatever their schedule says. An
/// unknown id fails the whole call; a disabled monitor is checked when it was
/// asked for by id, and stays disabled — a one-off check is not an enable.
pub async fn check_many(
    store: &Arc<Store>,
    client: &reqwest::Client,
    config: &CheckConfig,
    ids: &[i64],
) -> Result<Vec<RecordedCheck>, AppError> {
    let monitors = store.get_monitors(ids)?;
    Ok(check_concurrently(store, client, config, monitors).await)
}

/// Check a single monitor immediately, outside the schedule.
pub async fn check_one(
    store: &Arc<Store>,
    client: &reqwest::Client,
    config: &CheckConfig,
    id: i64,
) -> Result<RecordedCheck, AppError> {
    let monitor = store.get_monitor(id)?;
    let outcome = checker::run_check(
        client,
        config,
        &monitor.url,
        monitor.check_method,
        &monitor.look_for_string,
    )
    .await;
    store.record_check(id, &outcome).await
}

pub async fn check_certificate_one(
    store: &Arc<Store>,
    id: i64,
) -> Result<Option<RecordedCertificateCheck>, AppError> {
    let monitor = store.get_monitor(id)?;
    if !monitor.cert_check_enabled {
        return Ok(None);
    }
    let outcome = crate::certificate::run_check(&monitor.url).await;
    store.record_certificate_check(id, &outcome)
}

pub async fn run_certificate_cycle(
    store: &Arc<Store>,
) -> Result<Vec<RecordedCertificateCheck>, AppError> {
    let now = Utc::now();
    let due: Vec<Monitor> = store
        .list_monitors()?
        .into_iter()
        .filter(|monitor| crate::certificate::is_due(monitor, now))
        .collect();
    let mut checked = Vec::with_capacity(due.len());
    for monitor in due {
        let outcome = crate::certificate::run_check(&monitor.url).await;
        match store.record_certificate_check(monitor.id, &outcome) {
            Ok(Some(recorded)) => checked.push(recorded),
            Ok(None) => {}
            Err(error) => {
                tracing::error!(monitor_id = monitor.id, error = %error, "failed to record certificate check")
            }
        }
    }
    Ok(checked)
}

async fn run_certificate_tick(app: &AppHandle, store: &Arc<Store>) -> Result<Vec<i64>, AppError> {
    let checked = run_certificate_cycle(store).await?;
    for recorded in &checked {
        crate::alerter::handle_certificate_check(app, store, recorded).await;
    }
    Ok(checked.iter().map(|recorded| recorded.after.id).collect())
}

async fn run_uptime_tick(
    app: &AppHandle,
    store: &Arc<Store>,
    client: &reqwest::Client,
    config: &CheckConfig,
) -> Result<Vec<i64>, AppError> {
    let checked = run_cycle(store, client, config).await?;
    for recorded in &checked {
        crate::alerter::handle_check(app, store, recorded).await;
    }
    crate::alerter::process_still_down(app, store).await;
    Ok(checked.iter().map(|recorded| recorded.after.id).collect())
}

pub async fn run_retention_cycle_at(
    store: &Arc<Store>,
    now: DateTime<Utc>,
) -> Result<usize, AppError> {
    let cutoff = now - chrono::Duration::days(crate::store::HISTORY_RETENTION_DAYS);
    let mut deleted = 0;
    loop {
        let batch = store.prune_history_batch_before(cutoff, RETENTION_BATCH_SIZE)?;
        deleted += batch;
        if batch < RETENTION_BATCH_SIZE {
            break;
        }
        tokio::task::yield_now().await;
    }
    store.set_last_prune_at(&crate::history::format_timestamp(now))?;
    tracing::info!(deleted, "history retention prune complete");
    Ok(deleted)
}

async fn scheduler_loop<F, Fut>(
    app: AppHandle,
    scheduler: &'static str,
    cadence: Duration,
    mut cycle: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<SchedulerOutcome, AppError>>,
{
    let mut interval = tokio::time::interval(cadence);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match cycle().await {
            Ok(SchedulerOutcome::Checked(monitor_ids)) if !monitor_ids.is_empty() => {
                if let Err(error) =
                    app.emit(CHECK_COMPLETED_EVENT, CheckCompletedPayload { monitor_ids })
                {
                    tracing::error!(scheduler, error = %error, "failed to emit check completion");
                }
            }
            Ok(SchedulerOutcome::Checked(_) | SchedulerOutcome::Complete) => {}
            Err(error) => tracing::error!(scheduler, error = %error, "check cycle failed"),
        }
    }
}

/// Spawn the scheduler loops. They run for the lifetime of the app,
/// independent of window visibility.
pub fn start(app: AppHandle) {
    let store = app.state::<Arc<Store>>().inner().clone();
    let ctx = app.state::<checker::CheckContext>();
    let client = ctx.client.clone();
    let config = ctx.config;

    let certificate_scheduler_app = app.clone();
    let certificate_cycle_app = app.clone();
    let certificate_store = store.clone();
    tauri::async_runtime::spawn(scheduler_loop(
        certificate_scheduler_app,
        "certificate",
        TICK_INTERVAL,
        move || {
            let app = certificate_cycle_app.clone();
            let store = certificate_store.clone();
            async move {
                run_certificate_tick(&app, &store)
                    .await
                    .map(SchedulerOutcome::Checked)
            }
        },
    ));

    let uptime_scheduler_app = app.clone();
    let uptime_cycle_app = app.clone();
    let uptime_store = store.clone();
    tauri::async_runtime::spawn(scheduler_loop(
        uptime_scheduler_app,
        "uptime",
        TICK_INTERVAL,
        move || {
            let app = uptime_cycle_app.clone();
            let store = uptime_store.clone();
            let client = client.clone();
            async move {
                run_uptime_tick(&app, &store, &client, &config)
                    .await
                    .map(SchedulerOutcome::Checked)
            }
        },
    ));

    let retention_store = store;
    tauri::async_runtime::spawn(scheduler_loop(
        app,
        "retention",
        RETENTION_INTERVAL,
        move || {
            let store = retention_store.clone();
            async move {
                run_retention_cycle_at(&store, Utc::now()).await?;
                Ok(SchedulerOutcome::Complete)
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{CheckMethod, MonitorInput};
    use chrono::TimeZone;
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
            cert_status: crate::store::CertificateStatus::NotYetChecked,
            cert_expires_at: None,
            cert_issuer: None,
            cert_failure_reason: None,
            cert_last_check_at: None,
            cert_expiry_alert_sent_at: None,
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
    async fn retention_cycle_records_its_completion() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let now = Utc.with_ymd_and_hms(2026, 8, 12, 12, 0, 0).unwrap();

        assert_eq!(run_retention_cycle_at(&store, now).await.unwrap(), 0);
        assert_eq!(
            store.get_settings().unwrap().last_prune_at.as_deref(),
            Some(crate::history::format_timestamp(now).as_str())
        );
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
        assert_eq!(
            checked.iter().map(|r| r.after.id).collect::<Vec<_>>(),
            vec![bad.id]
        );
        let bad_after = store.get_monitor(bad.id).unwrap();
        assert_eq!(bad_after.uptime_status, "down");
        assert_eq!(bad_after.uptime_failure_reason.as_deref(), Some("HTTP 500"));
    }

    #[tokio::test]
    async fn forced_checks_run_disabled_monitors_and_leave_them_disabled() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/ok");
            then.status(200);
        });

        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut disabled = input(&server.url("/ok"));
        disabled.uptime_check_enabled = false;
        let monitor = store.create_monitor(&disabled).unwrap();

        let config = CheckConfig {
            timeout: Duration::from_millis(500),
            retry_delay: Duration::from_millis(10),
        };
        let client = checker::build_client(&config);

        // The scheduler skips it, an explicit selection does not.
        assert!(run_cycle(&store, &client, &config).await.unwrap().is_empty());
        let checked = check_many(&store, &client, &config, &[monitor.id])
            .await
            .unwrap();
        assert_eq!(checked.len(), 1);

        let after = store.get_monitor(monitor.id).unwrap();
        assert_eq!(after.uptime_status, "up");
        assert!(!after.uptime_check_enabled, "a one-off check must not enable");

        // An unknown id fails the whole call.
        assert!(matches!(
            check_many(&store, &client, &config, &[monitor.id + 99]).await,
            Err(AppError::NotFound)
        ));
    }

    #[tokio::test]
    async fn a_forced_check_that_fails_drives_the_normal_transition_and_alert() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/bad");
            then.status(500);
        });

        let store = Arc::new(Store::open_in_memory().unwrap());
        let monitor = store.create_monitor(&input(&server.url("/bad"))).unwrap();
        let config = CheckConfig {
            timeout: Duration::from_millis(500),
            retry_delay: Duration::from_millis(10),
        };
        let client = checker::build_client(&config);

        // One failure is suspicion; the alerter stays quiet.
        let first = check_many(&store, &client, &config, &[monitor.id])
            .await
            .unwrap();
        assert!(crate::alerter::decide_after_check(&first[0], Utc::now()).is_none());

        // The second crosses the threshold: same state machine, same alert.
        let second = check_many(&store, &client, &config, &[monitor.id])
            .await
            .unwrap();
        assert_eq!(store.get_monitor(monitor.id).unwrap().uptime_status, "down");
        let alert = crate::alerter::decide_after_check(&second[0], Utc::now())
            .expect("a forced check crossing the threshold must alert");
        assert_eq!(alert.monitor_id, monitor.id);
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
