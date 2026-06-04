//! Kanban worker failure recording (Python `kanban_db._record_task_failure`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use tracing::{info, warn};

use crate::kanban::kanban_task_from_env;

const DEFAULT_FAILURE_LIMIT: i64 = 2;

/// Outcomes understood by the kanban DB (subset of Python).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanbanFailureOutcome {
    TimedOut,
    SpawnFailed,
    Crashed,
    GaveUp,
}

impl KanbanFailureOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::TimedOut => "timed_out",
            Self::SpawnFailed => "spawn_failed",
            Self::Crashed => "crashed",
            Self::GaveUp => "gave_up",
        }
    }
}

#[derive(Debug, Clone)]
pub struct KanbanFailureOptions<'a> {
    pub task_id: &'a str,
    pub error: &'a str,
    pub outcome: KanbanFailureOutcome,
    pub release_claim: bool,
    pub end_run: bool,
    pub failure_limit: Option<i64>,
    pub extra_payload: Option<serde_json::Value>,
}

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
    let path = hermes_config::hermes_home().join("kanban.db");
    path.exists().then_some(path)
}

fn dispatcher_failure_limit() -> i64 {
    std::env::var("HERMES_KANBAN_FAILURE_LIMIT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_FAILURE_LIMIT)
}

/// Record a worker failure (Python `_record_task_failure`).
pub fn record_task_failure(opts: KanbanFailureOptions<'_>) -> Result<bool, String> {
    let Some(db_path) = resolve_kanban_db_path() else {
        return Err("kanban.db not found".into());
    };
    record_task_failure_inner(&db_path, opts)
}

/// Record iteration-budget exhaustion for the active kanban worker task.
pub fn record_iteration_budget_exhausted(api_calls: u32, max_iterations: u32, error: &str) {
    let Some(task_id) = kanban_task_from_env() else {
        return;
    };
    let extra = serde_json::json!({
        "budget_used": api_calls,
        "budget_max": max_iterations,
    });
    let opts = KanbanFailureOptions {
        task_id: &task_id,
        error,
        outcome: KanbanFailureOutcome::TimedOut,
        release_claim: true,
        end_run: true,
        failure_limit: None,
        extra_payload: Some(extra),
    };
    match record_task_failure(opts) {
        Ok(blocked) => {
            info!(
                task_id = %task_id,
                blocked,
                api_calls,
                max_iterations,
                "recorded kanban iteration-budget failure"
            );
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "kanban failure: record_iteration_budget_exhausted failed");
        }
    }
}

fn record_task_failure_inner(
    db_path: &std::path::Path,
    opts: KanbanFailureOptions<'_>,
) -> Result<bool, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("open kanban db: {e}"))?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("begin txn: {e}"))?;
    let result = (|| -> Result<bool, String> {
        let row: (i64, Option<i64>) = conn
            .query_row(
                "SELECT consecutive_failures, max_retries FROM tasks WHERE id = ?1",
                params![opts.task_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("task lookup: {e}"))?;
        let failures = row.0 + 1;
        let (effective_limit, limit_source) = if let Some(limit) = opts.failure_limit {
            (limit.max(1), "dispatcher")
        } else if let Some(task_limit) = row.1 {
            (task_limit.max(1), "task")
        } else {
            (dispatcher_failure_limit().max(1), "dispatcher")
        };
        let err_snip: String = opts.error.chars().take(500).collect();
        let now = now_epoch();
        let blocked = failures >= effective_limit;
        let outcome = opts.outcome.as_str();

        if blocked {
            conn.execute(
                "UPDATE tasks SET status = 'blocked', claim_lock = NULL, claim_expires = NULL, \
                 worker_pid = NULL, consecutive_failures = ?1, last_failure_error = ?2 \
                 WHERE id = ?3 AND status IN ('running', 'ready')",
                params![failures, err_snip, opts.task_id],
            )
            .map_err(|e| format!("block task: {e}"))?;
        } else if opts.release_claim {
            conn.execute(
                "UPDATE tasks SET status = 'ready', claim_lock = NULL, claim_expires = NULL, \
                 worker_pid = NULL, consecutive_failures = ?1, last_failure_error = ?2 \
                 WHERE id = ?3 AND status = 'running'",
                params![failures, err_snip, opts.task_id],
            )
            .map_err(|e| format!("release claim: {e}"))?;
        }

        if opts.end_run {
            let run_id: Option<i64> = conn
                .query_row(
                    "SELECT current_run_id FROM tasks WHERE id = ?1",
                    params![opts.task_id],
                    |r| r.get(0),
                )
                .ok();
            if let Some(rid) = run_id {
                let run_outcome = if blocked { "gave_up" } else { outcome };
                let mut meta = serde_json::json!({
                    "failures": failures,
                    "effective_limit": effective_limit,
                    "limit_source": limit_source,
                    "trigger_outcome": outcome,
                });
                if let Some(extra) = &opts.extra_payload {
                    if let Some(obj) = meta.as_object_mut() {
                        if let Some(extra_obj) = extra.as_object() {
                            for (k, v) in extra_obj {
                                obj.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
                conn.execute(
                    "UPDATE task_runs SET status = ?1, outcome = ?2, error = ?3, \
                     metadata = ?4, ended_at = ?5, claim_lock = NULL, claim_expires = NULL, \
                     worker_pid = NULL WHERE id = ?6 AND ended_at IS NULL",
                    params![
                        run_outcome,
                        run_outcome,
                        err_snip,
                        meta.to_string(),
                        now,
                        rid
                    ],
                )
                .map_err(|e| format!("end run: {e}"))?;
                conn.execute(
                    "UPDATE tasks SET current_run_id = NULL WHERE id = ?1",
                    params![opts.task_id],
                )
                .map_err(|e| format!("clear current_run_id: {e}"))?;
                let event_kind = if blocked { "gave_up" } else { outcome };
                let mut payload = serde_json::json!({
                    "failures": failures,
                    "effective_limit": effective_limit,
                    "limit_source": limit_source,
                    "error": err_snip,
                    "trigger_outcome": outcome,
                });
                if let Some(extra) = &opts.extra_payload {
                    if let Some(obj) = payload.as_object_mut() {
                        if let Some(extra_obj) = extra.as_object() {
                            for (k, v) in extra_obj {
                                obj.insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
                conn.execute(
                    "INSERT INTO task_events (task_id, run_id, kind, payload, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![opts.task_id, rid, event_kind, payload.to_string(), now],
                )
                .map_err(|e| format!("task event: {e}"))?;
            }
        }
        Ok(blocked)
    })();
    match result {
        Ok(blocked) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| format!("commit: {e}"))?;
            Ok(blocked)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("kanban.db");
        let conn = Connection::open(&path).expect("open");
        conn.execute_batch(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL DEFAULT 'ready',
                consecutive_failures INTEGER NOT NULL DEFAULT 0,
                max_retries INTEGER,
                last_failure_error TEXT,
                claim_lock TEXT,
                claim_expires INTEGER,
                worker_pid INTEGER,
                current_run_id INTEGER
            );
            CREATE TABLE task_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                status TEXT,
                outcome TEXT,
                error TEXT,
                metadata TEXT,
                ended_at INTEGER,
                claim_lock TEXT,
                claim_expires INTEGER,
                worker_pid INTEGER
            );
            CREATE TABLE task_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT,
                run_id INTEGER,
                kind TEXT,
                payload TEXT,
                created_at INTEGER
            );",
        )
        .expect("schema");
        conn.execute(
            "INSERT INTO tasks (id, status, consecutive_failures, max_retries, current_run_id)
             VALUES ('t1', 'running', 0, 1, 1)",
            [],
        )
        .expect("task");
        conn.execute(
            "INSERT INTO task_runs (id, status) VALUES (1, 'running')",
            [],
        )
        .expect("run");
        (tmp, path)
    }

    #[test]
    fn per_task_limit_blocks_on_first_failure() {
        let (_tmp, path) = setup_db();
        let blocked = record_task_failure_inner(
            &path,
            KanbanFailureOptions {
                task_id: "t1",
                error: "spawn fail",
                outcome: KanbanFailureOutcome::SpawnFailed,
                release_claim: true,
                end_run: false,
                failure_limit: Some(10),
                extra_payload: None,
            },
        )
        .expect("record");
        assert!(blocked);
    }
}
