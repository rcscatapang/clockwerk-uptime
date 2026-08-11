//! Alert decisions and delivery.
//!
//! Decision rules:
//! - **Down** fires exactly when the status flips to `down` (two consecutive
//!   failures) and stamps `down_alert_sent_at` after a channel accepts it.
//! - **Still down** re-fires while the status stays `down` and 60 minutes
//!   have passed since the last alert; each one re-stamps
//!   `down_alert_sent_at`. A failed or interrupted delivery has no stamp and
//!   is retried on the next scheduler tick.
//! - **Recovered** fires on success only if a down alert actually went out
//!   (`down_alert_sent_at` set); it clears the stamp. A blip that never
//!   crossed the threshold stays silent.
//! - An alert carries the status it was fired for; if the monitor's status
//!   has changed again by dispatch time, the alert is dropped (stale guard).
//!
//! Every alert goes to a native notification and, when a webhook is
//! configured, to Slack. Delivery failures are logged and never affect a
//! check cycle.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::secrets;
use crate::slack;
use crate::state::TransitionEvent;
use crate::store::{uptime_status, Monitor, RecordedCheck, Store};

pub const REALERT_AFTER_MINUTES: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub monitor_id: i64,
    pub title: String,
    pub body: String,
    /// Status the alert was fired for; dispatch drops it if the monitor has
    /// since moved on.
    pub fired_for_status: &'static str,
    /// Identifies the specific up/down episode, so an old down alert cannot
    /// be delivered during a later down incident.
    pub fired_for_status_changed_at: Option<String>,
    /// New `down_alert_sent_at` value to store when this alert dispatches.
    pub sent_at_update: Option<String>,
}

/// Decide the alert (if any) for a just-recorded check.
pub fn decide_after_check(recorded: &RecordedCheck, now: DateTime<Utc>) -> Option<Alert> {
    match recorded.event? {
        TransitionEvent::WentDown => {
            let reason = recorded
                .after
                .uptime_failure_reason
                .as_deref()
                .unwrap_or("check failed");
            Some(Alert {
                monitor_id: recorded.after.id,
                title: "Monitor down".into(),
                body: format!("{} is down: {reason}", recorded.after.url),
                fired_for_status: uptime_status::DOWN,
                fired_for_status_changed_at: recorded.after.status_last_change_at.clone(),
                sent_at_update: Some(now.to_rfc3339()),
            })
        }
        TransitionEvent::Recovered => {
            // Silent recovery unless a down alert actually went out.
            recorded.before.down_alert_sent_at.as_ref()?;
            let downtime =
                human_duration_between(recorded.before.status_last_change_at.as_deref(), now);
            Some(Alert {
                monitor_id: recorded.after.id,
                title: "Monitor recovered".into(),
                body: format!("{} is back up after {downtime}", recorded.after.url),
                fired_for_status: uptime_status::UP,
                fired_for_status_changed_at: recorded.after.status_last_change_at.clone(),
                sent_at_update: None,
            })
        }
    }
}

/// Decide whether a down monitor is due for its hourly re-alert.
pub fn decide_still_down(monitor: &Monitor, now: DateTime<Utc>) -> Option<Alert> {
    if monitor.uptime_status != uptime_status::DOWN {
        return None;
    }
    if let Some(sent_at) = monitor
        .down_alert_sent_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    {
        if now
            .signed_duration_since(sent_at.with_timezone(&Utc))
            .num_minutes()
            < REALERT_AFTER_MINUTES
        {
            return None;
        }
    }
    let downtime = human_duration_between(monitor.status_last_change_at.as_deref(), now);
    Some(Alert {
        monitor_id: monitor.id,
        title: "Monitor still down".into(),
        body: format!("{} has been down for {downtime}", monitor.url),
        fired_for_status: uptime_status::DOWN,
        fired_for_status_changed_at: monitor.status_last_change_at.clone(),
        sent_at_update: Some(now.to_rfc3339()),
    })
}

/// Stale guard: the alert only goes out if the monitor is still in the
/// status it was fired for.
pub fn is_stale(
    alert: &Alert,
    current_status: &str,
    current_status_changed_at: Option<&str>,
) -> bool {
    current_status != alert.fired_for_status
        || current_status_changed_at != alert.fired_for_status_changed_at.as_deref()
}

pub fn slack_text(alert: &Alert, now: DateTime<Utc>) -> String {
    let emoji = if alert.fired_for_status == uptime_status::UP {
        "✅"
    } else {
        "🔴"
    };
    format!(
        "{emoji} {} ({})",
        alert.body,
        now.format("%Y-%m-%d %H:%M UTC")
    )
}

pub fn human_duration_between(start: Option<&str>, end: DateTime<Utc>) -> String {
    let Some(start) = start.and_then(|s| DateTime::parse_from_rfc3339(s).ok()) else {
        return "an unknown time".into();
    };
    let minutes = end
        .signed_duration_since(start.with_timezone(&Utc))
        .num_minutes()
        .max(0);
    match minutes {
        0 => "less than a minute".into(),
        1..=59 => format!("{minutes} min"),
        60..=1439 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, (minutes % 1440) / 60),
    }
}

fn alert_is_current(store: &Store, alert: &Alert) -> bool {
    match store.get_monitor(alert.monitor_id) {
        Ok(current) => {
            if is_stale(
                alert,
                &current.uptime_status,
                current.status_last_change_at.as_deref(),
            ) {
                tracing::info!(
                    monitor_id = alert.monitor_id,
                    "dropping stale alert: status moved on"
                );
                return false;
            }
            true
        }
        Err(_) => false, // monitor deleted since the check
    }
}

fn record_delivery(store: &Store, alert: &Alert) -> bool {
    match store.set_down_alert_sent_at_if_status(
        alert.monitor_id,
        alert.fired_for_status,
        alert.fired_for_status_changed_at.as_deref(),
        alert.sent_at_update.as_deref(),
    ) {
        Ok(recorded) => recorded,
        Err(e) => {
            tracing::error!(monitor_id = alert.monitor_id, error = %e, "alert bookkeeping failed");
            false
        }
    }
}

/// Deliver an alert to each configured channel. The monitor status is checked
/// at each delivery boundary, and bookkeeping is written only after a channel
/// accepts the alert. Channel failures are logged, never propagated.
pub async fn dispatch(app: &AppHandle, store: &Arc<Store>, alert: Alert) {
    let _alerting = store.lock_alerting().await;
    let mut delivery_recorded = false;

    if alert_is_current(store, &alert) {
        match app
            .notification()
            .builder()
            .title(&alert.title)
            .body(&alert.body)
            .show()
        {
            Ok(()) => delivery_recorded = record_delivery(store, &alert),
            Err(e) => tracing::warn!(error = %e, "native notification failed"),
        }
    }

    match secrets::get_slack_webhook() {
        Ok(Some(webhook)) if alert_is_current(store, &alert) => {
            match slack::send(&webhook, &slack_text(&alert, Utc::now())).await {
                Ok(()) if !delivery_recorded => {
                    record_delivery(store, &alert);
                }
                Ok(()) => {}
                Err(e) => tracing::warn!(error = %e, "slack alert failed"),
            }
        }
        Ok(Some(_)) => {}
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "keychain read failed while alerting"),
    }
}

/// Handle the alert consequences of one recorded check.
pub async fn handle_check(app: &AppHandle, store: &Arc<Store>, recorded: &RecordedCheck) {
    if let Some(alert) = decide_after_check(recorded, Utc::now()) {
        dispatch(app, store, alert).await;
    }
}

/// Hourly still-down re-alerts, evaluated on the scheduler tick.
pub async fn process_still_down(app: &AppHandle, store: &Arc<Store>) {
    let monitors = match store.list_monitors() {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(error = %e, "still-down scan failed");
            return;
        }
    };
    let now = Utc::now();
    for monitor in monitors {
        if let Some(alert) = decide_still_down(&monitor, now) {
            dispatch(app, store, alert).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::CheckMethod;

    fn monitor(status: &str) -> Monitor {
        Monitor {
            id: 7,
            url: "https://example.com".into(),
            uptime_check_enabled: true,
            check_interval_minutes: 5,
            check_method: CheckMethod::GET,
            look_for_string: String::new(),
            uptime_status: status.into(),
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
        }
    }

    fn recorded(before: Monitor, after: Monitor, event: Option<TransitionEvent>) -> RecordedCheck {
        RecordedCheck {
            before,
            after,
            event,
        }
    }

    #[test]
    fn went_down_fires_and_stamps() {
        let mut after = monitor("down");
        after.uptime_failure_reason = Some("HTTP 500".into());
        let alert = decide_after_check(
            &recorded(monitor("up"), after, Some(TransitionEvent::WentDown)),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(alert.fired_for_status, "down");
        assert!(alert.sent_at_update.is_some());
        assert!(alert.body.contains("HTTP 500"));
    }

    #[test]
    fn no_event_no_alert() {
        assert!(
            decide_after_check(&recorded(monitor("up"), monitor("up"), None), Utc::now()).is_none()
        );
    }

    #[test]
    fn recovery_is_silent_without_prior_down_alert() {
        let before = monitor("down");
        let alert = decide_after_check(
            &recorded(before, monitor("up"), Some(TransitionEvent::Recovered)),
            Utc::now(),
        );
        assert!(alert.is_none());
    }

    #[test]
    fn recovery_fires_and_clears_after_down_alert() {
        let now = Utc::now();
        let mut before = monitor("down");
        before.down_alert_sent_at = Some(now.to_rfc3339());
        before.status_last_change_at = Some((now - chrono::Duration::minutes(90)).to_rfc3339());
        let alert = decide_after_check(
            &recorded(before, monitor("up"), Some(TransitionEvent::Recovered)),
            now,
        )
        .unwrap();
        assert_eq!(alert.fired_for_status, "up");
        assert!(alert.sent_at_update.is_none());
        assert!(alert.body.contains("1h 30m"), "body: {}", alert.body);
    }

    #[test]
    fn still_down_realerts_only_after_the_window() {
        let now = Utc::now();
        let mut m = monitor("down");
        m.status_last_change_at = Some((now - chrono::Duration::minutes(120)).to_rfc3339());

        m.down_alert_sent_at = Some((now - chrono::Duration::minutes(30)).to_rfc3339());
        assert!(decide_still_down(&m, now).is_none());

        m.down_alert_sent_at = Some((now - chrono::Duration::minutes(61)).to_rfc3339());
        let alert = decide_still_down(&m, now).unwrap();
        assert!(alert.body.contains("2h 0m"), "body: {}", alert.body);
        assert!(alert.sent_at_update.is_some());
    }

    #[test]
    fn failed_or_missing_down_alert_is_retried() {
        let now = Utc::now();
        assert!(decide_still_down(&monitor("up"), now).is_none());
        assert!(decide_still_down(&monitor("down"), now).is_some());
    }

    #[test]
    fn stale_guard() {
        let alert = Alert {
            monitor_id: 1,
            title: String::new(),
            body: String::new(),
            fired_for_status: "down",
            fired_for_status_changed_at: Some("episode-1".into()),
            sent_at_update: None,
        };
        assert!(is_stale(&alert, "up", Some("episode-1")));
        assert!(is_stale(&alert, "down", Some("episode-2")));
        assert!(!is_stale(&alert, "down", Some("episode-1")));
    }

    #[test]
    fn slack_text_uses_status_emoji() {
        let now = Utc::now();
        let down = Alert {
            monitor_id: 1,
            title: String::new(),
            body: "x is down".into(),
            fired_for_status: "down",
            fired_for_status_changed_at: None,
            sent_at_update: None,
        };
        assert!(slack_text(&down, now).starts_with("🔴"));
        let up = Alert {
            fired_for_status: "up",
            ..down
        };
        assert!(slack_text(&up, now).starts_with("✅"));
    }

    #[test]
    fn human_durations() {
        let now = Utc::now();
        let ago = |m: i64| Some((now - chrono::Duration::minutes(m)).to_rfc3339());
        assert_eq!(
            human_duration_between(ago(0).as_deref(), now),
            "less than a minute"
        );
        assert_eq!(human_duration_between(ago(5).as_deref(), now), "5 min");
        assert_eq!(human_duration_between(ago(75).as_deref(), now), "1h 15m");
        assert_eq!(human_duration_between(ago(3000).as_deref(), now), "2d 2h");
        assert_eq!(human_duration_between(None, now), "an unknown time");
    }
}
