//! Pure up/down state machine applied after every check.
//!
//! One failure is suspicion, not state: the visible status only flips to
//! `down` at two consecutive failures, and back to `up` on the next success.
//! The returned event tells the alerting layer which transition (if any)
//! fired; deciding whether to notify is its concern, not this module's.

use crate::checker::CheckOutcome;
use crate::store::{uptime_status, Monitor};

pub const DOWN_THRESHOLD: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionEvent {
    WentDown,
    Recovered,
}

/// The monitor-row updates and result row produced by one check.
#[derive(Debug, Clone)]
pub struct StateChange {
    pub uptime_status: &'static str,
    pub consecutive_failures: i64,
    pub uptime_failure_reason: Option<String>,
    /// `Some(now)` when the visible status changed this check.
    pub status_changed_at: Option<String>,
    /// Status recorded on the `check_results` row (`up`/`down`) — the raw
    /// outcome of this check, independent of the visible status.
    pub result_status: &'static str,
    pub event: Option<TransitionEvent>,
}

pub fn apply(monitor: &Monitor, outcome: &CheckOutcome, now: &str) -> StateChange {
    if outcome.success {
        let recovered = monitor.uptime_status == uptime_status::DOWN;
        StateChange {
            uptime_status: uptime_status::UP,
            consecutive_failures: 0,
            uptime_failure_reason: None,
            status_changed_at: (monitor.uptime_status != uptime_status::UP)
                .then(|| now.to_string()),
            result_status: uptime_status::UP,
            event: recovered.then_some(TransitionEvent::Recovered),
        }
    } else {
        let failures = monitor.consecutive_failures + 1;
        let went_down =
            monitor.uptime_status != uptime_status::DOWN && failures >= DOWN_THRESHOLD;
        StateChange {
            uptime_status: if went_down || monitor.uptime_status == uptime_status::DOWN {
                uptime_status::DOWN
            } else if monitor.uptime_status == uptime_status::UP {
                // Below the threshold the previous visible status stands.
                uptime_status::UP
            } else {
                uptime_status::NOT_YET_CHECKED
            },
            consecutive_failures: failures,
            uptime_failure_reason: outcome.failure_reason.clone(),
            status_changed_at: went_down.then(|| now.to_string()),
            result_status: uptime_status::DOWN,
            event: went_down.then_some(TransitionEvent::WentDown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(status: &str, failures: i64) -> Monitor {
        Monitor {
            id: 1,
            url: "https://example.com".into(),
            uptime_check_enabled: true,
            check_interval_minutes: 5,
            check_method: crate::store::CheckMethod::GET,
            look_for_string: String::new(),
            uptime_status: status.into(),
            uptime_failure_reason: None,
            consecutive_failures: failures,
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

    fn success() -> CheckOutcome {
        CheckOutcome {
            success: true,
            http_status: Some(200),
            response_time_ms: 42,
            failure_reason: None,
        }
    }

    fn failure() -> CheckOutcome {
        CheckOutcome {
            success: false,
            http_status: None,
            response_time_ms: 42,
            failure_reason: Some("timeout".into()),
        }
    }

    #[test]
    fn first_success_flips_to_up_with_status_change() {
        let change = apply(&monitor("not_yet_checked", 0), &success(), "T");
        assert_eq!(change.uptime_status, "up");
        assert_eq!(change.consecutive_failures, 0);
        assert_eq!(change.status_changed_at.as_deref(), Some("T"));
        assert_eq!(change.result_status, "up");
        assert!(change.event.is_none());
    }

    #[test]
    fn success_while_up_changes_nothing_visible() {
        let change = apply(&monitor("up", 0), &success(), "T");
        assert_eq!(change.uptime_status, "up");
        assert!(change.status_changed_at.is_none());
        assert!(change.event.is_none());
    }

    #[test]
    fn success_after_down_recovers() {
        let change = apply(&monitor("down", 5), &success(), "T");
        assert_eq!(change.uptime_status, "up");
        assert_eq!(change.consecutive_failures, 0);
        assert!(change.uptime_failure_reason.is_none());
        assert_eq!(change.status_changed_at.as_deref(), Some("T"));
        assert_eq!(change.event, Some(TransitionEvent::Recovered));
    }

    #[test]
    fn single_failure_is_suspicion_not_state() {
        for status in ["up", "not_yet_checked"] {
            let change = apply(&monitor(status, 0), &failure(), "T");
            assert_eq!(change.uptime_status, status, "from {status}");
            assert_eq!(change.consecutive_failures, 1);
            assert_eq!(change.uptime_failure_reason.as_deref(), Some("timeout"));
            assert!(change.status_changed_at.is_none());
            assert_eq!(change.result_status, "down");
            assert!(change.event.is_none());
        }
    }

    #[test]
    fn second_failure_goes_down_and_fires() {
        for status in ["up", "not_yet_checked"] {
            let change = apply(&monitor(status, 1), &failure(), "T");
            assert_eq!(change.uptime_status, "down", "from {status}");
            assert_eq!(change.consecutive_failures, 2);
            assert_eq!(change.status_changed_at.as_deref(), Some("T"));
            assert_eq!(change.event, Some(TransitionEvent::WentDown));
        }
    }

    #[test]
    fn failure_while_down_stays_down_without_refiring() {
        let change = apply(&monitor("down", 2), &failure(), "T");
        assert_eq!(change.uptime_status, "down");
        assert_eq!(change.consecutive_failures, 3);
        assert!(change.status_changed_at.is_none());
        assert!(change.event.is_none());
    }
}
