//! Kanban worker failure recording (Python `kanban_db._record_task_failure` budget path).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use tracing::{info, warn};

use crate::kanban::kanban_task_from_env;

const DEFAULT_FAILURE_LIMIT: i64 = 2;

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn resolve_kanban_db_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HERMES_KANBAN_DB") {
        let t = p.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t));
        }
    }
    if let Ok(root) = std::env::var("HERMES_KANBAN_ROOT") {
        let t = root.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t).join("kanban.db"));
        }
    }
    hermes_config::hermes_home()
        .join("kanban.db")
        .exists()
        .then(|| hermes_config::hermes_home().join("kanban.db"))
}

/// Record iteration-budget exhaustion for the active kanban worker task.
///
/// Mirrors Python `conversation_loop` finalize path:
/// `_record_task_failure(..., outcome="timed_out", release_claim=True, end_run=True)`.
pub fn record_iteration_budget_exhausted(api_calls: u32, max_iterations: u32, error: &str) {
    let Some(task_id) = kanban_task_from_env() else {
        return;
    };
    let Some(db_path) = resolve_kanban_db_path() else {
        warn!(task_id = %task_id, "kanban failure: no kanban.db found");
        return;
    };
    if let Err(e) = record_iteration_budget_exhausted_inner(
        &db_path,
        &task_id,
        error,
        api_calls,
        max_iterations,
    ) {
        warn!(task_id = %task_id, error = %e, "kanban failure: record_iteration_budget_exhausted failed");
    }
}

fn record_iteration_budget_exhausted_inner(
    db_path: &std::path::Path,
    task_id: &str,
    error: &str,
    api_calls: u32,
    max_iterations: u32,
) -> Result<(), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("open kanban db: {e}"))?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("begin txn: {e}"))?;
    let result = (|| -> Result<bool, String> {
        let row: (i64, Option<i64>) = conn
            .query_row(
                "SELECT consecutive_failures, max_retries FROM tasks WHERE id = ?1",
                params![task_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("task lookup: {e}"))?;
        let failures = row.0 + 1;
        let effective_limit = row.1.unwrap_or(DEFAULT_FAILURE_LIMIT).max(1);
        let err_snip: String = error.chars().take(500).collect();
        let now = now_epoch();
        let blocked = failures >= effective_limit;
        if blocked {
            conn.execute(
                "UPDATE tasks SET status = 'blocked', claim_lock = NULL, claim_expires = NULL, \
                 worker_pid = NULL, consecutive_failures = ?1, last_failure_error = ?2 \
                 WHERE id = ?3 AND status IN ('running', 'ready')",
                params![failures, err_snip, task_id],
            )
            .map_err(|e| format!("block task: {e}"))?;
            let run_id: Option<i64> = conn
                .query_row(
                    "SELECT current_run_id FROM tasks WHERE id = ?1",
                    params![task_id],
                    |r| r.get(0),
                )
                .ok();
            if let Some(rid) = run_id {
                let meta = serde_json::json!({
                    "failures": failures,
                    "trigger_outcome": "timed_out",
                    "effective_limit": effective_limit,
                    "limit_source": "dispatcher",
                    "budget_used": api_calls,
                    "budget_max": max_iterations,
                });
                conn.execute(
                    "UPDATE task_runs SET status = 'gave_up', outcome = 'gave_up', error = ?1, \
                     metadata = ?2, ended_at = ?3, claim_lock = NULL, claim_expires = NULL, \
                     worker_pid = NULL WHERE id = ?4 AND ended_at IS NULL",
                    params![
                        err_snip,
                        meta.to_string(),
                        now,
                        rid
                    ],
                )
                .map_err(|e| format!("end run: {e}"))?;
                conn.execute(
                    "UPDATE tasks SET current_run_id = NULL WHERE id = ?1",
                    params![task_id],
                )
                .map_err(|e| format!("clear current_run_id: {e}"))?;
                let payload = serde_json::json!({
                    "failures": failures,
                    "effective_limit": effective_limit,
                    "limit_source": "dispatcher",
                    "error": err_snip,
                    "trigger_outcome": "timed_out",
                    "budget_used": api_calls,
                    "budget_max": max_iterations,
                });
                conn.execute(
                    "INSERT INTO task_events (task_id, run_id, kind, payload, created_at) \
                     VALUES (?1, ?2, 'gave_up', ?3, ?4)",
                    params![task_id, rid, payload.to_string(), now],
                )
                .map_err(|e| format!("gave_up event: {e}"))?;
            }
        } else {
            conn.execute(
                "UPDATE tasks SET status = 'ready', claim_lock = NULL, claim_expires = NULL, \
                 worker_pid = NULL, consecutive_failures = ?1, last_failure_error = ?2 \
                 WHERE id = ?3 AND status = 'running'",
                params![failures, err_snip, task_id],
            )
            .map_err(|e| format!("release claim: {e}"))?;
            let run_id: Option<i64> = conn
                .query_row(
                    "SELECT current_run_id FROM tasks WHERE id = ?1",
                    params![task_id],
                    |r| r.get(0),
                )
                .ok();
            if let Some(rid) = run_id {
                let meta = serde_json::json!({ "failures": failures });
                conn.execute(
                    "UPDATE task_runs SET status = 'timed_out', outcome = 'timed_out', error = ?1, \
                     metadata = ?2, ended_at = ?3, claim_lock = NULL, claim_expires = NULL, \
                     worker_pid = NULL WHERE id = ?4 AND ended_at IS NULL",
                    params![err_snip, meta.to_string(), now, rid],
                )
                .map_err(|e| format!("end run timed_out: {e}"))?;
                conn.execute(
                    "UPDATE tasks SET current_run_id = NULL WHERE id = ?1",
                    params![task_id],
                )
                .map_err(|e| format!("clear current_run_id: {e}"))?;
                let payload = serde_json::json!({
                    "error": err_snip,
                    "failures": failures,
                    "budget_used": api_calls,
                    "budget_max": max_iterations,
                });
                conn.execute(
                    "INSERT INTO task_events (task_id, run_id, kind, payload, created_at) \
                     VALUES (?1, ?2, 'timed_out', ?3, ?4)",
                    params![task_id, rid, payload.to_string(), now],
                )
                .map_err(|e| format!("timed_out event: {e}"))?;
            }
        }
        Ok(blocked)
    })();
    match result {
        Ok(blocked) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("commit: {e}"))?;
            info!(
                task_id = %task_id,
                blocked,
                api_calls,
                max_iterations,
                "recorded kanban iteration-budget failure"
            );
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
