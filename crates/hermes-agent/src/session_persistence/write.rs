//! Write transactions with BEGIN IMMEDIATE and jitter retry (Python `SessionDB._execute_write`).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use hermes_core::AgentError;
use rand::RngExt;
use rusqlite::Connection;

const WRITE_MAX_RETRIES: usize = 15;
const WRITE_RETRY_MIN_MS: u64 = 20;
const WRITE_RETRY_MAX_MS: u64 = 150;
pub const CHECKPOINT_EVERY_N_WRITES: u64 = 50;

/// Execute a write inside BEGIN IMMEDIATE with lock-contention retry.
pub fn execute_write<F, T>(conn: &Arc<Mutex<Connection>>, mut op: F) -> Result<T, AgentError>
where
    F: FnMut(&Connection) -> Result<T, AgentError>,
{
    let mut last_err: Option<AgentError> = None;
    for attempt in 0..WRITE_MAX_RETRIES {
        let result = {
            let guard = conn
                .lock()
                .map_err(|_| AgentError::Io("state db lock poisoned".into()))?;
            guard
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(|e| AgentError::Io(format!("BEGIN IMMEDIATE failed: {e}")))?;
            match op(&guard) {
                Ok(value) => {
                    if let Err(e) = guard.execute_batch("COMMIT") {
                        let _ = guard.execute_batch("ROLLBACK");
                        Err(AgentError::Io(format!("COMMIT failed: {e}")))
                    } else {
                        Ok(value)
                    }
                }
                Err(e) => {
                    let _ = guard.execute_batch("ROLLBACK");
                    Err(e)
                }
            }
        };

        match result {
            Ok(value) => return Ok(value),
            Err(e) => {
                let retryable = e
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("locked") || e.to_string().to_ascii_lowercase().contains("busy");
                if retryable && attempt + 1 < WRITE_MAX_RETRIES {
                    last_err = Some(e);
                    let jitter = rand::rng().random_range(WRITE_RETRY_MIN_MS..=WRITE_RETRY_MAX_MS);
                    thread::sleep(Duration::from_millis(jitter));
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        AgentError::Io("database is locked after max retries".into())
    }))
}

/// Best-effort PASSIVE WAL checkpoint; never raises.
pub fn try_wal_checkpoint(conn: &Arc<Mutex<Connection>>) {
    let Ok(guard) = conn.lock() else {
        return;
    };
    if let Ok(row) = guard.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
        Ok((r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    }) {
        if row.0 > 0 {
            tracing::debug!(
                "WAL checkpoint: {}/{} pages checkpointed",
                row.1,
                row.0
            );
        }
    }
}
