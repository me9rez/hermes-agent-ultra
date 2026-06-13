//! Background job management commands.
//!
//! Provides `/background`, `/mission`, `/queue`, `/clear-queue` slash commands
//! and all supporting infrastructure (queuing, scheduling, termination, recovery).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use hermes_core::AgentError;

use crate::alpha_runtime::{
    enqueue_loop_event, ensure_alpha_runtime_bootstrap, ensure_trading_runtime_bootstrap,
    load_alpha_loops, load_last_trading_alpha_report, recover_orphan_loop_events,
    refresh_trading_alpha_report, render_mission_board, render_trading_alpha_board,
    replay_loop_queue,
};
use crate::commands::{CommandResult, emit_command_output};

// ---------------------------------------------------------------------------
// Job counting & record
// ---------------------------------------------------------------------------

/// Returns (queued, running, completed, failed) counts.
pub(crate) fn background_job_counts() -> (usize, usize, usize, usize) {
    let jobs_dir = hermes_config::hermes_home().join("background_jobs");
    let mut queued = 0usize;
    let mut running = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;
    let Ok(entries) = std::fs::read_dir(jobs_dir) else {
        return (queued, running, completed, failed);
    };
    for entry in entries.filter_map(Result::ok) {
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let status = read_json_map(&entry.path())
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        match status.as_str() {
            "queued" => queued = queued.saturating_add(1),
            "running" => running = running.saturating_add(1),
            "completed" => completed = completed.saturating_add(1),
            "failed" => failed = failed.saturating_add(1),
            _ => {}
        }
    }
    (queued, running, completed, failed)
}

#[derive(Debug, Clone)]
struct BackgroundJobRecord {
    id: String,
    status: String,
    task: String,
    pid: Option<u32>,
    attempts: u64,
    created_at: String,
    started_at: String,
    finished_at: String,
    log_path: PathBuf,
    status_path: PathBuf,
}

// ---------------------------------------------------------------------------
// Collecting / resolving
// ---------------------------------------------------------------------------

fn collect_background_jobs(limit: usize) -> Vec<BackgroundJobRecord> {
    let jobs_dir = hermes_config::hermes_home().join("background_jobs");
    let Ok(entries) = std::fs::read_dir(jobs_dir) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let status_path = entry.path();
        if status_path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let map = read_json_map(&status_path);
        if map.is_empty() {
            continue;
        }
        let id = map
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToString::to_string)
            .or_else(|| {
                status_path
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| "unknown".to_string());
        let status = map
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let task = map
            .get("task")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let pid = map
            .get("pid")
            .and_then(|v| v.as_u64())
            .and_then(|raw| u32::try_from(raw).ok());
        let attempts = map.get("attempts").and_then(|v| v.as_u64()).unwrap_or(0);
        let created_at = map
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let started_at = map
            .get("started_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let finished_at = map
            .get("finished_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let log_path = map
            .get("log_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| status_path.with_extension("log"));
        rows.push(BackgroundJobRecord {
            id,
            status,
            task,
            pid,
            attempts,
            created_at,
            started_at,
            finished_at,
            log_path,
            status_path,
        });
    }
    rows.sort_by(|a, b| b.id.cmp(&a.id));
    rows.truncate(limit.max(1));
    rows
}

fn resolve_background_job(id_or_prefix: &str) -> Option<BackgroundJobRecord> {
    let needle = id_or_prefix.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return None;
    }
    collect_background_jobs(500).into_iter().find(|job| {
        let id = job.id.to_ascii_lowercase();
        id == needle || id.starts_with(&needle)
    })
}

// ---------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------

pub(crate) fn tail_text_lines(text: &str, limit: usize) -> String {
    let rows: Vec<&str> = text.lines().collect();
    let cap = limit.max(1);
    let start = rows.len().saturating_sub(cap);
    rows[start..].join("\n")
}

fn tail_file_lines(path: &Path, limit: usize) -> Result<String, AgentError> {
    let body = std::fs::read_to_string(path).map_err(|e| {
        AgentError::Io(format!(
            "Failed to read background log {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(tail_text_lines(&body, limit))
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn read_json_map(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn write_json_map(
    path: &Path,
    map: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), std::io::Error> {
    let content = serde_json::to_string_pretty(&serde_json::Value::Object(map.clone()))
        .unwrap_or_else(|_| "{}".to_string());
    std::fs::write(path, content)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub(crate) fn render_background_status(limit: usize) -> String {
    let (queued, running, completed, failed) = background_job_counts();
    let rows = collect_background_jobs(limit);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Background queue status: queued={} running={} completed={} failed={}",
        queued, running, completed, failed
    );
    if rows.is_empty() {
        out.push_str("\nNo background jobs found.");
        return out;
    }
    out.push_str("\nRecent background jobs:\n");
    for (idx, row) in rows.iter().enumerate() {
        let pid_suffix = row.pid.map(|pid| format!(" pid={pid}")).unwrap_or_default();
        let _ = writeln!(
            out,
            "{}. {} [{}{}] attempts={} task={}",
            idx + 1,
            row.id,
            row.status,
            pid_suffix,
            row.attempts,
            super::truncate_chars(row.task.trim(), 84)
        );
    }
    out.push_str("\nUse `/background tail <job-id> [N]` to inspect logs.");
    out
}

// ---------------------------------------------------------------------------
// Process helpers (platform-specific)
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn process_running(pid: u32) -> bool {
    // SAFETY: libc::kill with signal 0 only performs existence/permission check.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        true
    } else {
        matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        )
    }
}

#[cfg(not(unix))]
fn process_running(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn terminate_pid(pid: u32) -> std::io::Result<()> {
    // SAFETY: pid is sourced from our own status record; SIGTERM is best-effort.
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn terminate_pid(_pid: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Process termination is unsupported on this platform.",
    ))
}

// ---------------------------------------------------------------------------
// Job lifecycle
// ---------------------------------------------------------------------------

fn terminate_background_job(id_or_prefix: &str) -> Result<String, AgentError> {
    let Some(job) = resolve_background_job(id_or_prefix) else {
        return Ok(format!(
            "Background job '{}' not found. Use `/background status`.",
            id_or_prefix
        ));
    };
    let mut map = read_json_map(&job.status_path);
    if map.is_empty() {
        return Err(AgentError::Io(format!(
            "Status file missing or unreadable: {}",
            job.status_path.display()
        )));
    }
    let status = map
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_ascii_lowercase();
    if status == "completed" || status == "failed" || status == "canceled" {
        return Ok(format!(
            "Background job {} already {}.\nStatus file: {}",
            job.id,
            status,
            job.status_path.display()
        ));
    }

    let mut termination_note = String::new();
    if let Some(pid) = map
        .get("pid")
        .and_then(|v| v.as_u64())
        .and_then(|raw| u32::try_from(raw).ok())
    {
        if process_running(pid) {
            match terminate_pid(pid) {
                Ok(()) => termination_note = format!("Sent SIGTERM to pid {}.", pid),
                Err(err) => termination_note = format!("Failed to terminate pid {}: {}.", pid, err),
            }
        } else {
            termination_note = format!("Pid {} was not running.", pid);
        }
    }

    map.insert(
        "status".into(),
        serde_json::Value::String("canceled".into()),
    );
    map.insert(
        "finished_at".into(),
        serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
    );
    map.insert(
        "error".into(),
        serde_json::Value::String("canceled by operator".into()),
    );
    map.insert("pid".into(), serde_json::Value::Null);
    write_json_map(&job.status_path, &map)
        .map_err(|e| AgentError::Io(format!("Failed to update background status: {}", e)))?;

    Ok(format!(
        "Canceled background job {}\nStatus file: {}\n{}",
        job.id,
        job.status_path.display(),
        if termination_note.is_empty() {
            "No active child pid recorded.".to_string()
        } else {
            termination_note
        }
    ))
}

fn claim_queued_background_job(
    status_path: &Path,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, AgentError> {
    let mut queued = read_json_map(status_path);
    if queued.is_empty() {
        return Ok(None);
    }
    let status = queued
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("queued")
        .to_ascii_lowercase();
    if status != "queued" {
        return Ok(None);
    }
    let started = chrono::Utc::now().to_rfc3339();
    let attempts = queued
        .get("attempts")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .saturating_add(1);
    queued.insert(
        "status".to_string(),
        serde_json::Value::String("running".into()),
    );
    queued.insert("started_at".to_string(), serde_json::Value::String(started));
    queued.insert("attempts".to_string(), serde_json::json!(attempts));
    write_json_map(status_path, &queued)
        .map_err(|e| AgentError::Io(format!("Failed to claim background job: {}", e)))?;
    Ok(Some(queued))
}

fn schedule_background_job_execution(status_path: PathBuf, log_path: PathBuf, task: String) {
    tokio::spawn(async move {
        let queued = match claim_queued_background_job(&status_path) {
            Ok(Some(claimed)) => claimed,
            Ok(None) => return,
            Err(_) => return,
        };
        let started = queued
            .get("started_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                let mut failed = queued.clone();
                failed.insert("status".into(), serde_json::Value::String("failed".into()));
                failed.insert(
                    "finished_at".into(),
                    serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
                );
                failed.insert(
                    "error".into(),
                    serde_json::Value::String(format!("current_exe: {}", e)),
                );
                let _ = write_json_map(&status_path, &failed);
                return;
            }
        };

        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg("chat")
            .arg("--query")
            .arg(task)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Ensure detached children do not survive runtime/session teardown.
        cmd.kill_on_drop(true);

        if let Ok(home) = std::env::var("HERMES_HOME") {
            cmd.env("HERMES_HOME", home);
        }

        let child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let mut failed = queued.clone();
                failed.insert("status".into(), serde_json::Value::String("failed".into()));
                failed.insert(
                    "finished_at".into(),
                    serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
                );
                failed.insert(
                    "error".into(),
                    serde_json::Value::String(format!("spawn failed: {}", e)),
                );
                failed.insert("pid".into(), serde_json::Value::Null);
                let _ = write_json_map(&status_path, &failed);
                return;
            }
        };
        if let Some(pid) = child.id() {
            let mut running = queued.clone();
            running.insert("pid".into(), serde_json::json!(pid));
            let _ = write_json_map(&status_path, &running);
        }

        let out = child.wait_with_output().await;
        match out {
            Ok(output) => {
                let exit = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let log = format!(
                    "task: {}\nstarted_at: {}\nfinished_at: {}\nexit_code: {}\n\n[stdout]\n{}\n\n[stderr]\n{}\n",
                    queued.get("task").and_then(|v| v.as_str()).unwrap_or(""),
                    started,
                    chrono::Utc::now().to_rfc3339(),
                    exit,
                    stdout,
                    stderr
                );
                let _ = std::fs::write(&log_path, log);

                let mut done = queued.clone();
                done.insert(
                    "status".into(),
                    serde_json::Value::String(if output.status.success() {
                        "completed".into()
                    } else {
                        "failed".into()
                    }),
                );
                done.insert(
                    "finished_at".into(),
                    serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
                );
                done.insert("exit_code".into(), serde_json::json!(exit));
                done.insert("pid".into(), serde_json::Value::Null);
                let _ = write_json_map(&status_path, &done);
            }
            Err(e) => {
                let mut failed = queued.clone();
                failed.insert("status".into(), serde_json::Value::String("failed".into()));
                failed.insert(
                    "finished_at".into(),
                    serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
                );
                failed.insert(
                    "error".into(),
                    serde_json::Value::String(format!("spawn/output failed: {}", e)),
                );
                failed.insert("pid".into(), serde_json::Value::Null);
                let _ = write_json_map(&status_path, &failed);
            }
        }
    });
}

/// Recover queued background jobs on startup (exported for the runtime).
pub fn recover_queued_background_jobs(max_jobs: usize) -> usize {
    let jobs_dir = hermes_config::hermes_home().join("background_jobs");
    let Ok(entries) = std::fs::read_dir(&jobs_dir) else {
        return 0;
    };
    let mut recovered = 0usize;
    for entry in entries.filter_map(Result::ok) {
        if recovered >= max_jobs.max(1) {
            break;
        }
        let status_path = entry.path();
        if status_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            != "json"
        {
            continue;
        }
        let map = read_json_map(&status_path);
        let status = map
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if status != "queued" {
            continue;
        }
        let task = map
            .get("task")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let log_path = map
            .get("log_path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| status_path.with_extension("log"));
        if let Some(task) = task {
            schedule_background_job_execution(status_path.clone(), log_path, task);
            recovered = recovered.saturating_add(1);
        }
    }
    recovered
}

// ---------------------------------------------------------------------------
// Public API: QueuedBackgroundJob
// ---------------------------------------------------------------------------

pub(crate) struct QueuedBackgroundJob {
    pub(crate) id: String,
    pub(crate) task: String,
    pub(crate) status_path: PathBuf,
    pub(crate) log_path: PathBuf,
}

pub(crate) fn queue_background_job(task: &str) -> Result<QueuedBackgroundJob, AgentError> {
    let task = task.trim();
    if task.is_empty() {
        return Err(AgentError::Config(
            "Background task cannot be empty.".to_string(),
        ));
    }
    let job_id = format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        uuid::Uuid::new_v4().simple()
    );
    let jobs_dir = hermes_config::hermes_home().join("background_jobs");
    std::fs::create_dir_all(&jobs_dir).map_err(|e| {
        AgentError::Io(format!(
            "Failed to create background job directory {}: {}",
            jobs_dir.display(),
            e
        ))
    })?;
    let status_path = jobs_dir.join(format!("{}.json", job_id));
    let log_path = jobs_dir.join(format!("{}.log", job_id));

    let status = serde_json::json!({
        "id": job_id,
        "task": task,
        "status": "queued",
        "attempts": 0,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "started_at": serde_json::Value::Null,
        "finished_at": serde_json::Value::Null,
        "exit_code": serde_json::Value::Null,
        "log_path": log_path,
    });
    std::fs::write(
        &status_path,
        serde_json::to_string_pretty(&status).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(|e| AgentError::Io(format!("Failed to write background status: {}", e)))?;

    schedule_background_job_execution(status_path.clone(), log_path.clone(), task.to_string());
    Ok(QueuedBackgroundJob {
        id: status["id"].as_str().unwrap_or("unknown").to_string(),
        task: task.to_string(),
        status_path,
        log_path,
    })
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

pub(crate) fn handle_background_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        emit_command_output(
            host,
            "Usage: /background <message>\n\
             - /background status|list\n\
             - /background tail <job-id> [N]\n\
             - /background stop <job-id>\n\
             - /background event <source> <payload>\n\
             Queues a task to run in the background while you continue chatting.",
        );
        return Ok(CommandResult::Handled);
    }
    let sub = args[0].trim().to_ascii_lowercase();
    if sub == "status" || sub == "list" {
        emit_command_output(host, render_background_status(12));
        return Ok(CommandResult::Handled);
    }
    if sub == "tail" || sub == "log" || sub == "logs" || sub == "show" {
        let limit = args
            .get(2)
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .unwrap_or(80)
            .clamp(5, 800);
        let requested_id = args
            .get(1)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                collect_background_jobs(1)
                    .into_iter()
                    .next()
                    .map(|row| row.id)
            });
        let Some(id_or_prefix) = requested_id else {
            emit_command_output(
                host,
                "Usage: /background tail <job-id> [N]\nNo jobs available yet.",
            );
            return Ok(CommandResult::Handled);
        };
        let Some(job) = resolve_background_job(&id_or_prefix) else {
            emit_command_output(
                host,
                format!(
                    "Background job '{}' not found. Use `/background status`.",
                    id_or_prefix
                ),
            );
            return Ok(CommandResult::Handled);
        };
        let tail = if job.log_path.exists() {
            tail_file_lines(&job.log_path, limit)?
        } else {
            "(log file does not exist yet)".to_string()
        };
        emit_command_output(
            host,
            format!(
                "Background job\nid: {}\nstatus: {}\nattempts: {}\ncreated_at: {}\nstarted_at: {}\nfinished_at: {}\nstatus_file: {}\nlog_file: {}\n\n--- log tail ({}) ---\n{}",
                job.id,
                job.status,
                job.attempts,
                if job.created_at.is_empty() {
                    "(n/a)"
                } else {
                    job.created_at.as_str()
                },
                if job.started_at.is_empty() {
                    "(n/a)"
                } else {
                    job.started_at.as_str()
                },
                if job.finished_at.is_empty() {
                    "(n/a)"
                } else {
                    job.finished_at.as_str()
                },
                job.status_path.display(),
                job.log_path.display(),
                limit,
                if tail.trim().is_empty() {
                    "(empty)"
                } else {
                    tail.trim_end()
                }
            ),
        );
        return Ok(CommandResult::Handled);
    }
    if sub == "stop" || sub == "cancel" || sub == "kill" {
        let requested_id = args
            .get(1)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                collect_background_jobs(200).into_iter().find_map(|job| {
                    if matches!(job.status.as_str(), "running" | "queued") {
                        Some(job.id)
                    } else {
                        None
                    }
                })
            });
        let Some(id_or_prefix) = requested_id else {
            emit_command_output(
                host,
                "Usage: /background stop <job-id>\nNo running/queued jobs found.",
            );
            return Ok(CommandResult::Handled);
        };
        emit_command_output(host, terminate_background_job(&id_or_prefix)?);
        return Ok(CommandResult::Handled);
    }
    if sub == "event" {
        let Some(source) = args.get(1).copied() else {
            emit_command_output(host, "Usage: /background event <source> <payload>");
            return Ok(CommandResult::Handled);
        };
        let payload = args.get(2..).unwrap_or(&[]).join(" ");
        if payload.trim().is_empty() {
            emit_command_output(host, "Usage: /background event <source> <payload>");
            return Ok(CommandResult::Handled);
        }
        let triage_args = vec!["queue", source];
        let mut merged = triage_args;
        let payload_parts: Vec<String> =
            payload.split_whitespace().map(|s| s.to_string()).collect();
        let payload_refs: Vec<&str> = payload_parts.iter().map(String::as_str).collect();
        merged.extend(payload_refs);
        return super::handle_trigger_triage_command(host, &merged);
    }
    let job = queue_background_job(&args.join(" "))?;
    emit_command_output(
        host,
        format!(
            "[Background task queued: \"{}\"]\nJob ID: {}\nStatus: {}\nLogs:   {}\nThis task runs in a detached `hermes chat --query ...` process.",
            job.task,
            job.id,
            job.status_path.display(),
            job.log_path.display()
        ),
    );
    Ok(CommandResult::Handled)
}

pub(crate) fn handle_queue_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        emit_command_output(
            host,
            "Usage: /queue <prompt>\nUse `/queue status` to inspect queued/running background jobs.",
        );
        return Ok(CommandResult::Handled);
    }

    if args[0].eq_ignore_ascii_case("status") || args[0].eq_ignore_ascii_case("list") {
        emit_command_output(host, render_background_status(12));
        return Ok(CommandResult::Handled);
    }

    handle_background_command(host, args)
}

pub(crate) fn handle_clear_queue_command(
    host: &mut impl crate::app::SlashCommandHost,
) -> Result<CommandResult, AgentError> {
    let jobs_dir = hermes_config::hermes_home().join("background_jobs");
    let mut removed = 0usize;
    if let Ok(read_dir) = std::fs::read_dir(&jobs_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let map = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            let status = map
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            if matches!(
                status.as_str(),
                "queued" | "running" | "failed" | "completed"
            ) {
                if status == "running" {
                    let pid = map
                        .get("pid")
                        .and_then(|v| v.as_u64())
                        .and_then(|raw| u32::try_from(raw).ok());
                    if let Some(pid) = pid {
                        if process_running(pid) {
                            let _ = terminate_pid(pid);
                        }
                    }
                }
                if std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    emit_command_output(
        host,
        format!("Cleared {} queued/background status file(s).", removed),
    );
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Mission command
// ---------------------------------------------------------------------------

pub(crate) async fn handle_mission_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match action.as_str() {
        "init" => {
            let written = ensure_alpha_runtime_bootstrap(true)?;
            let trading_written = ensure_trading_runtime_bootstrap(true)?;
            let mut details = String::new();
            for path in written {
                let _ = writeln!(details, "- {}", path.display());
            }
            for path in trading_written {
                let _ = writeln!(details, "- {}", path.display());
            }
            emit_command_output(
                host,
                format!(
                    "Mission runtime initialized.\n{}\nUse `/mission status` to inspect active loops.",
                    details.trim_end()
                ),
            );
            Ok(CommandResult::Handled)
        }
        "recover" => {
            let recovered = recover_orphan_loop_events(600)?;
            emit_command_output(
                host,
                format!(
                    "Mission queue recovery complete. Marked {} orphaned running event(s).",
                    recovered
                ),
            );
            Ok(CommandResult::Handled)
        }
        "replay" => {
            let limit = args
                .get(1)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(32);
            let replayed = replay_loop_queue(limit)?;
            emit_command_output(
                host,
                format!(
                    "Mission queue replay complete. Replayed {} event(s) (limit={}).",
                    replayed, limit
                ),
            );
            Ok(CommandResult::Handled)
        }
        "enqueue" => {
            if args.len() < 4 {
                emit_command_output(
                    host,
                    "Usage: /mission enqueue <loop-id> <event-type> <payload text>",
                );
                return Ok(CommandResult::Handled);
            }
            let loop_id = args[1];
            let event_type = args[2];
            let payload = args[3..].join(" ");
            let event = enqueue_loop_event(loop_id, event_type, &payload)?;
            emit_command_output(
                host,
                format!(
                    "Queued mission event {} loop={} type={} status={}",
                    event.id, event.loop_id, event.event_type, event.status
                ),
            );
            Ok(CommandResult::Handled)
        }
        "trading" => {
            let sub = args
                .get(1)
                .copied()
                .unwrap_or("status")
                .to_ascii_lowercase();
            match sub.as_str() {
                "status" | "show" => {
                    let report = load_last_trading_alpha_report()?;
                    emit_command_output(host, render_trading_alpha_board(&report));
                    Ok(CommandResult::Handled)
                }
                "refresh" | "run" | "scan" => {
                    let report = refresh_trading_alpha_report()?;
                    emit_command_output(host, render_trading_alpha_board(&report));
                    Ok(CommandResult::Handled)
                }
                "postmortem" => {
                    let report = load_last_trading_alpha_report()?;
                    emit_command_output(
                        host,
                        format!("Trading postmortem\n\n{}", report.postmortem),
                    );
                    Ok(CommandResult::Handled)
                }
                "autoresearch" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Autoresearch artifacts\n");
                    out.push_str("----------------------\n");
                    out.push_str(&format!("hypotheses: {}\n", report.hypotheses.len()));
                    for h in report.hypotheses.iter().take(12) {
                        let _ = writeln!(
                            out,
                            "- {} novelty={:.3} expected_gain_sol={:.4} :: {}",
                            h.id, h.novelty_score, h.expected_gain_sol, h.statement
                        );
                    }
                    out.push_str("\nexperiments:\n");
                    for e in report.experiments.iter().take(12) {
                        let _ = writeln!(
                            out,
                            "- {} {} -> {} pass={}",
                            e.id, e.control, e.treatment, e.pass_criterion
                        );
                    }
                    out.push_str("\nbacktest_matrix:\n");
                    for row in report.backtest_matrix.iter().take(20) {
                        let _ = writeln!(out, "- {}", row);
                    }
                    out.push_str("\nwalkforward_checks:\n");
                    for row in report.walkforward_checks.iter().take(20) {
                        let _ = writeln!(out, "- {}", row);
                    }
                    out.push_str("\nmeta_ranking:\n");
                    for row in report.meta_ranking.iter().take(20) {
                        let _ = writeln!(out, "- {}", row);
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                "allocator" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Capital Allocator\n");
                    out.push_str("-----------------\n");
                    for row in &report.capital_allocator {
                        let _ = writeln!(
                            out,
                            "- {} weight={:.4} capital_sol={:.6} max_loss_sol={:.6} throttle={:.3}",
                            row.project_id,
                            row.target_weight,
                            row.target_capital_sol,
                            row.max_loss_budget_sol,
                            row.throttle_factor
                        );
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                "governor" => {
                    let report = load_last_trading_alpha_report()?;
                    emit_command_output(
                        host,
                        format!(
                            "Portfolio Risk Governor\n\nmode={}\nhalt_new_entries={}\nmax_portfolio_drawdown_pct={:.4}\nmax_project_drawdown_pct={:.4}\nmax_ruin_probability={:.4}\nreason={}",
                            report.risk_governor.mode,
                            report.risk_governor.halt_new_entries,
                            report.risk_governor.max_portfolio_drawdown_pct,
                            report.risk_governor.max_project_drawdown_pct,
                            report.risk_governor.max_ruin_probability,
                            report.risk_governor.reason
                        ),
                    );
                    Ok(CommandResult::Handled)
                }
                "drift" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Repo Drift Sentinel\n");
                    out.push_str("-------------------\n");
                    for row in &report.repo_drift {
                        let _ = writeln!(
                            out,
                            "- {} state={} head={} baseline={} dirty_files={} changed_since_baseline={}",
                            row.project_id,
                            row.drift_state,
                            row.git_head,
                            row.baseline_head,
                            row.dirty_files,
                            row.changed_since_baseline
                        );
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                "audit" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Run Context Audits\n");
                    out.push_str("------------------\n");
                    for row in &report.run_context_audits {
                        let _ = writeln!(
                            out,
                            "- {} passed={} files_scanned={} missing={}",
                            row.project_id,
                            row.passed,
                            row.files_scanned,
                            if row.missing_metrics.is_empty() {
                                "none".to_string()
                            } else {
                                row.missing_metrics.join(",")
                            }
                        );
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                "provenance" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Env Provenance Gates\n");
                    out.push_str("--------------------\n");
                    for row in &report.env_provenance {
                        let _ = writeln!(
                            out,
                            "- {} passed={} inspected_files={} conflicts={} decision={}",
                            row.project_id,
                            row.passed,
                            row.inspected_files.len(),
                            if row.conflicting_keys.is_empty() {
                                "none".to_string()
                            } else {
                                row.conflicting_keys.join(",")
                            },
                            row.decision
                        );
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                "replay" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Replay Canary Harness\n");
                    out.push_str("---------------------\n");
                    for row in &report.replay_canary {
                        let _ = writeln!(
                            out,
                            "- {} sample_size={} pass_rate={:.3} decision={}",
                            row.project_id, row.sample_size, row.pass_rate, row.decision
                        );
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                "runbook" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Automated Remediation Runbook (Dry Run)\n");
                    out.push_str("---------------------------------------\n");
                    for row in &report.remediation_runbook {
                        let _ = writeln!(
                            out,
                            "- [{}] {} :: {}\n  cmd: {}\n  why: {}",
                            row.priority, row.project_id, row.title, row.command, row.rationale
                        );
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                "sources" => {
                    let report = load_last_trading_alpha_report()?;
                    let mut out = String::new();
                    out.push_str("Research Source Ingestion\n");
                    out.push_str("-------------------------\n");
                    for row in &report.research_sources {
                        let _ = writeln!(
                            out,
                            "- {}:{} found={} items={} path={}",
                            row.project_id, row.source, row.found, row.items, row.path
                        );
                    }
                    emit_command_output(host, out.trim_end());
                    Ok(CommandResult::Handled)
                }
                _ => {
                    emit_command_output(
                        host,
                        "Usage: /mission trading [status|refresh|postmortem|autoresearch|allocator|governor|drift|audit|provenance|replay|runbook|sources]",
                    );
                    Ok(CommandResult::Handled)
                }
            }
        }
        "status" | "show" => {
            let loops = load_alpha_loops()?;
            let (queued, running, completed, failed) = background_job_counts();
            let board = render_mission_board(
                host.current_model(),
                host.session_objective(),
                (queued, running, completed, failed),
            )
            .await?;
            let enabled = loops.iter().filter(|l| l.enabled).count();
            let trading = loops.iter().filter(|l| l.trading_sensitive).count();
            let public = loops.len().saturating_sub(trading);
            let mut out = String::new();
            out.push_str(&board);
            let _ = writeln!(
                out,
                "\nLoop inventory: total={} enabled={} trading_private={} public={}",
                loops.len(),
                enabled,
                trading,
                public
            );
            out.push_str("\nActions:\n");
            out.push_str("- /mission init\n");
            out.push_str("- /mission recover\n");
            out.push_str("- /mission replay [limit]\n");
            out.push_str("- /mission enqueue <loop-id> <event-type> <payload>\n");
            out.push_str("- /mission trading [status|refresh|postmortem|autoresearch|allocator|governor|drift|audit|provenance|replay|runbook|sources]\n");
            out.push_str("- /objective <text>\n");
            out.push_str("- /background <task>\n");
            emit_command_output(host, out.trim_end());
            Ok(CommandResult::Handled)
        }
        _ => {
            emit_command_output(
                host,
                "Usage: /mission [status|init|recover|replay [limit]|enqueue <loop-id> <event-type> <payload>|trading ...]",
            );
            Ok(CommandResult::Handled)
        }
    }
}
