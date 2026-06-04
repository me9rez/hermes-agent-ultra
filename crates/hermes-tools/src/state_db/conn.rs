//! state.db connection helper for read-side tools and gateway hooks.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;

use super::error::StateDbError;

/// Shared handle to an open `state.db` (schema owned by `hermes-agent` init).
#[derive(Clone)]
pub struct StateDb {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

impl StateDb {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StateDbError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StateDbError(format!("create state db dir: {e}")))?;
        }
        let conn = Connection::open(&path).map_err(|e| StateDbError(format!("open state db: {e}")))?;
        conn.busy_timeout(Duration::from_secs(1))
            .map_err(|e| StateDbError(format!("busy_timeout: {e}")))?;
        let initialized: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions' LIMIT 1",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if !initialized {
            return Err(StateDbError(
                "state.db schema not initialized — start hermes CLI or gateway once".into(),
            ));
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    pub fn open_default() -> Result<Self, StateDbError> {
        Self::open(hermes_config::state_db_path())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }
}
