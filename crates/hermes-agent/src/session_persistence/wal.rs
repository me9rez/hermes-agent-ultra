//! WAL journal mode with NFS/SMB fallback (Python `hermes_state.apply_wal_with_fallback`).

use std::sync::{Mutex, OnceLock};

use rusqlite::Connection;
use tracing::warn;

const WAL_INCOMPAT_MARKERS: &[&str] = &["locking protocol", "not authorized"];

static WAL_FALLBACK_WARNED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static LAST_INIT_ERROR: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn warned_paths() -> &'static Mutex<Vec<String>> {
    WAL_FALLBACK_WARNED.get_or_init(|| Mutex::new(Vec::new()))
}

fn last_init_error_lock() -> &'static Mutex<Option<String>> {
    LAST_INIT_ERROR.get_or_init(|| Mutex::new(None))
}

/// Record (or clear) the most recent state.db init failure.
pub fn set_last_init_error(msg: Option<String>) {
    if let Ok(mut guard) = last_init_error_lock().lock() {
        *guard = msg;
    }
}

/// Return the most recent state.db init failure, if any.
pub fn get_last_init_error() -> Option<String> {
    last_init_error_lock().lock().ok()?.clone()
}

/// User-facing message when the session DB is unavailable (Python parity).
pub fn format_session_db_unavailable(prefix: &str) -> String {
    let cause = get_last_init_error();
    let Some(cause) = cause else {
        return format!("{prefix}.");
    };
    let hint = if WAL_INCOMPAT_MARKERS
        .iter()
        .any(|m| cause.to_ascii_lowercase().contains(m))
    {
        " (state.db may be on NFS/SMB/FUSE — see https://www.sqlite.org/wal.html)"
    } else {
        ""
    };
    format!("{prefix}: {cause}{hint}.")
}

fn on_disk_journal_mode(conn: &Connection) -> Option<String> {
    conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .ok()
        .map(|m| m.trim().to_ascii_lowercase())
}

fn log_wal_fallback_once(db_label: &str, exc: &str) {
    let Ok(mut warned) = warned_paths().lock() else {
        return;
    };
    if warned.iter().any(|p| p == db_label) {
        return;
    }
    warned.push(db_label.to_string());
    warn!(
        db = db_label,
        error = exc,
        "WAL journal_mode unsupported on this filesystem — falling back to journal_mode=DELETE \
         (slower rollback-journal mode; reduces concurrency but works on NFS/SMB/FUSE). \
         See https://www.sqlite.org/wal.html"
    );
}

/// Set `journal_mode=WAL`, falling back to DELETE on NFS/SMB incompatibility.
pub fn apply_wal_with_fallback(conn: &Connection, db_label: &str) -> Result<String, rusqlite::Error> {
    if on_disk_journal_mode(conn).as_deref() == Some("wal") {
        return Ok("wal".into());
    }

    match conn.execute_batch("PRAGMA journal_mode=WAL") {
        Ok(()) => Ok("wal".into()),
        Err(exc) => {
            let msg = exc.to_string().to_ascii_lowercase();
            if !WAL_INCOMPAT_MARKERS.iter().any(|m| msg.contains(m)) {
                return Err(exc);
            }
            if on_disk_journal_mode(conn).as_deref() == Some("wal") {
                return Err(exc);
            }
            log_wal_fallback_once(db_label, &exc.to_string());
            conn.execute_batch("PRAGMA journal_mode=DELETE")?;
            Ok("delete".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_unavailable_includes_cause() {
        set_last_init_error(Some("locking protocol".into()));
        let msg = format_session_db_unavailable("Session database not available");
        assert!(msg.contains("locking protocol"));
        assert!(msg.contains("NFS"));
        set_last_init_error(None);
    }
}
