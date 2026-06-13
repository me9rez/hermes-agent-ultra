//! Diagnostic and introspection slash commands (`/log`, `/debug-dump`, `/insights`, etc.).

use std::fmt::Write as _;
use std::path::Path;

use hermes_core::AgentError;

use crate::commands::{CommandResult, emit_command_output};

pub(crate) fn handle_image_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        let status = host
            .pending_image_hint()
            .map(|path| {
                format!(
                    "Pending image hint: {}\nUse `/image clear` to remove it.",
                    path
                )
            })
            .unwrap_or_else(|| {
                "No pending image hint.\nUsage: /image <path> | /image clear".to_string()
            });
        emit_command_output(host, status);
        return Ok(CommandResult::Handled);
    }

    if args[0].eq_ignore_ascii_case("clear") {
        host.clear_pending_image_hint();
        emit_command_output(host, "Cleared pending image hint.");
        return Ok(CommandResult::Handled);
    }

    let path = args.join(" ").trim().to_string();
    if path.is_empty() {
        emit_command_output(host, "Usage: /image <path> | /image clear");
        return Ok(CommandResult::Handled);
    }
    let exists = Path::new(&path).exists();
    host.set_pending_image_hint(path.clone());
    if exists {
        emit_command_output(
            host,
            format!(
                "Image hint queued: `{}`.\nIt will be injected into the next prompt automatically.",
                path
            ),
        );
    } else {
        emit_command_output(
            host,
            format!(
                "Image hint queued: `{}` (path not found right now).\nIt will still be injected into the next prompt.",
                path
            ),
        );
    }
    Ok(CommandResult::Handled)
}

pub(crate) fn handle_interactive_question_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        emit_command_output(
            host,
            "Interactive question picker:\n\
             Usage: `/ask <question> | <option 1> | <option 2> [| <option 3> ...]`\n\
             Example: `/ask Proceed with deploy? | yes (recommended)::deploy now | no::pause and inspect logs`\n\
             In TUI mode this opens a native selection UI.\n\
             In non-TUI mode, provide your answer inline as normal text.",
        );
        return Ok(CommandResult::Handled);
    }

    let raw = args.join(" ");
    let segments: Vec<String> = raw
        .split('|')
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    if segments.len() < 2 {
        emit_command_output(
            host,
            "Interactive picker is available in TUI mode. For non-TUI usage provide options as `question | option1 | option2`.",
        );
        return Ok(CommandResult::Handled);
    }

    let question = segments[0].clone();
    let options = &segments[1..];
    let recommended = options
        .iter()
        .position(|opt| opt.to_ascii_lowercase().contains("recommended"))
        .unwrap_or(0);
    let selected = options
        .get(recommended)
        .map(|v| v.as_str())
        .unwrap_or("(none)");

    let mut out = String::new();
    let _ = writeln!(out, "Interactive question (non-TUI fallback)");
    let _ = writeln!(out, "Q: {}", question);
    let _ = writeln!(out, "Options:");
    for (idx, option) in options.iter().enumerate() {
        let marker = if idx == recommended {
            " (recommended)"
        } else {
            ""
        };
        let _ = writeln!(out, "  {}. {}{}", idx + 1, option, marker);
    }
    let _ = writeln!(out, "\nSelected: {}", selected);
    let _ = writeln!(
        out,
        "Tip: In TUI mode, `/ask ...` opens a selectable picker."
    );
    emit_command_output(host, out.trim_end());
    Ok(CommandResult::Handled)
}

pub(crate) fn handle_insights_command(
    host: &mut impl crate::app::SlashCommandHost,
) -> Result<CommandResult, AgentError> {
    let msg_count = host.messages().len();
    let user_count = host
        .messages()
        .iter()
        .filter(|m| m.role == hermes_core::MessageRole::User)
        .count();
    let assistant_count = host
        .messages()
        .iter()
        .filter(|m| m.role == hermes_core::MessageRole::Assistant)
        .count();
    emit_command_output(
        host,
        format!(
            "Session insights:\n  - Total messages: {}\n  - User messages: {}\n  - Hermes messages: {}\n  - Session: {}",
            msg_count,
            user_count,
            assistant_count,
            host.session_id()
        ),
    );
    Ok(CommandResult::Handled)
}

pub(crate) fn handle_log_command(
    host: &mut impl crate::app::SlashCommandHost,
) -> Result<CommandResult, AgentError> {
    let logs_dir = hermes_config::hermes_home().join("logs");
    let mut files = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(&logs_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files.reverse();
    if files.is_empty() {
        emit_command_output(
            host,
            format!("No log files found in {}", logs_dir.display()),
        );
        return Ok(CommandResult::Handled);
    }
    let mut out = format!("Recent log files in {}:\n", logs_dir.display());
    for path in files.into_iter().take(12) {
        let _ = writeln!(
            out,
            "  - {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    out.push_str("Use `hermes logs` for full tail output.");
    emit_command_output(host, out.trim_end());
    Ok(CommandResult::Handled)
}

pub(crate) fn handle_debug_dump_command(
    host: &mut impl crate::app::SlashCommandHost,
    _args: &[&str],
) -> Result<CommandResult, AgentError> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let prefix = host.session_id().chars().take(8).collect::<String>();
    let stem = format!("debug-{}-{}", prefix, stamp);
    let snapshot_path = host.persist_session_snapshot(Some(&stem))?;
    let logs_dir = hermes_config::hermes_home().join("logs");
    let log_files = std::fs::read_dir(&logs_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.filter_map(|entry| entry.ok()))
        .filter(|entry| entry.path().is_file())
        .count();
    let out = format!(
        "Debug snapshot written.\n  session_id: {}\n  model: {}\n  messages: {}\n  snapshot: {}\n  logs_dir: {} ({} files)\nTip: run `hermes debug share --local` for a support bundle.",
        host.session_id(),
        host.current_model(),
        host.messages().len(),
        snapshot_path.display(),
        logs_dir.display(),
        log_files
    );
    emit_command_output(host, out);
    Ok(CommandResult::Handled)
}

pub(crate) fn handle_dump_format_command(
    host: &mut impl crate::app::SlashCommandHost,
) -> Result<CommandResult, AgentError> {
    let mut out = String::new();
    let _ = writeln!(out, "Session snapshot format");
    let _ = writeln!(out, "  root keys: session_info, messages");
    let _ = writeln!(
        out,
        "  session_info keys: session_id, model, personality, message_count, created_at"
    );
    let _ = writeln!(
        out,
        "  message keys: role, content, tool_call_id, tool_calls, reasoning_content"
    );
    let _ = writeln!(
        out,
        "  save path: {}/sessions/<session-id>.json",
        host.state_root().display()
    );
    let _ = writeln!(out, "Use `/save [name]` to persist a snapshot now.");
    emit_command_output(host, out.trim_end());
    Ok(CommandResult::Handled)
}
