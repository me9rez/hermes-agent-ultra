//! Shared state.db error type for read-side helpers.

use std::fmt;

#[derive(Debug, Clone)]
pub struct StateDbError(pub String);

impl fmt::Display for StateDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StateDbError {}

impl From<rusqlite::Error> for StateDbError {
    fn from(e: rusqlite::Error) -> Self {
        Self(e.to_string())
    }
}

impl From<std::sync::PoisonError<()>> for StateDbError {
    fn from(_: std::sync::PoisonError<()>) -> Self {
        Self("state db lock poisoned".into())
    }
}
