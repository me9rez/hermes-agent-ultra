//! `/curator` slash command handler and helpers.

use std::fmt::Write as _;

use crate::commands::{CommandResult, emit_command_output, truncate_chars};
use hermes_agent::RunConversationParams;
use hermes_core::{AgentError, MessageRole};

pub(crate) async fn handle_curator_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let skills_dir = hermes_config::hermes_home().join("skills");
    let store = hermes_skills::UsageStore::with_dir(skills_dir.clone());
    let curator_config = curator_config_from_app(host);

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();

    match sub.as_str() {
        "status" | "" => {
            let rows = store.agent_created_report();
            let state = hermes_skills::load_curator_state(&store);

            if rows.is_empty() {
                let mut out = String::from("No agent-created skills found.\n\n");
                out.push_str(&format!(
                    "curator: {}\n",
                    curator_status_label(&curator_config, &state)
                ));
                out.push_str(&format!(
                    "  interval: every {}h\n",
                    curator_config.interval_hours
                ));
                out.push_str(&format!(
                    "  stale after: {}d\n",
                    curator_config.stale_after_days
                ));
                out.push_str(&format!(
                    "  archive after: {}d\n",
                    curator_config.archive_after_days
                ));
                if let Some(countdown) = next_run_countdown(&state, &curator_config) {
                    out.push_str(&format!("  next run eligible: {}\n", countdown));
                }
                out.push_str(
                    "\nSkills created by the agent during background review will appear here.",
                );
                emit_command_output(host, &out);
            } else {
                let mut out = format!("## Agent-created skills ({})\n\n", rows.len());

                // Most active top 5
                let mut sorted_by_activity: Vec<_> = rows.iter().collect();
                sorted_by_activity.sort_by_key(|r| -(r.activity_count as i64));
                let top_active: Vec<_> = sorted_by_activity.iter().take(5).collect();
                if !top_active.is_empty() {
                    out.push_str("**Most active:**\n");
                    for row in &top_active {
                        let pin_mark = if row.pinned { "📌 " } else { "  " };
                        let _ = writeln!(
                            out,
                            "{}`{}` activity={} state={}",
                            pin_mark, row.name, row.activity_count, row.state
                        );
                    }
                    out.push('\n');
                }

                // All rows (with latest_active sort)
                out.push_str("### All agent-created skills\n\n");
                for row in &rows {
                    let pin_mark = if row.pinned { "📌 " } else { "  " };
                    let _ = writeln!(
                        out,
                        "{}`{}` activity={} state={}",
                        pin_mark, row.name, row.activity_count, row.state
                    );
                }

                out.push('\n');
                out.push_str(&format!(
                    "curator: {} interval: every {}h\n",
                    curator_status_label(&curator_config, &state),
                    curator_config.interval_hours
                ));
                if let Some(countdown) = next_run_countdown(&state, &curator_config) {
                    out.push_str(&format!("next run eligible: {}\n", countdown));
                }

                out.push_str(
                    "\nUse `/curator run` to run the curator manually.\nUse `/curator history` to view run history.",
                );
                emit_command_output(host, out.trim_end());
            }
        }
        "run" => {
            let dry_run = args
                .get(1)
                .is_some_and(|s| s.eq_ignore_ascii_case("--dry-run"));

            // Gate checks
            if !curator_config.enabled {
                emit_command_output(host, "Curator is disabled in config.");
                return Ok(CommandResult::Handled);
            }
            let state = hermes_skills::load_curator_state(&store);
            if state.paused {
                emit_command_output(host, "Curator is paused. Use `/curator unpause` to resume.");
                return Ok(CommandResult::Handled);
            }

            // Phase 1: deterministic auto-transitions (fast, milliseconds)
            let phase1 = hermes_skills::apply_automatic_transitions(&store, &curator_config);
            let mut out = format!(
                "── curator run ──\nPhase 1 — Auto transitions:\n│ checked: {} │ stale: {} │ archived: {} │ reactivated: {} │",
                phase1.checked, phase1.marked_stale, phase1.archived, phase1.reactivated
            );
            emit_command_output(host, &out);

            if dry_run {
                return Ok(CommandResult::Handled);
            }

            // Phase 2: LLM review (slow, 30-120s)
            // Build the curator prompt and spawn an LLM agent via llm_runner
            let prompt = hermes_skills::build_curator_prompt(&store);
            let agent = host.agent().clone();
            let tool_schemas = host.tool_schemas().to_vec();

            let llm_start = std::time::Instant::now();
            let llm_result: Result<
                hermes_skills::CuratorReviewResult,
                hermes_skills::CuratorError,
            > = {
                let conv_result = agent
                    .run_conversation(RunConversationParams {
                        user_message: prompt,
                        conversation_history: vec![],
                        task_id: None,
                        stream_callback: None,
                        persist_user_message: None,
                        tools: Some(tool_schemas),
                        persist_session: false,
                    })
                    .await;

                match conv_result {
                    Ok(conv) => extract_curator_review_result(conv, llm_start),
                    Err(e) => Err(hermes_skills::CuratorError::LlmError(format!(
                        "Agent conversation failed: {}",
                        e
                    ))),
                }
            };

            match llm_result {
                Ok(review) => {
                    let model_label = format!("{} / {}", review.model, review.provider);
                    let _ = write!(
                        out,
                        "\nPhase 2 — LLM review ({}):\n{}",
                        model_label,
                        truncate_chars(&review.final_response, 2000)
                    );
                    if !review.summary.is_empty() {
                        let _ = write!(out, "\nSummary: {}", truncate_chars(&review.summary, 500));
                    }
                    let _ = write!(out, "\nTool calls: {}", review.tool_calls.len());
                    emit_command_output(host, &out);

                    // Persist curator state with LLM summary
                    save_post_run_state(&store, Some(&review.summary));
                }
                Err(hermes_skills::CuratorError::LlmError(ref msg)) => {
                    let _ = write!(
                        out,
                        "\nPhase 2 — LLM review: skipped ({})",
                        truncate_chars(msg, 200)
                    );
                    emit_command_output(host, &out);

                    // Still persist Phase 1 state changes
                    save_post_run_state(&store, None);
                }
                Err(_) => {
                    let _ = write!(out, "\nPhase 2 — LLM review: skipped (unknown error)");
                    emit_command_output(host, &out);
                    save_post_run_state(&store, None);
                }
            }
        }
        "history" => {
            let state = hermes_skills::load_curator_state(&store);
            if state.run_count == 0 {
                emit_command_output(host, "No curator run history yet.");
            } else {
                let mut out = String::from("Curator run history\n\n");
                let _ = writeln!(out, "run_count: {}", state.run_count);
                if let Some(ref last) = state.last_run_at {
                    let _ = writeln!(out, "last_run_at: {}", last);
                }
                if let Some(ref summary) = state.last_run_summary {
                    let _ = writeln!(out, "last_summary: {}", truncate_chars(summary, 160));
                }
                emit_command_output(host, out.trim_end());
            }
        }
        "backup" => {
            let sub = args.get(1).map(|s| s.to_ascii_lowercase());
            match sub.as_deref() {
                Some("create") | None => match backup_skills(&skills_dir) {
                    Ok(path) => {
                        emit_command_output(host, format!("Backup created at {}", path.display()));
                    }
                    Err(e) => {
                        emit_command_output(host, format!("Backup failed: {}", e));
                    }
                },
                Some("list") => match list_backups(&skills_dir) {
                    Ok(backups) => {
                        if backups.is_empty() {
                            emit_command_output(host, "No curator backups found.");
                        } else {
                            let mut out = String::from("Curator backups\n");
                            for (name, _) in &backups {
                                let _ = writeln!(out, "- {}", name);
                            }
                            emit_command_output(host, out.trim_end());
                        }
                    }
                    Err(e) => {
                        emit_command_output(host, format!("Failed to list backups: {}", e));
                    }
                },
                Some("rollback") => {
                    let Some(backup_name) = args.get(2) else {
                        emit_command_output(host, "Usage: /curator backup rollback <backup-name>");
                        return Ok(CommandResult::Handled);
                    };
                    match rollback_skills(&skills_dir, backup_name) {
                        Ok(()) => {
                            emit_command_output(
                                host,
                                format!("Rolled back to backup `{}`.", backup_name),
                            );
                        }
                        Err(e) => {
                            emit_command_output(host, format!("Rollback failed: {}", e));
                        }
                    }
                }
                Some(other) => {
                    emit_command_output(
                        host,
                        format!(
                            "Unknown backup subcommand '{}'. Use create, list, or rollback.",
                            other
                        ),
                    );
                }
            }
        }
        other => {
            emit_command_output(
                host,
                format!(
                    "Unknown curator subcommand '{}'. Try: status, run, history, backup.",
                    other
                ),
            );
        }
    }
    Ok(CommandResult::Handled)
}

fn curator_config_from_app(host: &impl crate::app::ModelRuntime) -> hermes_skills::CuratorConfig {
    let gc = &host.config().curator;
    hermes_skills::CuratorConfig {
        enabled: gc.enabled,
        interval_hours: gc.interval_hours,
        min_idle_hours: gc.min_idle_hours,
        stale_after_days: gc.stale_after_days,
        archive_after_days: gc.archive_after_days,
        prune_builtins: gc.prune_builtins,
    }
}

fn curator_status_label(
    config: &hermes_skills::CuratorConfig,
    state: &hermes_skills::CuratorState,
) -> &'static str {
    if state.paused {
        "PAUSED"
    } else if config.enabled {
        "ENABLED"
    } else {
        "DISABLED"
    }
}

fn next_run_countdown(
    state: &hermes_skills::CuratorState,
    config: &hermes_skills::CuratorConfig,
) -> Option<String> {
    if !config.enabled || state.paused {
        return None;
    }
    let last = state.last_run_at.as_ref()?;
    let last_dt: chrono::DateTime<chrono::Utc> = last.parse().ok()?;
    let interval = chrono::Duration::seconds((config.interval_hours * 3600) as i64);
    let eligible = last_dt + interval;
    let now = chrono::Utc::now();
    if now >= eligible {
        Some("now".to_string())
    } else {
        let remaining = eligible - now;
        let hours = remaining.num_hours();
        let mins = (remaining.num_minutes() % 60).abs();
        if hours > 0 {
            Some(format!("in ~{}h {}m", hours, mins))
        } else {
            Some(format!("in ~{}m", mins))
        }
    }
}

#[allow(dead_code)]
fn build_curator_run_report(
    record: &hermes_skills::CuratorRunRecord,
    model: Option<String>,
    provider: Option<String>,
) -> hermes_skills::CuratorRunReport {
    let before_count = 0u64;
    let after_count = 0u64;
    let consolidated_count = 0u64;
    let pruned_count = 0u64;
    let transitions = record.auto_transitions.checked
        + record.auto_transitions.marked_stale
        + record.auto_transitions.archived
        + record.auto_transitions.reactivated;
    let tool_calls_total = record
        .llm_review
        .as_ref()
        .map_or(0, |r| r.tool_calls.len() as u64);

    hermes_skills::CuratorRunReport {
        started_at: record.started_at.clone(),
        duration_seconds: record.duration_seconds,
        model: model.or_else(|| record.model.clone()),
        provider: provider.or_else(|| record.provider.clone()),
        dry_run: record.dry_run,
        auto_transitions: record.auto_transitions.clone(),
        counts: hermes_skills::CuratorRunCounts {
            before: before_count,
            after: after_count,
            delta: (after_count as i64) - (before_count as i64),
            archived_this_run: record.auto_transitions.archived,
            consolidated_this_run: consolidated_count,
            pruned_this_run: pruned_count,
            state_transitions: transitions,
            tool_calls_total,
        },
        consolidated: vec![],
        pruned: vec![],
        tool_calls: record
            .llm_review
            .as_ref()
            .map_or(vec![], |r| r.tool_calls.clone()),
        llm_error: None,
    }
}

#[allow(dead_code)]
fn build_curator_run_report_from_transitions(
    result: &hermes_skills::TransitionResult,
) -> hermes_skills::CuratorRunReport {
    let transitions = result.checked + result.marked_stale + result.archived + result.reactivated;
    hermes_skills::CuratorRunReport {
        started_at: chrono::Utc::now().to_rfc3339(),
        duration_seconds: 0.0,
        model: None,
        provider: None,
        dry_run: false,
        auto_transitions: result.clone(),
        counts: hermes_skills::CuratorRunCounts {
            before: 0,
            after: 0,
            delta: 0,
            archived_this_run: result.archived,
            consolidated_this_run: 0,
            pruned_this_run: 0,
            state_transitions: transitions,
            tool_calls_total: 0,
        },
        consolidated: vec![],
        pruned: vec![],
        tool_calls: vec![],
        llm_error: None,
    }
}

fn backup_skills(skills_dir: &std::path::Path) -> Result<std::path::PathBuf, std::io::Error> {
    let backup_root = skills_dir.join(".curator_backups");
    std::fs::create_dir_all(&backup_root)?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let backup_dir = backup_root.join(&ts);

    if backup_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("backup directory already exists: {}", backup_dir.display()),
        ));
    }

    std::fs::create_dir_all(&backup_dir)?;
    for entry in std::fs::read_dir(skills_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".curator_backups"
            || name_str == ".archive"
            || name_str.starts_with(".curator_state")
        {
            continue;
        }
        let dest = backup_dir.join(&name);
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }

    tracing::info!("curator: backup created at {}", backup_dir.display());
    Ok(backup_dir)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dest = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

fn list_backups(
    skills_dir: &std::path::Path,
) -> Result<Vec<(String, std::path::PathBuf)>, std::io::Error> {
    let backup_root = skills_dir.join(".curator_backups");
    if !backup_root.exists() {
        return Ok(vec![]);
    }
    let mut backups = Vec::new();
    for entry in std::fs::read_dir(&backup_root)? {
        let entry = entry?;
        if entry.path().is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            backups.push((name, entry.path()));
        }
    }
    backups.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(backups)
}

fn rollback_skills(skills_dir: &std::path::Path, backup_name: &str) -> Result<(), std::io::Error> {
    let backup_dir = skills_dir.join(".curator_backups").join(backup_name);
    if !backup_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("backup not found: {}", backup_name),
        ));
    }

    for entry in std::fs::read_dir(skills_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".curator_backups"
            || name_str == ".archive"
            || name_str.starts_with(".curator_state")
        {
            continue;
        }
        if entry.path().is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }

    for entry in std::fs::read_dir(&backup_dir)? {
        let entry = entry?;
        let dest = skills_dir.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }

    tracing::info!("curator: rolled back to backup {}", backup_name);
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase 2 helpers
// ---------------------------------------------------------------------------

/// Extract `CuratorReviewResult` from an `AgentLoop::run_conversation` result.
fn extract_curator_review_result(
    conv: hermes_agent::ConversationResult,
    started: std::time::Instant,
) -> Result<hermes_skills::CuratorReviewResult, hermes_skills::CuratorError> {
    let duration = started.elapsed().as_secs_f64();

    // Collect tool calls from Assistant messages in the conversation
    let tool_calls: Vec<hermes_skills::ToolCallRecord> = conv
        .loop_result
        .messages
        .iter()
        .filter(|m| m.role == MessageRole::Assistant)
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .map(|tc| hermes_skills::ToolCallRecord {
            name: tc.function.name.clone(),
            arguments: tc.function.arguments.clone(),
        })
        .collect();

    // Final response: prefer ConversationResult::final_response, fallback to last Assistant message
    let final_response = conv.final_response.clone().unwrap_or_else(|| {
        conv.loop_result
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant && m.content.is_some())
            .and_then(|m| m.content.clone())
            .unwrap_or_default()
    });

    // Summary: take first 500 chars of final_response as fallback
    let summary = truncate_chars(&final_response, 500);

    let model = conv
        .loop_result
        .model
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let provider = conv
        .loop_result
        .provider
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    Ok(hermes_skills::CuratorReviewResult {
        final_response,
        summary,
        model,
        provider,
        tool_calls,
        error: None,
        duration_seconds: duration,
    })
}

/// Persist curator state after a run, optionally recording an LLM summary.
fn save_post_run_state(store: &hermes_skills::UsageStore, llm_summary: Option<&str>) {
    let mut state = hermes_skills::load_curator_state(store);
    state.run_count = state.run_count.saturating_add(1);
    state.last_run_at = Some(chrono::Utc::now().to_rfc3339());
    if let Some(summary) = llm_summary {
        if !summary.is_empty() {
            state.last_run_summary = Some(summary.to_string());
        }
    }
    if let Err(e) = hermes_skills::save_curator_state(store, &state) {
        tracing::warn!("Failed to persist curator state: {}", e);
    }
}
