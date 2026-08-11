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

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

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
];

/// The `monitors.uptime_status` values. Stored as text; the frontend types
/// declare the same union.
pub mod uptime_status {
    pub const NOT_YET_CHECKED: &str = "not_yet_checked";
    pub const UP: &str = "up";
    pub const DOWN: &str = "down";
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
    pub cert_status: String,
    pub cert_expires_at: Option<String>,
    pub cert_issuer: Option<String>,
    pub cert_failure_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Latest real check's response time; derived, not a monitors column.
    pub last_response_time_ms: Option<i64>,
}

// Not constructed yet; the check engine will produce these rows.
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            autostart_enabled: false,
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
    let parsed = url::Url::parse(raw_url)
        .map_err(|e| AppError::InvalidUrl(format!("invalid URL: {e}")))?;
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
            let changed = conn.execute(
                "UPDATE monitors SET
                   url = ?1, uptime_check_enabled = ?2, check_interval_minutes = ?3,
                   check_method = ?4, look_for_string = ?5, cert_check_enabled = ?6,
                   updated_at = ?7
                 WHERE id = ?8",
                params![
                    v.url,
                    v.uptime_check_enabled,
                    v.check_interval_minutes,
                    v.check_method.as_str(),
                    v.look_for_string,
                    // update: no explicit choice preserves the stored toggle
                    resolve_cert_enabled(&v, existing.cert_check_enabled),
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

    // --- check recording ----------------------------------------------------

    /// Apply one check outcome atomically: read the monitor, run the state
    /// machine, update the monitor row, insert the `check_results` row.
    /// Returns the updated monitor and the transition that fired, if any.
    pub fn record_check(
        &self,
        id: i64,
        outcome: &crate::checker::CheckOutcome,
    ) -> Result<(Monitor, Option<crate::state::TransitionEvent>), AppError> {
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
            Ok((updated, change.event))
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
            })
        })
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), AppError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('autostart_enabled', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![if settings.autostart_enabled { "true" } else { "false" }],
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
     cert_failure_reason, created_at, updated_at, \
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
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        last_response_time_ms: row.get(19)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn update_without_cert_choice_preserves_stored_toggle() {
        let store = Store::open_in_memory().unwrap();
        let mut off = input("https://a.example");
        off.cert_check_enabled = Some(false);
        let m = store.create_monitor(&off).unwrap();
        assert!(!m.cert_check_enabled);

        // No explicit choice on update → stored value survives.
        let updated = store.update_monitor(m.id, &input("https://a.example")).unwrap();
        assert!(!updated.cert_check_enabled);

        // Explicit choice on update is honored.
        let mut on = input("https://a.example");
        on.cert_check_enabled = Some(true);
        assert!(store.update_monitor(m.id, &on).unwrap().cert_check_enabled);

        // Switching to http forces it off.
        let http = input("http://a.example");
        assert!(!store.update_monitor(m.id, &http).unwrap().cert_check_enabled);
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
        assert_eq!(versions, vec![1]);
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
            })
            .unwrap();
        assert!(store.get_settings().unwrap().autostart_enabled);
    }
}
