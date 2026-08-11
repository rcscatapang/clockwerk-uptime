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
use crate::store::{
    uptime_status, CertificateEvent, CertificateStatus, Monitor, RecordedCertificateCheck,
    RecordedCheck, Store,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateAlert {
    pub monitor_id: i64,
    pub title: String,
    pub body: String,
    pub fired_for_status: CertificateStatus,
    pub fired_for_check_at: String,
    pub expiry_sent_at_update: Option<String>,
}

trait DeliverableAlert {
    fn monitor_id(&self) -> i64;
    fn content(&self) -> (&str, &str);
    fn is_stale(&self, current: &Monitor) -> bool;
    fn record_delivery(&self, store: &Store) -> Result<bool, crate::error::AppError>;
    fn slack_text(&self, now: DateTime<Utc>) -> String;
}

impl DeliverableAlert for Alert {
    fn monitor_id(&self) -> i64 {
        self.monitor_id
    }

    fn content(&self) -> (&str, &str) {
        (&self.title, &self.body)
    }

    fn is_stale(&self, current: &Monitor) -> bool {
        is_stale(
            self,
            &current.uptime_status,
            current.status_last_change_at.as_deref(),
        )
    }

    fn record_delivery(&self, store: &Store) -> Result<bool, crate::error::AppError> {
        store.set_down_alert_sent_at_if_status(
            self.monitor_id,
            self.fired_for_status,
            self.fired_for_status_changed_at.as_deref(),
            self.sent_at_update.as_deref(),
        )
    }

    fn slack_text(&self, now: DateTime<Utc>) -> String {
        slack_text(self, now)
    }
}

impl DeliverableAlert for CertificateAlert {
    fn monitor_id(&self) -> i64 {
        self.monitor_id
    }

    fn content(&self) -> (&str, &str) {
        (&self.title, &self.body)
    }

    fn is_stale(&self, current: &Monitor) -> bool {
        is_certificate_stale(
            self,
            current.cert_status,
            current.cert_last_check_at.as_deref(),
        )
    }

    fn record_delivery(&self, store: &Store) -> Result<bool, crate::error::AppError> {
        match &self.expiry_sent_at_update {
            Some(sent_at) => store.set_cert_expiry_alert_sent_at_if_current(
                self.monitor_id,
                self.fired_for_status,
                &self.fired_for_check_at,
                sent_at,
            ),
            None => Ok(true),
        }
    }

    fn slack_text(&self, now: DateTime<Utc>) -> String {
        let emoji = if self.fired_for_status == CertificateStatus::Valid {
            "⚠️"
        } else {
            "🔴"
        };
        format!(
            "{emoji} {} ({})",
            self.body,
            now.format("%Y-%m-%d %H:%M UTC")
        )
    }
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

pub fn decide_after_certificate_check(
    recorded: &RecordedCertificateCheck,
    now: DateTime<Utc>,
) -> Option<CertificateAlert> {
    let checked_at = recorded.after.cert_last_check_at.clone()?;
    match recorded.event.as_ref()? {
        CertificateEvent::BecameInvalid => Some(CertificateAlert {
            monitor_id: recorded.after.id,
            title: "Certificate invalid".into(),
            body: format!(
                "{} has an invalid TLS certificate: {}",
                recorded.after.url,
                recorded
                    .after
                    .cert_failure_reason
                    .as_deref()
                    .unwrap_or("verification failed")
            ),
            fired_for_status: CertificateStatus::Invalid,
            fired_for_check_at: checked_at,
            expiry_sent_at_update: None,
        }),
        CertificateEvent::ExpiresSoon { days_remaining } => Some(CertificateAlert {
            monitor_id: recorded.after.id,
            title: "Certificate expires soon".into(),
            body: format!(
                "{} TLS certificate expires in {}",
                recorded.after.url,
                match days_remaining {
                    0 => "less than a day".to_string(),
                    1 => "1 day".to_string(),
                    days => format!("{days} days"),
                }
            ),
            fired_for_status: CertificateStatus::Valid,
            fired_for_check_at: checked_at,
            expiry_sent_at_update: Some(now.to_rfc3339()),
        }),
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

pub fn is_certificate_stale(
    alert: &CertificateAlert,
    current_status: CertificateStatus,
    current_last_check_at: Option<&str>,
) -> bool {
    current_status != alert.fired_for_status
        || current_last_check_at != Some(alert.fired_for_check_at.as_str())
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

/// Deliver an alert to each configured channel. The monitor status is checked
/// at each delivery boundary, and bookkeeping is written only after a channel
/// accepts the alert. Channel failures are logged, never propagated.
pub async fn dispatch(app: &AppHandle, store: &Arc<Store>, alert: Alert) {
    dispatch_pending(app, store, alert).await;
}

pub async fn dispatch_certificate(app: &AppHandle, store: &Arc<Store>, alert: CertificateAlert) {
    dispatch_pending(app, store, alert).await;
}

async fn dispatch_pending<A: DeliverableAlert>(app: &AppHandle, store: &Arc<Store>, alert: A) {
    let _alerting = store.lock_alerting().await;
    let mut delivery_recorded = false;
    let (title, body) = alert.content();

    if alert_is_current(store, &alert) {
        match app.notification().builder().title(title).body(body).show() {
            Ok(()) => delivery_recorded = record_delivery(store, &alert),
            Err(e) => tracing::warn!(error = %e, "native notification failed"),
        }
    }

    match secrets::get_slack_webhook() {
        Ok(Some(webhook)) if alert_is_current(store, &alert) => {
            match slack::send(&webhook, &alert.slack_text(Utc::now())).await {
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

fn alert_is_current(store: &Store, alert: &impl DeliverableAlert) -> bool {
    let Ok(current) = store.get_monitor(alert.monitor_id()) else {
        return false;
    };
    if alert.is_stale(&current) {
        tracing::info!(
            monitor_id = alert.monitor_id(),
            "dropping stale alert: status moved on"
        );
        return false;
    }
    true
}

fn record_delivery(store: &Store, alert: &impl DeliverableAlert) -> bool {
    match alert.record_delivery(store) {
        Ok(recorded) => recorded,
        Err(error) => {
            tracing::error!(error = %error, "alert bookkeeping failed");
            false
        }
    }
}

/// Handle the alert consequences of one recorded check.
pub async fn handle_check(app: &AppHandle, store: &Arc<Store>, recorded: &RecordedCheck) {
    if let Some(alert) = decide_after_check(recorded, Utc::now()) {
        dispatch(app, store, alert).await;
    }
}

pub async fn handle_certificate_check(
    app: &AppHandle,
    store: &Arc<Store>,
    recorded: &RecordedCertificateCheck,
) {
    if let Some(alert) = decide_after_certificate_check(recorded, Utc::now()) {
        dispatch_certificate(app, store, alert).await;
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
            cert_status: CertificateStatus::NotYetChecked,
            cert_expires_at: None,
            cert_issuer: None,
            cert_failure_reason: None,
            cert_last_check_at: None,
            cert_expiry_alert_sent_at: None,
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

    #[test]
    fn certificate_alerts_format_and_guard_the_recorded_check() {
        let now = Utc::now();
        let mut after = monitor("up");
        after.cert_status = CertificateStatus::Invalid;
        after.cert_failure_reason = Some("certificate expired".into());
        after.cert_last_check_at = Some("check-1".into());
        let invalid = decide_after_certificate_check(
            &RecordedCertificateCheck {
                after,
                event: Some(CertificateEvent::BecameInvalid),
            },
            now,
        )
        .unwrap();
        assert!(invalid.body.contains("certificate expired"));
        assert!(!is_certificate_stale(
            &invalid,
            CertificateStatus::Invalid,
            Some("check-1")
        ));
        assert!(is_certificate_stale(
            &invalid,
            CertificateStatus::Valid,
            Some("check-1")
        ));

        let mut expiring_after = monitor("up");
        expiring_after.cert_status = CertificateStatus::Valid;
        expiring_after.cert_last_check_at = Some("check-2".into());
        let expiring = decide_after_certificate_check(
            &RecordedCertificateCheck {
                after: expiring_after,
                event: Some(CertificateEvent::ExpiresSoon { days_remaining: 3 }),
            },
            now,
        )
        .unwrap();
        assert!(expiring.body.contains("3 days"));
        assert!(expiring.expiry_sent_at_update.is_some());
    }
}
