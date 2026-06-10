//! Session slash command handlers.
//!
//! Provides `/save`, `/load`, `/resume`, `/sessions`, `/snapshot`, `/rollback`,
//! `/timetravel`, `/branch`, `/title` (`/topic` compat) slash-command implementations,
//! snapshot integrity helpers, branch checkpoint logic, and state.db session management.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hermes_agent::{SessionPersistence, format_session_db_unavailable};
use hermes_core::AgentError;

use super::{CommandResult, emit_command_output, truncate_chars};
use crate::App;

// ---------------------------------------------------------------------------
// handle_save_command
// ---------------------------------------------------------------------------

pub(crate) fn handle_save_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let path = app.persist_session_snapshot(args.first().copied())?;
    emit_command_output(app, format!("Session saved to {}", path.display()));
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Snapshot enumeration
// ---------------------------------------------------------------------------

pub(crate) fn enumerate_saved_sessions(sessions_dir: &Path) -> Vec<(String, PathBuf, SystemTime)> {
    let mut entries: Vec<(String, PathBuf, SystemTime)> = std::fs::read_dir(sessions_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.filter_map(|e| e.ok()))
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                return None;
            }
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if stem.trim().is_empty() {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((stem, path, modified))
        })
        .collect();
    entries.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    entries
}

// ---------------------------------------------------------------------------
// SnapshotIntegrity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct SnapshotIntegrity {
    pub(crate) valid: bool,
    pub(crate) reason: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) message_count: usize,
}

pub(crate) fn inspect_snapshot_integrity(path: &Path) -> SnapshotIntegrity {
    let raw = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(err) => {
            return SnapshotIntegrity {
                valid: false,
                reason: Some(format!("read_failed: {}", err)),
                session_id: None,
                message_count: 0,
            };
        }
    };
    let data: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            return SnapshotIntegrity {
                valid: false,
                reason: Some(format!("json_invalid: {}", err)),
                session_id: None,
                message_count: 0,
            };
        }
    };
    let messages = match data.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => {
            return SnapshotIntegrity {
                valid: false,
                reason: Some("missing_messages_array".to_string()),
                session_id: data
                    .get("session_info")
                    .and_then(|v| v.get("session_id"))
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                message_count: 0,
            };
        }
    };
    SnapshotIntegrity {
        valid: true,
        reason: None,
        session_id: data
            .get("session_info")
            .and_then(|v| v.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
        message_count: messages.len(),
    }
}

pub(crate) fn is_canonical_snapshot_name(name: &str, integrity: &SnapshotIntegrity) -> bool {
    let stem = name.trim();
    let Some(session_id) = integrity
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    !stem.is_empty() && stem.eq_ignore_ascii_case(session_id)
}

// ---------------------------------------------------------------------------
// Resolve & load helpers
// ---------------------------------------------------------------------------

pub(crate) fn resolve_saved_session_entry<'a>(
    entries: &'a [(String, PathBuf, SystemTime)],
    requested: &str,
) -> Result<&'a (String, PathBuf, SystemTime), String> {
    if let Some(entry) = entries
        .iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case(requested))
    {
        return Ok(entry);
    }

    let prefix_matches: Vec<&(String, PathBuf, SystemTime)> = entries
        .iter()
        .filter(|(name, _, _)| name.starts_with(requested))
        .collect();
    match prefix_matches.as_slice() {
        [entry] => Ok(*entry),
        [] => Err(format!("not_found: {}", requested)),
        many => Err(format!(
            "ambiguous: {}",
            many.iter()
                .map(|entry| format!("`{}`", entry.0))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub(crate) fn message_from_snapshot_entry(entry: &serde_json::Value) -> hermes_core::Message {
    let role_str = entry.get("role").and_then(|r| r.as_str()).unwrap_or("User");
    let content_str = entry.get("content").and_then(|c| c.as_str()).unwrap_or("");
    match role_str {
        "Assistant" => hermes_core::Message::assistant(content_str),
        "System" => hermes_core::Message::system(content_str),
        "Tool" => hermes_core::Message::assistant(content_str),
        _ => hermes_core::Message::user(content_str),
    }
}

pub(crate) fn load_messages_from_snapshot(
    path: &Path,
) -> Result<Vec<hermes_core::Message>, AgentError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AgentError::Io(format!("Failed to read session: {}", e)))?;
    let data: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AgentError::Config(format!("Failed to parse session: {}", e)))?;
    let messages = data
        .get("messages")
        .and_then(|m| m.as_array())
        .ok_or_else(|| AgentError::Config("Session file has no messages array.".to_string()))?;
    Ok(messages.iter().map(message_from_snapshot_entry).collect())
}

// ---------------------------------------------------------------------------
// Message signature & diff
// ---------------------------------------------------------------------------

pub(crate) fn message_signature(message: &hermes_core::Message) -> String {
    let role = match message.role {
        hermes_core::MessageRole::System => "system",
        hermes_core::MessageRole::User => "user",
        hermes_core::MessageRole::Assistant => "assistant",
        hermes_core::MessageRole::Tool => "tool",
    };
    format!(
        "{}|{}",
        role,
        message.content.as_deref().unwrap_or_default()
    )
}

pub(crate) fn summarize_branch_diff(
    left_name: &str,
    left_messages: &[hermes_core::Message],
    right_name: &str,
    right_messages: &[hermes_core::Message],
) -> String {
    let left_set: HashSet<String> = left_messages.iter().map(message_signature).collect();
    let right_set: HashSet<String> = right_messages.iter().map(message_signature).collect();
    let only_left = left_set.difference(&right_set).count();
    let only_right = right_set.difference(&left_set).count();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Branch diff: `{}` vs `{}`",
        left_name.trim(),
        right_name.trim()
    );
    let _ = writeln!(
        out,
        "  messages: {} vs {}",
        left_messages.len(),
        right_messages.len()
    );
    let _ = writeln!(out, "  unique_to_{}: {}", left_name.trim(), only_left);
    let _ = writeln!(out, "  unique_to_{}: {}", right_name.trim(), only_right);
    let left_last = left_messages
        .iter()
        .rev()
        .find(|m| m.role == hermes_core::MessageRole::Assistant)
        .and_then(|m| m.content.as_deref())
        .unwrap_or("");
    let right_last = right_messages
        .iter()
        .rev()
        .find(|m| m.role == hermes_core::MessageRole::Assistant)
        .and_then(|m| m.content.as_deref())
        .unwrap_or("");
    let _ = writeln!(
        out,
        "  last_assistant_{}: {}",
        left_name.trim(),
        truncate_chars(left_last, 120)
    );
    let _ = writeln!(
        out,
        "  last_assistant_{}: {}",
        right_name.trim(),
        truncate_chars(right_last, 120)
    );
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// State.db session helpers
// ---------------------------------------------------------------------------

pub(crate) fn session_db(app: &App) -> SessionPersistence {
    SessionPersistence::new(&app.state_root)
}

/// Try to restore a session from `state.db`. Returns `Ok(None)` when DB is
/// unavailable or target not found.
pub(crate) fn try_load_session_from_db(
    app: &mut App,
    target: Option<&str>,
    resume_mode: bool,
) -> Result<Option<CommandResult>, AgentError> {
    let sp = session_db(app);
    if sp.ensure_db().is_err() {
        return Ok(None);
    }

    let session_id = if let Some(name) = target.map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(Some(by_title)) = sp.resolve_session_by_title(name) {
            by_title
        } else if let Ok(Some(by_id)) = sp.resolve_session_id(name) {
            by_id
        } else {
            return Ok(None);
        }
    } else {
        let rows = sp.list_sessions_rich(None, &["tool", "internal"], 1, 0, 1, true)?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        row.id
    };

    let resolved = sp.resolve_resume_session_id(&session_id)?;
    let messages = sp.load_session(&resolved)?;
    if messages.is_empty() {
        return Ok(None);
    }

    let meta = sp.get_session(&resolved)?;
    let display = meta
        .as_ref()
        .and_then(|s| s.title.clone())
        .unwrap_or_else(|| resolved.clone());
    let model = meta.as_ref().and_then(|s| s.model.clone());

    let old_session_id = app.session_id.clone();
    app.messages = messages;
    app.ui_messages.clear();

    if resume_mode {
        if resolved != old_session_id {
            app.notify_memory_session_switch(&resolved, &old_session_id, false, "resume");
        } else {
            app.agent.set_runtime_session_id(&resolved);
        }
        app.session_id = resolved.clone();
        let _ = sp.reopen_session(&resolved);
    }

    let mut model_note = String::new();
    if let Some(restored_model) = model.as_deref().filter(|s| !s.is_empty()) {
        if !restored_model.eq_ignore_ascii_case(&app.current_model) {
            let previous = app.current_model.clone();
            app.switch_model(restored_model);
            model_note = format!("\nModel restored: {} -> {}", previous, app.current_model);
        }
    }

    let verb = if resume_mode { "Resumed" } else { "Loaded" };
    emit_command_output(
        app,
        format!(
            "{} session '{}' from state.db ({} messages; session_id={}){}",
            verb,
            display,
            app.messages.len(),
            resolved,
            model_note
        ),
    );
    Ok(Some(CommandResult::Handled))
}

pub(crate) fn format_db_session_list(app: &App) -> Result<Option<String>, AgentError> {
    let sp = session_db(app);
    if sp.ensure_db().is_err() {
        return Ok(None);
    }
    let rows = sp.list_sessions_rich(None, &["tool", "internal"], 20, 0, 0, true)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut out = String::from("Saved sessions (state.db):\n");
    for (idx, row) in rows.iter().enumerate() {
        let title = row.title.as_deref().unwrap_or("(untitled)");
        let marker = if idx == 0 { " (latest)" } else { "" };
        let preview = row.preview.as_deref().unwrap_or("");
        let _ = writeln!(
            out,
            "- `{}`{} — id={} msgs={} — {}",
            title, marker, row.id, row.message_count, preview
        );
    }
    out.push_str("\nUsage: `/load <session-title|id>` or `/resume [session-title|id]`");
    Ok(Some(out.trim_end().to_string()))
}

// ---------------------------------------------------------------------------
// Load session from path
// ---------------------------------------------------------------------------

pub(crate) fn load_session_from_path(
    app: &mut App,
    session_name: &str,
    path: &Path,
    resume_mode: bool,
) -> Result<CommandResult, AgentError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AgentError::Io(format!("Failed to read session: {}", e)))?;
    let data: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AgentError::Config(format!("Failed to parse session: {}", e)))?;

    let Some(messages) = data.get("messages").and_then(|m| m.as_array()) else {
        emit_command_output(app, "Session file has no messages array.");
        return Ok(CommandResult::Handled);
    };

    let old_session_id = app.session_id.clone();
    app.messages.clear();
    app.ui_messages.clear();
    for msg in messages {
        app.messages.push(message_from_snapshot_entry(msg));
    }

    let session_info = data.get("session_info");
    if resume_mode {
        if let Some(restored_id) = session_info
            .and_then(|s| s.get("session_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if restored_id != old_session_id {
                app.notify_memory_session_switch(restored_id, &old_session_id, false, "resume");
            } else {
                app.agent.set_runtime_session_id(restored_id);
            }
            app.session_id = restored_id.to_string();
        }
    }

    let mut model_note = String::new();
    if let Some(restored_model) = session_info
        .and_then(|s| s.get("model"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !restored_model.eq_ignore_ascii_case(&app.current_model) {
            let previous = app.current_model.clone();
            app.switch_model(restored_model);
            model_note = format!("\nModel restored: {} -> {}", previous, app.current_model);
        }
    }

    if let Some(personality) = session_info
        .and_then(|s| s.get("personality"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        app.current_personality = Some(personality.to_string());
    }

    let verb = if resume_mode { "Resumed" } else { "Loaded" };
    emit_command_output(
        app,
        format!(
            "{} session '{}' ({} messages; session_id={}){}",
            verb,
            session_name,
            app.messages.len(),
            app.session_id,
            model_note
        ),
    );
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /load
// ---------------------------------------------------------------------------

pub(crate) fn handle_load_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        if let Some(db_list) = format_db_session_list(app)? {
            emit_command_output(app, db_list);
            return Ok(CommandResult::Handled);
        }
    }

    let sessions_dir = hermes_config::hermes_home().join("sessions");

    if args.is_empty() {
        if !sessions_dir.exists() {
            emit_command_output(app, "No saved sessions found.");
            return Ok(CommandResult::Handled);
        }
        let entries = enumerate_saved_sessions(&sessions_dir);
        if entries.is_empty() {
            emit_command_output(app, "No saved sessions found.");
        } else {
            let mut out = String::from("Saved sessions:\n");
            for (idx, (name, _, _)) in entries.iter().enumerate() {
                let integrity = inspect_snapshot_integrity(&entries[idx].1);
                let marker = if integrity.valid { "✓" } else { "⚠" };
                let detail = if integrity.valid {
                    format!(
                        "session_id={} messages={}",
                        integrity.session_id.unwrap_or_else(|| "?".to_string()),
                        integrity.message_count
                    )
                } else {
                    integrity
                        .reason
                        .unwrap_or_else(|| "invalid snapshot".to_string())
                };
                if idx == 0 {
                    out.push_str(&format!("- {} `{}` (latest) — {}\n", marker, name, detail));
                } else {
                    out.push_str(&format!("- {} `{}` — {}\n", marker, name, detail));
                }
            }
            out.push_str("\nUsage: `/load <session-name>` or `/resume [session-name]`");
            emit_command_output(app, out.trim_end());
        }
        return Ok(CommandResult::Handled);
    }

    let name = args[0];
    if let Some(result) = try_load_session_from_db(app, Some(name), false)? {
        return Ok(result);
    }
    let path = sessions_dir.join(format!("{}.json", name));
    if !path.exists() {
        emit_command_output(
            app,
            format!("Session '{}' not found at {}", name, path.display()),
        );
        return Ok(CommandResult::Handled);
    }
    load_session_from_path(app, name, &path, false)
}

// ---------------------------------------------------------------------------
// /resume
// ---------------------------------------------------------------------------

pub(crate) fn handle_resume_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        if let Some(result) = try_load_session_from_db(app, None, true)? {
            return Ok(result);
        }
    } else if let Some(result) = try_load_session_from_db(app, Some(args[0]), true)? {
        return Ok(result);
    }

    let sessions_dir = hermes_config::hermes_home().join("sessions");
    if !sessions_dir.exists() {
        emit_command_output(
            app,
            format_session_db_unavailable(
                "No saved sessions found and session database not available",
            ),
        );
        return Ok(CommandResult::Handled);
    }
    let entries = enumerate_saved_sessions(&sessions_dir);
    if entries.is_empty() {
        emit_command_output(
            app,
            format_session_db_unavailable(
                "No saved sessions found and session database not available",
            ),
        );
        return Ok(CommandResult::Handled);
    }

    if args.is_empty() {
        let pick = entries
            .iter()
            .find(|(name, path, _)| {
                let integrity = inspect_snapshot_integrity(path);
                integrity.valid
                    && integrity.message_count > 0
                    && is_canonical_snapshot_name(name, &integrity)
            })
            .or_else(|| {
                entries.iter().find(|(name, path, _)| {
                    let integrity = inspect_snapshot_integrity(path);
                    integrity.valid && is_canonical_snapshot_name(name, &integrity)
                })
            })
            .or_else(|| {
                entries
                    .iter()
                    .find(|(_, path, _)| inspect_snapshot_integrity(path).valid)
            });
        if let Some((name, path, _)) = pick {
            return load_session_from_path(app, name, path, true);
        }
        emit_command_output(
            app,
            "No valid saved sessions found (all snapshots are malformed). Use `/sessions` to inspect and `/save` to create a fresh checkpoint.",
        );
        return Ok(CommandResult::Handled);
    }

    let requested = args[0];
    match resolve_saved_session_entry(&entries, requested) {
        Ok((name, path, _)) => {
            let integrity = inspect_snapshot_integrity(path);
            if !integrity.valid {
                emit_command_output(
                    app,
                    format!(
                        "Session '{}' is present but invalid: {}.\nUse `/sessions` to inspect snapshot health.",
                        requested,
                        integrity
                            .reason
                            .unwrap_or_else(|| "malformed session snapshot".to_string())
                    ),
                );
                return Ok(CommandResult::Handled);
            }
            load_session_from_path(app, name, path, true)
        }
        Err(err) if err.starts_with("not_found:") => {
            emit_command_output(
                app,
                format!(
                    "Session '{}' not found. Use `/load` to list saved sessions.",
                    requested
                ),
            );
            Ok(CommandResult::Handled)
        }
        Err(err) => {
            emit_command_output(
                app,
                format!(
                    "Session name '{}' is ambiguous. Matches: {}",
                    requested,
                    err.trim_start_matches("ambiguous: ")
                ),
            );
            Ok(CommandResult::Handled)
        }
    }
}

// ---------------------------------------------------------------------------
// /sessions
// ---------------------------------------------------------------------------

pub(crate) fn handle_sessions_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        return handle_load_command(app, args);
    }
    let action = args[0].to_ascii_lowercase();
    if action == "doctor" || action == "verify" {
        let sessions_dir = hermes_config::hermes_home().join("sessions");
        let entries = enumerate_saved_sessions(&sessions_dir);
        if entries.is_empty() {
            emit_command_output(app, "No saved sessions found.");
            return Ok(CommandResult::Handled);
        }
        let mut invalid = Vec::new();
        let mut by_session_id: HashMap<String, Vec<String>> = HashMap::new();
        for (name, path, _) in entries {
            let integrity = inspect_snapshot_integrity(&path);
            if integrity.valid {
                if let Some(id) = integrity.session_id {
                    by_session_id.entry(id).or_default().push(name);
                }
            } else {
                invalid.push((
                    name,
                    integrity
                        .reason
                        .unwrap_or_else(|| "invalid snapshot".to_string()),
                ));
            }
        }
        let split = by_session_id
            .iter()
            .filter(|(_, names)| names.len() > 1)
            .map(|(session_id, names)| format!("{} => {}", session_id, names.join(", ")))
            .collect::<Vec<_>>();
        let mut out = String::new();
        out.push_str("Session snapshot doctor\n");
        out.push_str("-----------------------\n");
        let _ = writeln!(out, "invalid_snapshots={}", invalid.len());
        let _ = writeln!(out, "split_session_ids={}", split.len());
        if !invalid.is_empty() {
            out.push_str("invalid_details:\n");
            for (name, reason) in invalid.into_iter().take(20) {
                let _ = writeln!(out, "- {}: {}", name, reason);
            }
        }
        if !split.is_empty() {
            out.push_str("split_details:\n");
            for row in split.into_iter().take(20) {
                let _ = writeln!(out, "- {}", row);
            }
        }
        out.push_str("Recommendation: `/save` now to create a fresh canonical checkpoint.");
        emit_command_output(app, out.trim_end());
        return Ok(CommandResult::Handled);
    }
    handle_resume_command(app, args)
}

// ---------------------------------------------------------------------------
// /snapshot
// ---------------------------------------------------------------------------

pub(crate) fn handle_snapshot_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let sessions_dir = hermes_config::hermes_home().join("sessions");
    if args.is_empty() || args[0].eq_ignore_ascii_case("list") {
        if !sessions_dir.exists() {
            emit_command_output(
                app,
                format!(
                    "No snapshots found in {}.\nUse `/snapshot save [name]` to create one.",
                    sessions_dir.display()
                ),
            );
            return Ok(CommandResult::Handled);
        }
        let entries = enumerate_saved_sessions(&sessions_dir);
        if entries.is_empty() {
            emit_command_output(
                app,
                format!(
                    "No snapshots found in {}.\nUse `/snapshot save [name]` to create one.",
                    sessions_dir.display()
                ),
            );
            return Ok(CommandResult::Handled);
        }
        let mut out = String::new();
        let _ = writeln!(out, "Session snapshots:");
        for (idx, (name, path, _)) in entries.iter().take(20).enumerate() {
            let marker = if idx == 0 { " (latest)" } else { "" };
            let _ = writeln!(out, "  - {}{}  -> {}", name, marker, path.display());
        }
        let _ = writeln!(
            out,
            "\nUse `/snapshot save [name]` to create, `/rollback latest` to restore latest, or `/load <snapshot-name>` to load a specific snapshot."
        );
        emit_command_output(app, out.trim_end());
        return Ok(CommandResult::Handled);
    }

    let save_name = if args[0].eq_ignore_ascii_case("save") {
        args.get(1).copied()
    } else {
        args.first().copied()
    };
    let path = app.persist_session_snapshot(save_name)?;
    emit_command_output(app, format!("Snapshot saved: {}", path.display()));
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /rollback
// ---------------------------------------------------------------------------

pub(crate) fn handle_rollback_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() || args[0].eq_ignore_ascii_case("list") {
        let sessions_dir = hermes_config::hermes_home().join("sessions");
        let entries = enumerate_saved_sessions(&sessions_dir);
        let mut out = String::from("Rollback controls:\n");
        out.push_str("- `/rollback undo [n]`      revert the last exchange(s)\n");
        out.push_str("- `/rollback latest`        load latest snapshot\n");
        out.push_str("- `/rollback load <name>`   load named snapshot\n");
        if entries.is_empty() {
            out.push_str("- snapshots: none yet (`/snapshot save` to create one)\n");
        } else {
            out.push_str("- recent snapshots:\n");
            for (name, _, _) in entries.into_iter().take(5) {
                out.push_str(&format!("    - {}\n", name));
            }
        }
        emit_command_output(app, out.trim_end());
        return Ok(CommandResult::Handled);
    }

    let sub = args[0];
    if sub.eq_ignore_ascii_case("undo") || sub.parse::<usize>().is_ok() {
        let steps = if sub.eq_ignore_ascii_case("undo") {
            args.get(1)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
        } else {
            sub.parse::<usize>().unwrap_or(1)
        };
        let bounded = steps.clamp(1, 64);
        for _ in 0..bounded {
            app.undo_last();
        }
        emit_command_output(
            app,
            format!("Rolled back {} exchange(s) via undo.", bounded),
        );
        return Ok(CommandResult::Handled);
    }

    if sub.eq_ignore_ascii_case("latest") {
        let sessions_dir = hermes_config::hermes_home().join("sessions");
        let entries = enumerate_saved_sessions(&sessions_dir);
        let Some((name, path, _)) = entries.first() else {
            emit_command_output(app, "No snapshots available to rollback.");
            return Ok(CommandResult::Handled);
        };
        return load_session_from_path(app, name, path, false);
    }

    if sub.eq_ignore_ascii_case("load") {
        let Some(name) = args.get(1).copied() else {
            emit_command_output(app, "Usage: /rollback load <snapshot-name>");
            return Ok(CommandResult::Handled);
        };
        return handle_load_command(app, &[name]);
    }

    handle_load_command(app, &[sub])
}

// ---------------------------------------------------------------------------
// /timetravel
// ---------------------------------------------------------------------------

pub(crate) fn handle_timetravel_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        return handle_snapshot_command(app, &["list"]);
    }
    match args[0].to_ascii_lowercase().as_str() {
        "help" => {
            emit_command_output(
                app,
                "Usage: /timetravel [list|latest|goto <snapshot>|undo [n]|branch [label]]\n\
                 - list: show snapshot checkpoints\n\
                 - latest: jump to latest snapshot\n\
                 - goto <snapshot>: jump to named snapshot\n\
                 - undo [n]: undo latest exchange(s)\n\
                 - branch [label]: create a branch checkpoint marker",
            );
            Ok(CommandResult::Handled)
        }
        "list" | "ls" | "show" => handle_snapshot_command(app, &["list"]),
        "latest" => handle_rollback_command(app, &["latest"]),
        "goto" | "jump" => {
            let Some(name) = args.get(1).copied() else {
                emit_command_output(app, "Usage: /timetravel goto <snapshot-name>");
                return Ok(CommandResult::Handled);
            };
            handle_load_command(app, &[name])
        }
        "undo" => handle_rollback_command(app, args),
        "branch" | "fork" => {
            let label = args.get(1).copied().unwrap_or("timetravel");
            handle_branch_command(app, &[label])
        }
        other => {
            if other.parse::<usize>().is_ok() {
                handle_rollback_command(app, args)
            } else {
                emit_command_output(
                    app,
                    format!(
                        "Unknown /timetravel action '{}'. Use `/timetravel help`.",
                        other
                    ),
                );
                Ok(CommandResult::Handled)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// branch_checkpoint_name
// ---------------------------------------------------------------------------

pub(crate) fn branch_checkpoint_name(session_id: &str, label: Option<&str>) -> String {
    let requested = label.unwrap_or("branch").trim();
    let sanitized = requested
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    format!(
        "branch-{}-{}",
        &session_id[..8.min(session_id.len())],
        if sanitized.is_empty() {
            "checkpoint"
        } else {
            sanitized.as_str()
        }
    )
}

// ---------------------------------------------------------------------------
// /branch
// ---------------------------------------------------------------------------

pub(crate) fn handle_branch_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let sessions_dir = hermes_config::hermes_home().join("sessions");
    let entries = enumerate_saved_sessions(&sessions_dir);
    let action = args
        .first()
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "save".to_string());

    match action.as_str() {
        "help" => {
            emit_command_output(
                app,
                "Usage: /branch [label]\n\
                       /branch list\n\
                       /branch diff <left> [right]\n\
                       /branch merge <source> [target]\n\
                 Notes:\n\
                 - save: creates a branch checkpoint snapshot\n\
                 - diff: compares message footprints between snapshots\n\
                 - merge: appends unique messages from source into target/current session",
            );
            return Ok(CommandResult::Handled);
        }
        "list" | "ls" | "show" => {
            if entries.is_empty() {
                emit_command_output(app, "No snapshots found. Use `/branch <label>` first.");
                return Ok(CommandResult::Handled);
            }
            let mut out = String::from("Branch checkpoints:\n");
            let mut shown = 0usize;
            for (name, path, _) in entries.iter() {
                if !name.starts_with("branch-") {
                    continue;
                }
                let integrity = inspect_snapshot_integrity(path);
                let marker = if integrity.valid { "✓" } else { "⚠" };
                let detail = if integrity.valid {
                    format!("messages={}", integrity.message_count)
                } else {
                    integrity
                        .reason
                        .unwrap_or_else(|| "invalid snapshot".to_string())
                };
                let _ = writeln!(out, "  - {} `{}` ({})", marker, name, detail);
                shown += 1;
                if shown >= 25 {
                    break;
                }
            }
            if shown == 0 {
                out.push_str("  (no branch-* checkpoints found)\n");
            }
            out.push_str(
                "\nUse `/branch diff <left> [right]` or `/branch merge <source> [target]`.",
            );
            emit_command_output(app, out.trim_end());
            return Ok(CommandResult::Handled);
        }
        "diff" => {
            let Some(left_name) = args.get(1).copied() else {
                emit_command_output(app, "Usage: /branch diff <left> [right]");
                return Ok(CommandResult::Handled);
            };
            let right_name = args.get(2).copied().unwrap_or("latest");
            let left_entry = match resolve_saved_session_entry(&entries, left_name) {
                Ok(entry) => entry,
                Err(err) if err.starts_with("not_found:") => {
                    emit_command_output(app, format!("Snapshot '{}' not found.", left_name));
                    return Ok(CommandResult::Handled);
                }
                Err(err) => {
                    emit_command_output(
                        app,
                        format!(
                            "Snapshot '{}' is ambiguous. Matches: {}",
                            left_name,
                            err.trim_start_matches("ambiguous: ")
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
            };
            let right_entry = if right_name.eq_ignore_ascii_case("latest") {
                match entries.first() {
                    Some(entry) => entry,
                    None => {
                        emit_command_output(app, "No snapshots found.");
                        return Ok(CommandResult::Handled);
                    }
                }
            } else {
                match resolve_saved_session_entry(&entries, right_name) {
                    Ok(entry) => entry,
                    Err(err) if err.starts_with("not_found:") => {
                        emit_command_output(app, format!("Snapshot '{}' not found.", right_name));
                        return Ok(CommandResult::Handled);
                    }
                    Err(err) => {
                        emit_command_output(
                            app,
                            format!(
                                "Snapshot '{}' is ambiguous. Matches: {}",
                                right_name,
                                err.trim_start_matches("ambiguous: ")
                            ),
                        );
                        return Ok(CommandResult::Handled);
                    }
                }
            };
            let left_messages = load_messages_from_snapshot(&left_entry.1)?;
            let right_messages = load_messages_from_snapshot(&right_entry.1)?;
            emit_command_output(
                app,
                summarize_branch_diff(
                    &left_entry.0,
                    &left_messages,
                    &right_entry.0,
                    &right_messages,
                ),
            );
            return Ok(CommandResult::Handled);
        }
        "merge" => {
            let Some(source_name) = args.get(1).copied() else {
                emit_command_output(app, "Usage: /branch merge <source> [target]");
                return Ok(CommandResult::Handled);
            };
            let source_entry = match resolve_saved_session_entry(&entries, source_name) {
                Ok(entry) => entry,
                Err(err) if err.starts_with("not_found:") => {
                    emit_command_output(app, format!("Snapshot '{}' not found.", source_name));
                    return Ok(CommandResult::Handled);
                }
                Err(err) => {
                    emit_command_output(
                        app,
                        format!(
                            "Snapshot '{}' is ambiguous. Matches: {}",
                            source_name,
                            err.trim_start_matches("ambiguous: ")
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
            };

            let mut target_label = "current".to_string();
            let mut merged_messages = app.messages.clone();
            if let Some(target_name) = args.get(2).copied() {
                let target_entry = match resolve_saved_session_entry(&entries, target_name) {
                    Ok(entry) => entry,
                    Err(err) if err.starts_with("not_found:") => {
                        emit_command_output(app, format!("Snapshot '{}' not found.", target_name));
                        return Ok(CommandResult::Handled);
                    }
                    Err(err) => {
                        emit_command_output(
                            app,
                            format!(
                                "Snapshot '{}' is ambiguous. Matches: {}",
                                target_name,
                                err.trim_start_matches("ambiguous: ")
                            ),
                        );
                        return Ok(CommandResult::Handled);
                    }
                };
                target_label = target_entry.0.clone();
                merged_messages = load_messages_from_snapshot(&target_entry.1)?;
            }

            let source_messages = load_messages_from_snapshot(&source_entry.1)?;
            let mut seen: HashSet<String> = merged_messages.iter().map(message_signature).collect();
            let mut appended = 0usize;
            for msg in source_messages {
                let sig = message_signature(&msg);
                if seen.insert(sig) {
                    merged_messages.push(msg);
                    appended += 1;
                }
            }
            let merged_total = merged_messages.len();
            app.messages = merged_messages;
            app.ui_messages
                .retain(|msg| msg.insert_at <= app.messages.len());
            let stem = branch_checkpoint_name(
                &app.session_id,
                Some(&format!("merge-{}-into-{}", source_entry.0, target_label)),
            );
            let path = app.persist_session_snapshot(Some(&stem))?;
            emit_command_output(
                app,
                format!(
                    "Branch merge complete.\n  source: {}\n  target: {}\n  appended_unique_messages: {}\n  merged_total_messages: {}\n  snapshot: {}",
                    source_entry.0,
                    target_label,
                    appended,
                    merged_total,
                    path.display()
                ),
            );
            return Ok(CommandResult::Handled);
        }
        _ => {}
    }

    let label = if args.is_empty() {
        None
    } else {
        Some(args.join(" "))
    };
    let stem = branch_checkpoint_name(&app.session_id, label.as_deref());
    match app.persist_session_snapshot(Some(&stem)) {
        Ok(path) => emit_command_output(
            app,
            format!(
                "Branch checkpoint saved: {}\nContinue in current session or run `/resume {}`.",
                path.display(),
                stem
            ),
        ),
        Err(err) => emit_command_output(
            app,
            format!("Branch marker requested, but snapshot failed: {}", err),
        ),
    }
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /title compat (/topic, /branch aliases)
// ---------------------------------------------------------------------------

pub(crate) fn handle_session_compat_command(
    app: &mut App,
    cmd: &str,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let arg_joined = args.join(" ");
    let msg = match cmd {
        "/title" => {
            if arg_joined.trim().is_empty() {
                match session_db(app).get_session_title(&app.session_id) {
                    Ok(Some(title)) => format!("Session title: {title}"),
                    Ok(None) => "Session has no title.".to_string(),
                    Err(e) => format!(
                        "Session title unavailable: {}",
                        format_session_db_unavailable(&e.to_string())
                    ),
                }
            } else {
                match session_db(app).set_session_title(&app.session_id, Some(arg_joined.trim())) {
                    Ok(true) => format!("Session title set to: {}", arg_joined.trim()),
                    Ok(false) => "Session not found in state.db.".to_string(),
                    Err(e) => format!("Failed to set title: {e}"),
                }
            }
        }
        "/branch" => "Use `/branch` (native) for list/diff/merge/save controls.".to_string(),
        _ => "Compatibility command acknowledged.".to_string(),
    };
    emit_command_output(app, msg);
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env_lock;
    use clap::Parser;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    // --

    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        test_env_lock::lock()
    }

    struct TempHomeGuard {
        previous_home: Option<String>,
        previous_clipboard_mock: Option<String>,
        previous_runtime_env: Vec<(&'static str, Option<String>)>,
    }

    impl TempHomeGuard {
        fn new(path: &Path) -> Self {
            let previous_home = std::env::var("HERMES_HOME").ok();
            crate::env_vars::set_var("HERMES_HOME", path);
            let previous_clipboard_mock = std::env::var("HERMES_TEST_CLIPBOARD_TEXT").ok();
            crate::env_vars::remove_var("HERMES_TEST_CLIPBOARD_TEXT");
            let previous_runtime_env = [
                "HERMES_MODEL",
                "HERMES_INFERENCE_MODEL",
                "HERMES_INFERENCE_PROVIDER",
                "HERMES_TUI_PROVIDER",
            ]
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
            Self {
                previous_home,
                previous_clipboard_mock,
                previous_runtime_env,
            }
        }
    }

    impl Drop for TempHomeGuard {
        fn drop(&mut self) {
            match self.previous_home.take() {
                Some(value) => crate::env_vars::set_var("HERMES_HOME", value),
                None => crate::env_vars::remove_var("HERMES_HOME"),
            }
            match self.previous_clipboard_mock.take() {
                Some(value) => crate::env_vars::set_var("HERMES_TEST_CLIPBOARD_TEXT", value),
                None => crate::env_vars::remove_var("HERMES_TEST_CLIPBOARD_TEXT"),
            }
            for (key, value) in self.previous_runtime_env.drain(..) {
                match value {
                    Some(v) => crate::env_vars::set_var(key, v),
                    None => crate::env_vars::remove_var(key),
                }
            }
        }
    }

    async fn build_test_app_with_stream(home: &Path) -> App {
        let config_dir = home.join("config");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        let cli = crate::cli::Cli::try_parse_from(vec![
            "hermes".to_string(),
            "-C".to_string(),
            config_dir.display().to_string(),
            "--ignore-user-config".to_string(),
            "--ignore-rules".to_string(),
        ])
        .expect("parse cli");
        let mut app = App::new(cli).await.expect("build app");
        let (tx, _rx) = mpsc::unbounded_channel::<crate::tui::Event>();
        app.set_stream_handle(Some(tx.into()));
        app
    }

    fn latest_ui_assistant_text(app: &App) -> String {
        app.ui_messages
            .iter()
            .rev()
            .find(|row| row.message.role == hermes_core::MessageRole::Assistant)
            .and_then(|row| row.message.content.clone())
            .unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Session command tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn promoted_snapshot_command_lists_snapshots() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_snapshot_command(&mut app, &[]).expect("snapshot list");
        assert_eq!(result, CommandResult::Handled);

        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("Session snapshots:") || output.contains("No snapshots found in"));
    }

    #[tokio::test]
    async fn promoted_rollback_command_shows_controls() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_rollback_command(&mut app, &[]).expect("rollback list");
        assert_eq!(result, CommandResult::Handled);
        assert!(latest_ui_assistant_text(&app).contains("Rollback controls:"));
    }

    #[test]
    fn test_resume_command_is_registered_and_completable() {
        assert!(
            super::super::SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/resume")
        );
        let results = super::super::autocomplete("/res");
        assert!(results.contains(&"/resume"));
    }

    #[test]
    fn test_timetravel_command_and_alias_are_registered() {
        assert!(
            super::super::SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/timetravel")
        );
        assert!(
            super::super::SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/tt")
        );
        assert_eq!(super::super::canonical_command("/tt"), "/timetravel");
        let results = super::super::autocomplete("/time");
        assert!(results.contains(&"/timetravel"));
    }

    #[test]
    fn test_sessions_command_is_registered_and_completable() {
        assert!(
            super::super::SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/sessions")
        );
        let results = super::super::autocomplete("/sess");
        assert!(results.contains(&"/sessions"));
    }
}
