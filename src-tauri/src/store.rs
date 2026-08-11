//! SQLite store — the single owner of `monitor.db`.
//!
//! Rules:
//! - Only Rust touches SQLite, and only through this module. No SQL anywhere
//!   else in the crate; the frontend goes through Tauri commands exclusively.
//! - One connection behind a mutex, held in Tauri managed state.
//! - Versioned migrations run idempotently at startup before anything else
//!   touches the DB; `PRAGMA foreign_keys = ON` on every connection.

use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ValueRef},
    Connection, OptionalExtension, Row,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

use crate::error::AppError;
use crate::history::{
    self, HistoryEvent, HistoryRange, HistoryResponse, HistoryStatus, MonitorStatus, UptimeStats,
};

/// Ordered, append-only migration list. `schema_migrations` records which
/// versions have been applied; each entry runs at most once, in a transaction.
const MIGRATIONS: &[&str] = &[
    // v1 — full v1 schema: monitors, check_results, settings.
    "
    CREATE TABLE monitors (
      id INTEGER PRIMARY KEY,
      url TEXT NOT NULL UNIQUE,
      uptime_check_enabled INTEGER NOT NULL DEFAULT 1,
      check_interval_minutes INTEGER NOT NULL DEFAULT 5,
      check_method TEXT NOT NULL DEFAULT 'GET',
      look_for_string TEXT NOT NULL DEFAULT '',
      uptime_status TEXT NOT NULL DEFAULT 'not_yet_checked',
      uptime_failure_reason TEXT,
      consecutive_failures INTEGER NOT NULL DEFAULT 0,
      status_last_change_at TEXT,
      last_check_at TEXT,
      down_alert_sent_at TEXT,
      cert_check_enabled INTEGER NOT NULL DEFAULT 0,
      cert_status TEXT NOT NULL DEFAULT 'not_yet_checked',
      cert_expires_at TEXT,
      cert_issuer TEXT,
      cert_failure_reason TEXT,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE check_results (
      id INTEGER PRIMARY KEY,
      monitor_id INTEGER NOT NULL REFERENCES monitors(id) ON DELETE CASCADE,
      checked_at TEXT NOT NULL,
      status TEXT NOT NULL,
      http_status_code INTEGER,
      response_time_ms INTEGER,
      failure_reason TEXT
    );
    CREATE INDEX idx_check_results_monitor_time ON check_results (monitor_id, checked_at);

    CREATE TABLE settings (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
    ",
    // v2 — daily certificate scheduling and expiry-alert de-duplication.
    "
    ALTER TABLE monitors ADD COLUMN cert_last_check_at TEXT;
    ALTER TABLE monitors ADD COLUMN cert_expiry_alert_sent_at TEXT;
    ",
];

/// The `monitors.uptime_status` values. Stored as text; the frontend types
/// declare the same union.
pub mod uptime_status {
    pub const NOT_YET_CHECKED: &str = "not_yet_checked";
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateStatus {
    NotYetChecked,
    Valid,
    Invalid,
}

impl CertificateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotYetChecked => "not_yet_checked",
            Self::Valid => "valid",
            Self::Invalid => "invalid",
        }
    }
}

impl FromSql for CertificateStatus {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "not_yet_checked" => Ok(Self::NotYetChecked),
            "valid" => Ok(Self::Valid),
            "invalid" => Ok(Self::Invalid),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

/// ISO-8601 UTC timestamp — the one time format stored in the DB.
pub fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckMethod {
    GET,
    HEAD,
    POST,
}

impl CheckMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckMethod::GET => "GET",
            CheckMethod::HEAD => "HEAD",
            CheckMethod::POST => "POST",
        }
    }

    fn from_db(s: &str) -> Self {
        match s {
            "HEAD" => CheckMethod::HEAD,
            "POST" => CheckMethod::POST,
            _ => CheckMethod::GET,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: i64,
    pub url: String,
    pub uptime_check_enabled: bool,
    pub check_interval_minutes: i64,
    pub check_method: CheckMethod,
    pub look_for_string: String,
    pub uptime_status: String,
    pub uptime_failure_reason: Option<String>,
    pub consecutive_failures: i64,
    pub status_last_change_at: Option<String>,
    pub last_check_at: Option<String>,
    pub down_alert_sent_at: Option<String>,
    pub cert_check_enabled: bool,
    pub cert_status: CertificateStatus,
    pub cert_expires_at: Option<String>,
    pub cert_issuer: Option<String>,
    pub cert_failure_reason: Option<String>,
    pub cert_last_check_at: Option<String>,
    pub cert_expiry_alert_sent_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Latest real check's response time; derived, not a monitors column.
    pub last_response_time_ms: Option<i64>,
}

/// Everything one recorded check produced: the monitor row before and after,
/// and the state transition that fired, if any.
#[derive(Debug, Clone)]
pub struct RecordedCheck {
    pub before: Monitor,
    pub after: Monitor,
    pub event: Option<crate::state::TransitionEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertificateEvent {
    BecameInvalid,
    ExpiresSoon { days_remaining: i64 },
}

#[derive(Debug, Clone)]
pub struct RecordedCertificateCheck {
    pub after: Monitor,
    pub event: Option<CertificateEvent>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub id: i64,
    pub monitor_id: i64,
    pub checked_at: String,
    pub status: String,
    pub http_status_code: Option<i64>,
    pub response_time_ms: Option<i64>,
    pub failure_reason: Option<String>,
}

/// Create/update payload from the frontend. `cert_check_enabled: None` means
/// "use the scheme default" (https → on, http → off); an explicit value is
/// honored for https and forced off for http.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInput {
    pub url: String,
    #[serde(default = "default_interval")]
    pub check_interval_minutes: i64,
    #[serde(default = "default_method")]
    pub check_method: CheckMethod,
    #[serde(default)]
    pub look_for_string: String,
    #[serde(default = "default_true")]
    pub uptime_check_enabled: bool,
    #[serde(default)]
    pub cert_check_enabled: Option<bool>,
}

fn default_interval() -> i64 {
    5
}
fn default_method() -> CheckMethod {
    CheckMethod::GET
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub autostart_enabled: bool,
    #[serde(default)]
    pub slack_webhook_configured: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autostart_enabled: false,
            slack_webhook_configured: false,
        }
    }
}

/// Validated + normalized form of `MonitorInput`. Cert enablement stays
/// unresolved here: `None` means "no explicit choice", which resolves to the
/// scheme default on create and to the stored value on update (http always
/// forces off — see `resolve_cert_enabled`).
struct ValidatedInput {
    url: String,
    is_https: bool,
    check_interval_minutes: i64,
    check_method: CheckMethod,
    look_for_string: String,
    uptime_check_enabled: bool,
    cert_check_requested: Option<bool>,
}

fn resolve_cert_enabled(v: &ValidatedInput, fallback: bool) -> bool {
    if v.is_https {
        v.cert_check_requested.unwrap_or(fallback)
    } else {
        false
    }
}

fn validate(input: &MonitorInput) -> Result<ValidatedInput, AppError> {
    let raw_url = input.url.trim();
    let parsed =
        url::Url::parse(raw_url).map_err(|e| AppError::InvalidUrl(format!("invalid URL: {e}")))?;
    let is_https = match parsed.scheme() {
        "https" => true,
        "http" => false,
        other => {
            return Err(AppError::InvalidUrl(format!(
                "URL must use http or https (got {other})"
            )))
        }
    };
    if parsed.host_str().is_none() {
        return Err(AppError::InvalidUrl("URL must include a host".into()));
    }
    if input.check_interval_minutes < 1 {
        return Err(AppError::InvalidInput(
            "check interval must be at least 1 minute".into(),
        ));
    }
    let look_for_string = input.look_for_string.trim().to_string();
    if !look_for_string.is_empty() && input.check_method == CheckMethod::HEAD {
        return Err(AppError::InvalidInput(
            "a HEAD check has no body to search; use GET or POST with look-for-string".into(),
        ));
    }
    Ok(ValidatedInput {
        url: raw_url.to_string(),
        is_https,
        check_interval_minutes: input.check_interval_minutes,
        check_method: input.check_method,
        look_for_string,
        uptime_check_enabled: input.uptime_check_enabled,
        cert_check_requested: input.cert_check_enabled,
    })
}

pub struct Store {
    conn: Mutex<Connection>,
    alerting: AsyncMutex<()>,
}

impl Store {
    /// Open (creating if needed) the DB at `path` and bring the schema current.
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let conn = Connection::open(path).map_err(AppError::from)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, AppError> {
        Self::init(Connection::open_in_memory().map_err(AppError::from)?)
    }

    fn init(conn: Connection) -> Result<Self, AppError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Store {
            conn: Mutex::new(conn),
            alerting: AsyncMutex::new(()),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Run all pending migrations. Idempotent: applied versions are recorded in
    /// `schema_migrations` and skipped on later runs.
    fn migrate(&self) -> Result<(), AppError> {
        let mut conn = lock_conn(&self.conn);
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
               version INTEGER PRIMARY KEY,
               applied_at TEXT NOT NULL
             );",
        )?;
        let current: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )?;
        for (idx, sql) in MIGRATIONS.iter().enumerate() {
            let version = (idx + 1) as i64;
            if version <= current {
                continue;
            }
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, now_utc()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut conn = lock_conn(&self.conn);
        f(&mut conn)
    }

    // --- monitors -----------------------------------------------------------

    pub fn list_monitors(&self) -> Result<Vec<Monitor>, AppError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {MONITOR_COLUMNS} FROM monitors ORDER BY url"
            ))?;
            let rows = stmt.query_map([], monitor_from_row)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    pub fn get_monitor(&self, id: i64) -> Result<Monitor, AppError> {
        self.with_conn(|conn| get_monitor_inner(conn, id))
    }

    pub fn create_monitor(&self, input: &MonitorInput) -> Result<Monitor, AppError> {
        let v = validate(input)?;
        self.with_conn(|conn| {
            let now = now_utc();
            conn.execute(
                "INSERT INTO monitors
                   (url, uptime_check_enabled, check_interval_minutes, check_method,
                    look_for_string, cert_check_enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![
                    v.url,
                    v.uptime_check_enabled,
                    v.check_interval_minutes,
                    v.check_method.as_str(),
                    v.look_for_string,
                    resolve_cert_enabled(&v, true), // create: https defaults on
                    now,
                ],
            )?;
            get_monitor_inner(conn, conn.last_insert_rowid())
        })
    }

    pub fn update_monitor(&self, id: i64, input: &MonitorInput) -> Result<Monitor, AppError> {
        let v = validate(input)?;
        self.with_conn(|conn| {
            let existing = get_monitor_inner(conn, id)?;
            let cert_check_enabled = resolve_cert_enabled(&v, existing.cert_check_enabled);
            let reset_certificate = cert_check_enabled
                && (!existing.cert_check_enabled || existing.url != v.url);
            let changed = conn.execute(
                "UPDATE monitors SET
                   url = ?1, uptime_check_enabled = ?2, check_interval_minutes = ?3,
                   check_method = ?4, look_for_string = ?5, cert_check_enabled = ?6,
                   cert_status = CASE WHEN ?7 THEN 'not_yet_checked' ELSE cert_status END,
                   cert_expires_at = CASE WHEN ?7 THEN NULL ELSE cert_expires_at END,
                   cert_issuer = CASE WHEN ?7 THEN NULL ELSE cert_issuer END,
                   cert_failure_reason = CASE WHEN ?7 THEN NULL ELSE cert_failure_reason END,
                   cert_last_check_at = CASE WHEN ?7 THEN NULL ELSE cert_last_check_at END,
                   cert_expiry_alert_sent_at = CASE WHEN ?7 THEN NULL ELSE cert_expiry_alert_sent_at END,
                   updated_at = ?8
                 WHERE id = ?9",
                params![
                    v.url,
                    v.uptime_check_enabled,
                    v.check_interval_minutes,
                    v.check_method.as_str(),
                    v.look_for_string,
                    cert_check_enabled,
                    reset_certificate,
                    now_utc(),
                    id,
                ],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound);
            }
            get_monitor_inner(conn, id)
        })
    }

    pub fn delete_monitor(&self, id: i64) -> Result<(), AppError> {
        self.with_conn(|conn| {
            // check_results rows go with it via ON DELETE CASCADE.
            let changed = conn.execute("DELETE FROM monitors WHERE id = ?1", params![id])?;
            if changed == 0 {
                return Err(AppError::NotFound);
            }
            Ok(())
        })
    }

    // --- history -----------------------------------------------------------

    pub fn get_uptime_stats(&self, id: i64) -> Result<UptimeStats, AppError> {
        self.get_uptime_stats_at(id, Utc::now())
    }

    fn get_uptime_stats_at(&self, id: i64, now: DateTime<Utc>) -> Result<UptimeStats, AppError> {
        self.with_conn(|conn| {
            let monitor = get_monitor_inner(conn, id)?;
            let events = load_history_events(conn, id, now - Duration::days(30), now)?;
            let status = monitor_status(&monitor)?;
            Ok(history::uptime_stats(
                &events,
                status,
                monitor.last_check_at,
                now,
            ))
        })
    }

    pub fn get_history(&self, id: i64, range: HistoryRange) -> Result<HistoryResponse, AppError> {
        self.get_history_at(id, range, Utc::now())
    }

    fn get_history_at(
        &self,
        id: i64,
        range: HistoryRange,
        now: DateTime<Utc>,
    ) -> Result<HistoryResponse, AppError> {
        self.with_conn(|conn| {
            let monitor = get_monitor_inner(conn, id)?;
            let start = now - range.duration();
            let incident_boundary: Option<String> = conn
                .query_row(
                    "SELECT checked_at FROM check_results
                     WHERE monitor_id = ?1 AND checked_at < ?2 AND status != 'down'
                     ORDER BY checked_at DESC, id DESC LIMIT 1",
                    params![id, history::format_timestamp(start)],
                    |row| row.get(0),
                )
                .optional()?;
            let load_from = incident_boundary
                .as_deref()
                .and_then(history::parse_timestamp)
                .unwrap_or(start);
            let events = load_history_events(conn, id, load_from, now)?;
            Ok(history::history(
                &events,
                monitor_status(&monitor)?,
                range,
                now,
            ))
        })
    }

    // --- check recording ----------------------------------------------------

    /// Apply one check outcome atomically: read the monitor, run the state
    /// machine, update the monitor row, insert the `check_results` row.
    pub async fn record_check(
        &self,
        id: i64,
        outcome: &crate::checker::CheckOutcome,
    ) -> Result<RecordedCheck, AppError> {
        let _alerting = self.alerting.lock().await;
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            let monitor = tx.query_row(
                &format!("SELECT {MONITOR_COLUMNS} FROM monitors WHERE id = ?1"),
                params![id],
                monitor_from_row,
            )?;
            let now = now_utc();
            let change = crate::state::apply(&monitor, outcome, &now);
            tx.execute(
                "UPDATE monitors SET
                   uptime_status = ?1,
                   consecutive_failures = ?2,
                   uptime_failure_reason = ?3,
                   status_last_change_at = COALESCE(?4, status_last_change_at),
                   last_check_at = ?5,
                   updated_at = ?5
                 WHERE id = ?6",
                params![
                    change.uptime_status,
                    change.consecutive_failures,
                    change.uptime_failure_reason,
                    change.status_changed_at,
                    now,
                    id,
                ],
            )?;
            tx.execute(
                "INSERT INTO check_results
                   (monitor_id, checked_at, status, http_status_code, response_time_ms, failure_reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    now,
                    change.result_status,
                    outcome.http_status,
                    outcome.response_time_ms,
                    outcome.failure_reason,
                ],
            )?;
            let updated = tx.query_row(
                &format!("SELECT {MONITOR_COLUMNS} FROM monitors WHERE id = ?1"),
                params![id],
                monitor_from_row,
            )?;
            tx.commit()?;
            Ok(RecordedCheck {
                before: monitor,
                after: updated,
                event: change.event,
            })
        })
    }

    pub fn record_certificate_check(
        &self,
        id: i64,
        outcome: &crate::certificate::CertificateOutcome,
    ) -> Result<Option<RecordedCertificateCheck>, AppError> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            let before = tx.query_row(
                &format!("SELECT {MONITOR_COLUMNS} FROM monitors WHERE id = ?1"),
                params![id],
                monitor_from_row,
            )?;
            if !before.cert_check_enabled {
                return Ok(None);
            }
            let now = now_utc();
            let status = if outcome.valid {
                CertificateStatus::Valid
            } else {
                CertificateStatus::Invalid
            };
            let event = certificate_event(&before, outcome, &now);
            tx.execute(
                "UPDATE monitors SET
                   cert_status = ?1, cert_expires_at = ?2, cert_issuer = ?3,
                   cert_failure_reason = ?4, cert_last_check_at = ?5,
                   updated_at = ?5
                 WHERE id = ?6",
                params![
                    status.as_str(),
                    outcome.expires_at,
                    outcome.issuer,
                    outcome.failure_reason,
                    now,
                    id,
                ],
            )?;
            let after = tx.query_row(
                &format!("SELECT {MONITOR_COLUMNS} FROM monitors WHERE id = ?1"),
                params![id],
                monitor_from_row,
            )?;
            tx.commit()?;
            Ok(Some(RecordedCertificateCheck { after, event }))
        })
    }

    /// Prevent a monitor transition from racing alert delivery/bookkeeping.
    /// The lock is global because alert volume is tiny and delivery is already
    /// sequential; keeping it here gives every check path the same ordering.
    pub async fn lock_alerting(&self) -> AsyncMutexGuard<'_, ()> {
        self.alerting.lock().await
    }

    /// Update alert bookkeeping only while the monitor is still in the state
    /// that caused the alert. Returns false when the alert became stale.
    pub fn set_down_alert_sent_at_if_status(
        &self,
        id: i64,
        expected_status: &str,
        expected_status_changed_at: Option<&str>,
        sent_at: Option<&str>,
    ) -> Result<bool, AppError> {
        self.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE monitors SET down_alert_sent_at = ?1
                 WHERE id = ?2 AND uptime_status = ?3
                   AND status_last_change_at IS ?4",
                params![sent_at, id, expected_status, expected_status_changed_at],
            )?;
            Ok(changed == 1)
        })
    }

    pub fn set_cert_expiry_alert_sent_at_if_current(
        &self,
        id: i64,
        expected_status: CertificateStatus,
        expected_last_check_at: &str,
        sent_at: &str,
    ) -> Result<bool, AppError> {
        self.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE monitors SET cert_expiry_alert_sent_at = ?1
                 WHERE id = ?2 AND cert_status = ?3 AND cert_last_check_at = ?4",
                params![
                    sent_at,
                    id,
                    expected_status.as_str(),
                    expected_last_check_at
                ],
            )?;
            Ok(changed == 1)
        })
    }

    // --- settings -----------------------------------------------------------

    pub fn get_settings(&self) -> Result<Settings, AppError> {
        self.with_conn(|conn| {
            let autostart: Option<String> = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'autostart_enabled'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(Settings {
                autostart_enabled: autostart.as_deref() == Some("true"),
                // Enriched by the command layer from Keychain. The SQLite
                // store deliberately has no access to secrets.
                slack_webhook_configured: false,
            })
        })
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), AppError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('autostart_enabled', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![if settings.autostart_enabled {
                    "true"
                } else {
                    "false"
                }],
            )?;
            Ok(())
        })
    }
}

/// Lock the connection, recovering from poisoning: a panic elsewhere must not
/// cascade panics through every later command. Any in-flight transaction was
/// already rolled back on drop, so the connection itself is safe to reuse.
fn lock_conn(conn: &Mutex<Connection>) -> std::sync::MutexGuard<'_, Connection> {
    conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

const MONITOR_COLUMNS: &str = "id, url, uptime_check_enabled, check_interval_minutes, \
     check_method, look_for_string, uptime_status, uptime_failure_reason, \
     consecutive_failures, status_last_change_at, last_check_at, down_alert_sent_at, \
     cert_check_enabled, cert_status, cert_expires_at, cert_issuer, \
     cert_failure_reason, cert_last_check_at, cert_expiry_alert_sent_at, \
     created_at, updated_at, \
     (SELECT response_time_ms FROM check_results c \
        WHERE c.monitor_id = monitors.id AND c.status != 'gap' \
        ORDER BY c.checked_at DESC, c.id DESC LIMIT 1) AS last_response_time_ms";

fn get_monitor_inner(conn: &Connection, id: i64) -> Result<Monitor, AppError> {
    conn.query_row(
        &format!("SELECT {MONITOR_COLUMNS} FROM monitors WHERE id = ?1"),
        params![id],
        monitor_from_row,
    )
    .map_err(AppError::from)
}

fn monitor_from_row(row: &Row) -> Result<Monitor, rusqlite::Error> {
    Ok(Monitor {
        id: row.get(0)?,
        url: row.get(1)?,
        uptime_check_enabled: row.get(2)?,
        check_interval_minutes: row.get(3)?,
        check_method: CheckMethod::from_db(&row.get::<_, String>(4)?),
        look_for_string: row.get(5)?,
        uptime_status: row.get(6)?,
        uptime_failure_reason: row.get(7)?,
        consecutive_failures: row.get(8)?,
        status_last_change_at: row.get(9)?,
        last_check_at: row.get(10)?,
        down_alert_sent_at: row.get(11)?,
        cert_check_enabled: row.get(12)?,
        cert_status: row.get(13)?,
        cert_expires_at: row.get(14)?,
        cert_issuer: row.get(15)?,
        cert_failure_reason: row.get(16)?,
        cert_last_check_at: row.get(17)?,
        cert_expiry_alert_sent_at: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        last_response_time_ms: row.get(21)?,
    })
}

fn certificate_event(
    before: &Monitor,
    outcome: &crate::certificate::CertificateOutcome,
    checked_at: &str,
) -> Option<CertificateEvent> {
    if !outcome.valid {
        return (before.cert_status != CertificateStatus::Invalid)
            .then_some(CertificateEvent::BecameInvalid);
    }
    let expires_at = outcome
        .expires_at
        .as_deref()
        .and_then(history::parse_timestamp)?;
    let checked_at = history::parse_timestamp(checked_at)?;
    let seconds_remaining = expires_at.signed_duration_since(checked_at).num_seconds();
    if seconds_remaining < 0 {
        return None;
    }
    let days_remaining = (seconds_remaining + 86_399) / 86_400;
    if !(0..=crate::certificate::EXPIRY_WARNING_DAYS).contains(&days_remaining) {
        return None;
    }
    let already_alerted_today = before
        .cert_expiry_alert_sent_at
        .as_deref()
        .and_then(history::parse_timestamp)
        .is_some_and(|sent_at| checked_at.signed_duration_since(sent_at) < Duration::hours(24));
    (!already_alerted_today).then_some(CertificateEvent::ExpiresSoon { days_remaining })
}

fn load_history_events(
    conn: &Connection,
    monitor_id: i64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<HistoryEvent>, AppError> {
    let start_text = history::format_timestamp(start);
    let end_text = history::format_timestamp(end);
    let predecessor: Option<(String, String, Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT checked_at, status, response_time_ms, failure_reason
             FROM check_results
             WHERE monitor_id = ?1 AND checked_at < ?2
             ORDER BY checked_at DESC, id DESC LIMIT 1",
            params![monitor_id, start_text],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let mut events = predecessor
        .map(history_event_from_values)
        .transpose()?
        .into_iter()
        .collect::<Vec<_>>();
    let mut statement = conn.prepare(
        "SELECT checked_at, status, response_time_ms, failure_reason
         FROM check_results
         WHERE monitor_id = ?1 AND checked_at >= ?2 AND checked_at < ?3
         ORDER BY checked_at, id",
    )?;
    let rows = statement.query_map(params![monitor_id, start_text, end_text], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    for row in rows {
        events.push(history_event_from_values(row?)?);
    }
    Ok(events)
}

fn history_event_from_values(
    values: (String, String, Option<i64>, Option<String>),
) -> Result<HistoryEvent, AppError> {
    let checked_at = history::parse_timestamp(&values.0).ok_or_else(|| {
        AppError::Db(format!(
            "invalid check timestamp stored for history: {}",
            values.0
        ))
    })?;
    Ok(HistoryEvent {
        checked_at,
        status: HistoryStatus::from_db(&values.1).ok_or_else(|| {
            AppError::Db(format!(
                "invalid check status stored for history: {}",
                values.1
            ))
        })?,
        response_time_ms: values.2,
        failure_reason: values.3,
    })
}

fn monitor_status(monitor: &Monitor) -> Result<MonitorStatus, AppError> {
    MonitorStatus::from_db(&monitor.uptime_status).ok_or_else(|| {
        AppError::Db(format!(
            "invalid monitor status stored for history: {}",
            monitor.uptime_status
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::{Duration as StdDuration, Instant};

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

    fn insert_result(
        store: &Store,
        monitor_id: i64,
        checked_at: DateTime<Utc>,
        status: &str,
        response_time_ms: Option<i64>,
        failure_reason: Option<&str>,
    ) {
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO check_results
                       (monitor_id, checked_at, status, response_time_ms, failure_reason)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        monitor_id,
                        history::format_timestamp(checked_at),
                        status,
                        response_time_ms,
                        failure_reason
                    ],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn create_list_update_delete_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let m = store.create_monitor(&input("https://example.com")).unwrap();
        assert_eq!(m.url, "https://example.com");
        assert_eq!(m.uptime_status, "not_yet_checked");
        assert_eq!(store.list_monitors().unwrap().len(), 1);

        let mut upd = input("https://example.com/health");
        upd.check_interval_minutes = 10;
        upd.check_method = CheckMethod::POST;
        let m2 = store.update_monitor(m.id, &upd).unwrap();
        assert_eq!(m2.url, "https://example.com/health");
        assert_eq!(m2.check_interval_minutes, 10);
        assert_eq!(m2.check_method, CheckMethod::POST);

        store.delete_monitor(m.id).unwrap();
        assert!(store.list_monitors().unwrap().is_empty());
        assert!(matches!(store.get_monitor(m.id), Err(AppError::NotFound)));
    }

    #[test]
    fn rejects_bad_scheme() {
        let store = Store::open_in_memory().unwrap();
        for url in ["ftp://example.com", "file:///etc/passwd", "not a url"] {
            let err = store.create_monitor(&input(url)).unwrap_err();
            assert!(matches!(err, AppError::InvalidUrl(_)), "url: {url}");
        }
    }

    #[test]
    fn rejects_duplicate_url() {
        let store = Store::open_in_memory().unwrap();
        store.create_monitor(&input("https://example.com")).unwrap();
        let err = store
            .create_monitor(&input("https://example.com"))
            .unwrap_err();
        assert!(matches!(err, AppError::DuplicateUrl));

        // Also on update: renaming another monitor onto an existing URL.
        let other = store.create_monitor(&input("https://other.com")).unwrap();
        let err = store
            .update_monitor(other.id, &input("https://example.com"))
            .unwrap_err();
        assert!(matches!(err, AppError::DuplicateUrl));

        // Updating a monitor to its own URL is not a duplicate.
        store
            .update_monitor(other.id, &input("https://other.com"))
            .unwrap();
    }

    #[test]
    fn rejects_interval_below_one() {
        let store = Store::open_in_memory().unwrap();
        let mut i = input("https://example.com");
        i.check_interval_minutes = 0;
        assert!(matches!(
            store.create_monitor(&i).unwrap_err(),
            AppError::InvalidInput(_)
        ));
    }

    #[test]
    fn rejects_head_with_look_for_string() {
        let store = Store::open_in_memory().unwrap();
        let mut i = input("https://example.com");
        i.check_method = CheckMethod::HEAD;
        i.look_for_string = "ok".into();
        assert!(matches!(
            store.create_monitor(&i).unwrap_err(),
            AppError::InvalidInput(_)
        ));
    }

    #[test]
    fn cert_check_defaults_follow_scheme() {
        let store = Store::open_in_memory().unwrap();
        // https defaults on; explicit off is honored.
        let https = store.create_monitor(&input("https://a.example")).unwrap();
        assert!(https.cert_check_enabled);
        let mut off = input("https://b.example");
        off.cert_check_enabled = Some(false);
        assert!(!store.create_monitor(&off).unwrap().cert_check_enabled);
        // http forces off even when explicitly requested.
        let mut http = input("http://c.example");
        http.cert_check_enabled = Some(true);
        assert!(!store.create_monitor(&http).unwrap().cert_check_enabled);
    }

    #[test]
    fn enabling_or_retargeting_certificate_checks_resets_stale_metadata() {
        let store = Store::open_in_memory().unwrap();
        let monitor = store.create_monitor(&input("https://a.example")).unwrap();
        store
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE monitors SET cert_status = 'valid', cert_last_check_at = ?1,
                     cert_expires_at = ?2, cert_issuer = 'issuer'
                     WHERE id = ?3",
                    params![now_utc(), now_utc(), monitor.id],
                )?;
                Ok(())
            })
            .unwrap();
        let mut disabled = input("https://a.example");
        disabled.cert_check_enabled = Some(false);
        store.update_monitor(monitor.id, &disabled).unwrap();

        let mut enabled_input = input("https://a.example");
        enabled_input.cert_check_enabled = Some(true);
        let enabled = store.update_monitor(monitor.id, &enabled_input).unwrap();
        assert!(enabled.cert_check_enabled);
        assert_eq!(enabled.cert_status, CertificateStatus::NotYetChecked);
        assert!(enabled.cert_last_check_at.is_none());
        assert!(enabled.cert_expires_at.is_none());

        let retargeted = store
            .update_monitor(monitor.id, &input("https://b.example"))
            .unwrap();
        assert_eq!(retargeted.cert_status, CertificateStatus::NotYetChecked);
        assert!(retargeted.cert_last_check_at.is_none());
    }

    #[test]
    fn update_without_cert_choice_preserves_stored_toggle() {
        let store = Store::open_in_memory().unwrap();
        let mut off = input("https://a.example");
        off.cert_check_enabled = Some(false);
        let m = store.create_monitor(&off).unwrap();
        assert!(!m.cert_check_enabled);

        // No explicit choice on update → stored value survives.
        let updated = store
            .update_monitor(m.id, &input("https://a.example"))
            .unwrap();
        assert!(!updated.cert_check_enabled);

        // Explicit choice on update is honored.
        let mut on = input("https://a.example");
        on.cert_check_enabled = Some(true);
        assert!(store.update_monitor(m.id, &on).unwrap().cert_check_enabled);

        // Switching to http forces it off.
        let http = input("http://a.example");
        assert!(
            !store
                .update_monitor(m.id, &http)
                .unwrap()
                .cert_check_enabled
        );
    }

    #[test]
    fn migrations_are_idempotent() {
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap();
        let versions = store
            .with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT version FROM schema_migrations")?;
                let v = stmt.query_map([], |r| r.get::<_, i64>(0))?;
                Ok(v.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap();
        assert_eq!(versions, vec![1, 2]);
        let certificate_columns = store
            .with_conn(|conn| {
                let mut statement = conn.prepare("PRAGMA table_info(monitors)")?;
                let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap();
        assert!(certificate_columns.contains(&"cert_last_check_at".into()));
        assert!(certificate_columns.contains(&"cert_expiry_alert_sent_at".into()));
        // Still fully usable afterwards.
        store.create_monitor(&input("https://example.com")).unwrap();
    }

    #[test]
    fn delete_cascades_check_results() {
        let store = Store::open_in_memory().unwrap();
        let m = store.create_monitor(&input("https://example.com")).unwrap();
        store
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO check_results (monitor_id, checked_at, status) VALUES (?1, ?2, 'up')",
                    params![m.id, now_utc()],
                )?;
                Ok(())
            })
            .unwrap();
        store.delete_monitor(m.id).unwrap();
        let count: i64 = store
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM check_results", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn settings_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.get_settings().unwrap().autostart_enabled);
        store
            .save_settings(&Settings {
                autostart_enabled: true,
                slack_webhook_configured: false,
            })
            .unwrap();
        assert!(store.get_settings().unwrap().autostart_enabled);
    }

    #[test]
    fn history_stats_are_duration_weighted_and_exclude_gaps() {
        let store = Store::open_in_memory().unwrap();
        let monitor = store.create_monitor(&input("https://example.com")).unwrap();
        let start = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let now = start + Duration::hours(24);
        insert_result(&store, monitor.id, start, "up", Some(100), None);
        insert_result(
            &store,
            monitor.id,
            start + Duration::hours(12),
            "down",
            Some(300),
            Some("timeout"),
        );
        insert_result(
            &store,
            monitor.id,
            start + Duration::hours(18),
            "gap",
            None,
            None,
        );

        let stats = store.get_uptime_stats_at(monitor.id, now).unwrap();

        assert_eq!(stats.uptime_24h, Some(66.7));
        assert_eq!(stats.avg_response_time_ms_24h, Some(200.0));
        let history = store
            .get_history_at(monitor.id, HistoryRange::Day, now)
            .unwrap();
        assert_eq!(history.points.len(), 3);
        assert_eq!(history.points[2].status, crate::history::PointStatus::Gap);
        assert_eq!(history.incidents.len(), 1);
        assert!(history.incidents[0].includes_gap);
    }

    #[test]
    fn history_queries_use_the_monitor_time_index() {
        let store = Store::open_in_memory().unwrap();
        let details = store
            .with_conn(|conn| {
                let plans = [
                    "EXPLAIN QUERY PLAN
                         SELECT checked_at, status, response_time_ms, failure_reason
                         FROM check_results
                         WHERE monitor_id = 1 AND checked_at >= '2026-01-01'
                           AND checked_at < '2026-02-01'
                         ORDER BY checked_at, id",
                    "EXPLAIN QUERY PLAN
                         SELECT checked_at, status, response_time_ms, failure_reason
                         FROM check_results
                         WHERE monitor_id = 1 AND checked_at < '2026-01-01'
                         ORDER BY checked_at DESC, id DESC LIMIT 1",
                ];
                let mut details = Vec::new();
                for sql in plans {
                    let mut statement = conn.prepare(sql)?;
                    let rows = statement.query_map([], |row| row.get::<_, String>(3))?;
                    details.push(rows.collect::<Result<Vec<_>, _>>()?.join(" "));
                }
                Ok(details)
            })
            .unwrap();

        for detail in details {
            assert!(
                detail.contains("idx_check_results_monitor_time"),
                "{detail}"
            );
        }
    }

    #[test]
    fn month_history_stays_fast_with_one_hundred_thousand_results() {
        let store = Store::open_in_memory().unwrap();
        let monitor = store.create_monitor(&input("https://example.com")).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let start = now - Duration::days(30);
        store
            .with_conn(|conn| {
                let tx = conn.transaction()?;
                {
                    let mut statement = tx.prepare(
                        "INSERT INTO check_results
                           (monitor_id, checked_at, status, response_time_ms)
                         VALUES (?1, ?2, 'up', 100)",
                    )?;
                    for index in 0..100_000_i64 {
                        let checked_at = start + Duration::milliseconds(index * 25_920);
                        statement
                            .execute(params![monitor.id, history::format_timestamp(checked_at)])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .unwrap();

        let started = Instant::now();
        let history = store
            .get_history_at(monitor.id, HistoryRange::Month, now)
            .unwrap();

        assert!(started.elapsed() < StdDuration::from_secs(1));
        assert!(history.points.len() <= 500);
    }

    #[test]
    fn dashboard_queries_twenty_monitors_without_jank() {
        let store = Store::open_in_memory().unwrap();
        let monitors: Vec<Monitor> = (0..20)
            .map(|index| {
                store
                    .create_monitor(&input(&format!("https://{index}.example.com")))
                    .unwrap()
            })
            .collect();
        let now = Utc.with_ymd_and_hms(2026, 8, 11, 0, 0, 0).unwrap();
        let start = now - Duration::days(30);
        store
            .with_conn(|conn| {
                let tx = conn.transaction()?;
                {
                    let mut statement = tx.prepare(
                        "INSERT INTO check_results
                           (monitor_id, checked_at, status, response_time_ms)
                         VALUES (?1, ?2, 'up', 100)",
                    )?;
                    for monitor in &monitors {
                        for index in 0..8_640_i64 {
                            let checked_at = start + Duration::minutes(index * 5);
                            statement.execute(params![
                                monitor.id,
                                history::format_timestamp(checked_at)
                            ])?;
                        }
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .unwrap();

        let started = Instant::now();
        for monitor in &monitors {
            store.get_uptime_stats_at(monitor.id, now).unwrap();
            store
                .get_history_at(monitor.id, HistoryRange::Day, now)
                .unwrap();
        }

        assert!(started.elapsed() < StdDuration::from_secs(1));
    }

    #[test]
    fn certificate_alert_events_are_deduplicated() {
        let store = Store::open_in_memory().unwrap();
        let monitor = store.create_monitor(&input("https://example.com")).unwrap();
        let invalid = crate::certificate::CertificateOutcome {
            valid: false,
            expires_at: None,
            issuer: None,
            failure_reason: Some("self-signed certificate".into()),
        };

        let first = store
            .record_certificate_check(monitor.id, &invalid)
            .unwrap()
            .unwrap();
        assert_eq!(first.event, Some(CertificateEvent::BecameInvalid));
        let repeat = store
            .record_certificate_check(monitor.id, &invalid)
            .unwrap()
            .unwrap();
        assert_eq!(repeat.event, None);

        let far_future = crate::certificate::CertificateOutcome {
            valid: true,
            expires_at: Some((Utc::now() + Duration::days(30)).to_rfc3339()),
            issuer: Some("issuer".into()),
            failure_reason: None,
        };
        store
            .record_certificate_check(monitor.id, &far_future)
            .unwrap();
        let invalid_again = store
            .record_certificate_check(monitor.id, &invalid)
            .unwrap()
            .unwrap();
        assert_eq!(invalid_again.event, Some(CertificateEvent::BecameInvalid));
    }

    #[test]
    fn expiry_alert_fires_at_most_once_per_day() {
        let store = Store::open_in_memory().unwrap();
        let monitor = store.create_monitor(&input("https://example.com")).unwrap();
        let expiring = crate::certificate::CertificateOutcome {
            valid: true,
            expires_at: Some((Utc::now() + Duration::days(9)).to_rfc3339()),
            issuer: Some("issuer".into()),
            failure_reason: None,
        };
        let first = store
            .record_certificate_check(monitor.id, &expiring)
            .unwrap()
            .unwrap();
        assert!(matches!(
            first.event,
            Some(CertificateEvent::ExpiresSoon { days_remaining: 9 })
        ));
        store
            .set_cert_expiry_alert_sent_at_if_current(
                monitor.id,
                CertificateStatus::Valid,
                first.after.cert_last_check_at.as_deref().unwrap(),
                &now_utc(),
            )
            .unwrap();

        let repeated = store
            .record_certificate_check(monitor.id, &expiring)
            .unwrap()
            .unwrap();
        assert_eq!(repeated.event, None);
    }
}
