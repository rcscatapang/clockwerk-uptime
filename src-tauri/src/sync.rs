//! JSON monitor sync: file format, parsing, and the shapes the UI renders.
//!
//! The file is a JSON array of objects whose legal keys are exactly the
//! user-editable monitor fields — `SyncEntry` below is the single source of
//! truth for that set, and `deny_unknown_fields` makes anything else an error
//! rather than a silent write. Status and history columns are never importable.
//!
//! Reading files stays on this side of the wall: the frontend only passes a
//! path obtained from the Tauri dialog plugin, and has no filesystem
//! capability of its own.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::store::{CheckMethod, Monitor, MonitorInput, DEFAULT_CHECK_INTERVAL_MINUTES};

/// Generous for a monitor list; anything larger is a mistaken file.
pub const MAX_SYNC_FILE_BYTES: u64 = 1_048_576;

/// One entry of the sync file. Every key except `url` is optional: absent keys
/// leave the current value untouched on an existing monitor, and fall back to
/// the create defaults on a new one.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncEntry {
    pub url: String,
    #[serde(default)]
    pub uptime_check_enabled: Option<bool>,
    #[serde(default)]
    pub check_interval_minutes: Option<i64>,
    #[serde(default)]
    pub check_method: Option<CheckMethod>,
    #[serde(default)]
    pub look_for_string: Option<String>,
    #[serde(default)]
    pub cert_check_enabled: Option<bool>,
}

/// What a sync would do, grouped for the confirm dialog. URLs, not ids, so the
/// preview reads the same as the file the user picked.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub delete_missing: bool,
    pub to_add: Vec<String>,
    pub to_update: Vec<String>,
    pub to_delete: Vec<String>,
    pub unchanged: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
}

/// Read and parse a sync file. Size is checked before the read so a huge file
/// is never pulled into memory.
pub fn read_entries(path: &Path) -> Result<Vec<SyncEntry>, AppError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| AppError::InvalidInput(format!("could not open the file: {}", e.kind())))?;
    if !metadata.is_file() {
        return Err(AppError::InvalidInput(
            "the selected path is not a file".into(),
        ));
    }
    if metadata.len() > MAX_SYNC_FILE_BYTES {
        return Err(AppError::InvalidInput(format!(
            "the file is larger than the {} MB import limit",
            MAX_SYNC_FILE_BYTES / 1_048_576
        )));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| AppError::InvalidInput(format!("could not read the file: {}", e.kind())))?;
    parse_entries(&text)
}

/// Parse the file text. Entries are deserialized one at a time so a problem is
/// reported against the entry that caused it instead of a byte offset.
pub fn parse_entries(text: &str) -> Result<Vec<SyncEntry>, AppError> {
    let values: Vec<serde_json::Value> = serde_json::from_str(text).map_err(|_| {
        AppError::InvalidInput("the file must contain a JSON array of monitor objects".into())
    })?;
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            serde_json::from_value(value)
                .map_err(|e| entry_error(index, readable_serde_message(&e.to_string())))
        })
        .collect()
}

/// Prefix a message with the 1-based entry number the user sees in their file.
pub fn entry_error(index: usize, message: String) -> AppError {
    AppError::InvalidInput(format!("entry {}: {message}", index + 1))
}

/// Keep the part of a serde message that names the offending key or value and
/// drop its byte position: line/column mean nothing once entries are reported
/// by number.
fn readable_serde_message(message: &str) -> String {
    match message.find(" at line ") {
        Some(cut) => message[..cut].to_string(),
        None => message.to_string(),
    }
}

/// Merge one entry over the monitor it matched (or over the create defaults
/// when it matched none): a key absent from the entry keeps the current value.
///
/// One merge-time rule beyond that: a stored look-for string cannot survive a
/// switch to HEAD (there is no body to search), so an entry that asks for HEAD
/// without naming `look_for_string` clears it instead of aborting the run on a
/// contradiction the file never stated.
pub fn merge_entry(entry: &SyncEntry, current: Option<&Monitor>) -> MonitorInput {
    let check_method = entry
        .check_method
        .or_else(|| current.map(|m| m.check_method))
        .unwrap_or(CheckMethod::GET);
    let look_for_string = entry
        .look_for_string
        .clone()
        .or_else(|| current.map(|m| m.look_for_string.clone()))
        .unwrap_or_default();
    MonitorInput {
        url: entry.url.clone(),
        check_interval_minutes: entry
            .check_interval_minutes
            .or_else(|| current.map(|m| m.check_interval_minutes))
            .unwrap_or(DEFAULT_CHECK_INTERVAL_MINUTES),
        check_method,
        look_for_string: if check_method == CheckMethod::HEAD && entry.look_for_string.is_none() {
            String::new()
        } else {
            look_for_string
        },
        uptime_check_enabled: entry
            .uptime_check_enabled
            .or_else(|| current.map(|m| m.uptime_check_enabled))
            .unwrap_or(true),
        // `None` already means "keep the stored value" on update and "scheme
        // default" on create, so the entry's choice passes straight through.
        cert_check_enabled: entry.cert_check_enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_entry_and_leaves_absent_keys_unset() {
        let entries = parse_entries(r#"[{"url": "https://example.com"}]"#).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.com");
        assert!(entries[0].check_interval_minutes.is_none());
        assert!(entries[0].cert_check_enabled.is_none());
    }

    #[test]
    fn parses_every_legal_key() {
        let entries = parse_entries(
            r#"[{
                 "url": "https://example.com",
                 "uptime_check_enabled": false,
                 "check_interval_minutes": 15,
                 "check_method": "POST",
                 "look_for_string": "ok",
                 "cert_check_enabled": false
               }]"#,
        )
        .unwrap();
        let entry = &entries[0];
        assert_eq!(entry.uptime_check_enabled, Some(false));
        assert_eq!(entry.check_interval_minutes, Some(15));
        assert_eq!(entry.check_method, Some(CheckMethod::POST));
        assert_eq!(entry.look_for_string.as_deref(), Some("ok"));
        assert_eq!(entry.cert_check_enabled, Some(false));
    }

    #[test]
    fn rejects_unknown_keys_naming_key_and_entry() {
        let err = parse_entries(
            r#"[{"url": "https://a.test"}, {"url": "https://b.test", "uptime_status": "up"}]"#,
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.starts_with("entry 2:"), "{message}");
        assert!(message.contains("uptime_status"), "{message}");
    }

    #[test]
    fn rejects_a_missing_url_without_panicking() {
        let err = parse_entries(r#"[{"check_interval_minutes": 5}]"#).unwrap_err();
        assert!(err.to_string().contains("url"), "{err}");
    }

    #[test]
    fn rejects_a_non_array_document() {
        let err = parse_entries(r#"{"url": "https://example.com"}"#).unwrap_err();
        assert!(err.to_string().contains("JSON array"), "{err}");

        let err = parse_entries("not json at all").unwrap_err();
        assert!(err.to_string().contains("JSON array"), "{err}");
    }

    #[test]
    fn rejects_an_unknown_check_method() {
        let err = parse_entries(r#"[{"url": "https://a.test", "check_method": "PUT"}]"#).unwrap_err();
        assert!(err.to_string().starts_with("entry 1:"), "{err}");
    }

    #[test]
    fn errors_name_the_problem_without_a_serde_position_dump() {
        let err = parse_entries(r#"[{"url": "https://a.test", "check_method": "PUT"}]"#).unwrap_err();
        assert!(!err.to_string().contains("at line"), "{err}");

        let err = parse_entries("not json at all").unwrap_err();
        assert_eq!(
            err.to_string(),
            "the file must contain a JSON array of monitor objects"
        );
    }

    #[test]
    fn rejects_a_file_above_the_size_limit() {
        let dir = std::env::temp_dir().join("clockwerk-sync-size-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.json");
        std::fs::write(&path, vec![b' '; MAX_SYNC_FILE_BYTES as usize + 1]).unwrap();

        let err = read_entries(&path).unwrap_err();
        assert!(err.to_string().contains("import limit"), "{err}");

        std::fs::remove_file(&path).unwrap();
    }
}
