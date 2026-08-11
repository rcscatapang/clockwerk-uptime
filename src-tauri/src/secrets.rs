//! Keychain-backed secret storage.
//!
//! The Slack webhook URL is the app's only secret. It lives in the macOS
//! Keychain (service `tauri-uptime-monitor`), never in SQLite, is never
//! logged, and is never returned to the frontend — commands expose only a
//! configured/not-configured flag.

use keyring::Entry;

use crate::error::AppError;

const SERVICE: &str = "tauri-uptime-monitor";
const SLACK_WEBHOOK_ENTRY: &str = "slack_webhook_url";

fn entry() -> Result<Entry, AppError> {
    Entry::new(SERVICE, SLACK_WEBHOOK_ENTRY)
        .map_err(|e| AppError::Internal(format!("keychain unavailable: {e}")))
}

pub fn get_slack_webhook() -> Result<Option<String>, AppError> {
    match entry()?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Internal(format!("keychain read failed: {e}"))),
    }
}

pub fn set_slack_webhook(url: &str) -> Result<(), AppError> {
    entry()?
        .set_password(url)
        .map_err(|e| AppError::Internal(format!("keychain write failed: {e}")))
}

pub fn delete_slack_webhook() -> Result<(), AppError> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Internal(format!("keychain delete failed: {e}"))),
    }
}
