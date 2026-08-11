//! App-wide error type.
//!
//! Every Tauri command returns `Result<T, AppError>`. The frontend receives a
//! stable `{ code, message }` shape: `code` is the variant name (e.g.
//! `"DuplicateUrl"`) and is the contract the UI matches on; `message` is a
//! human-readable fallback. Commands never panic on user input.

use serde::ser::SerializeStruct;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    InvalidUrl(String),
    #[error("a monitor for this URL already exists")]
    DuplicateUrl,
    #[error("{0}")]
    InvalidInput(String),
    #[error("monitor not found")]
    NotFound,
    #[error("database error: {0}")]
    Db(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    /// Stable machine-readable code, matched by the frontend (`src/lib/tauri.ts`).
    pub fn code(&self) -> &'static str {
        match self {
            AppError::InvalidUrl(_) => "InvalidUrl",
            AppError::DuplicateUrl => "DuplicateUrl",
            AppError::InvalidInput(_) => "InvalidInput",
            AppError::NotFound => "NotFound",
            AppError::Db(_) => "Db",
            AppError::Internal(_) => "Internal",
        }
    }
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        match &err {
            // UNIQUE violation → typed duplicate error the UI can message.
            // monitors.url is the only application-level UNIQUE column, so the
            // extended result code alone identifies it (no message parsing).
            rusqlite::Error::SqliteFailure(e, _)
                if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                AppError::DuplicateUrl
            }
            rusqlite::Error::QueryReturnedNoRows => AppError::NotFound,
            _ => AppError::Db(err.to_string()),
        }
    }
}
