//! Objective/steer/btw/handoff/subgoal/sethome command handlers.
//!
//! Extracted from `commands/mod.rs` as part of the module decomposition.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use hermes_core::AgentError;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::alpha_runtime::{
    ObjectiveLearningLedgerEntry, append_counterfactual, append_objective_learning_entry,
    build_objective_dag_from_contract, canonical_objective_behavior_mode,
    canonical_objective_lifecycle_status, clear_objective_contract, clear_objective_dag,
    clear_objective_learning_ledger, load_contextlattice_policy, load_objective_contract,
    load_objective_dag, load_objective_ensemble_policy, load_objective_eval_trend,
    load_objective_learning_ledger, load_objective_profile, load_objective_simulation_policy,
    objective_lifecycle_is_active, objective_profile_specialized_for,
    reset_objective_profile_generalized, set_contextlattice_policy_mode,
    set_objective_contract_behavior_mode, set_objective_contract_lifecycle_status,
    set_objective_ensemble_mode, set_objective_profile, set_objective_simulation_mode,
    summarize_objective_contract, upsert_objective_contract, utility_terms_from_contract,
};
use crate::commands::background;
use crate::commands::{CommandResult, emit_command_output, truncate_chars, yes_no};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const SESSION_STEER_PREFIX: &str = "[SESSION_STEER] ";
const HOME_SESSION_MARKER_FILE: &str = "home-session.json";
const SUBGOAL_DIR: &str = "subgoals";
const HANDOFF_REQUESTS_DIR: &str = "handoff_requests";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubgoalItem {
    text: String,
    status: String,
    created_at: String,
    updated_at: String,
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubgoalChecklist {
    session_id: String,
    objective: Option<String>,
    updated_at: String,
    items: Vec<SubgoalItem>,
}

impl SubgoalChecklist {
    fn for_session(session_id: &str, objective: Option<&str>) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            session_id: session_id.to_string(),
            objective: objective
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            updated_at: now,
            items: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn home_session_marker_path() -> PathBuf {
    hermes_config::hermes_home().join(HOME_SESSION_MARKER_FILE)
}

fn load_home_session_marker() -> Option<serde_json::Value> {
    let path = home_session_marker_path();
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

fn subgoal_checklist_path(session_id: &str) -> PathBuf {
    hermes_config::hermes_home()
        .join(SUBGOAL_DIR)
        .join(format!("{session_id}.json"))
}

fn load_subgoal_checklist(session_id: &str) -> Option<SubgoalChecklist> {
    let path = subgoal_checklist_path(session_id);
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

fn save_subgoal_checklist(checklist: &SubgoalChecklist) -> Result<PathBuf, AgentError> {
    let path = subgoal_checklist_path(&checklist.session_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", parent.display(), e)))?;
    }
    let body = serde_json::to_string_pretty(checklist)
        .map_err(|e| AgentError::Config(format!("serialize subgoal checklist: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", path.display(), e)))?;
    Ok(path)
}

fn render_subgoal_checklist(checklist: &SubgoalChecklist) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Subgoal checklist");
    let _ = writeln!(out, "session: {}", checklist.session_id);
    if let Some(objective) = checklist.objective.as_deref() {
        let _ = writeln!(out, "objective: {}", truncate_chars(objective, 200));
    }
    if checklist.items.is_empty() {
        out.push_str("items: (none)\n");
    } else {
        for (idx, item) in checklist.items.iter().enumerate() {
            let marker = match item.status.as_str() {
                "completed" => "[x]",
                "impossible" => "[!]",
                _ => "[ ]",
            };
            let _ = writeln!(
                out,
                "{} {}. {} ({})",
                marker,
                idx + 1,
                item.text,
                item.status
            );
        }
    }
    out.push_str(
        "\nUsage: /subgoal <text> | /subgoal complete <n> | /subgoal impossible <n> | /subgoal undo <n> | /subgoal remove <n> | /subgoal clear",
    );
    out.trim_end().to_string()
}

pub(crate) fn set_session_steer(
    host: &mut impl crate::app::SlashCommandHost,
    steer: Option<String>,
) {
    host.messages_mut().retain(|m| {
        if m.role != hermes_core::MessageRole::System {
            return true;
        }
        !m.content
            .as_deref()
            .unwrap_or_default()
            .starts_with(SESSION_STEER_PREFIX)
    });
    if let Some(steer_text) = steer
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        host.messages_mut()
            .push(hermes_core::Message::system(format!(
                "{SESSION_STEER_PREFIX}{steer_text}"
            )));
    }
}

pub(crate) fn current_session_steer(host: &impl crate::app::SessionRuntime) -> Option<String> {
    host.messages()
        .iter()
        .rev()
        .find(|m| m.role == hermes_core::MessageRole::System)
        .and_then(|m| m.content.as_deref())
        .and_then(|raw| raw.strip_prefix(SESSION_STEER_PREFIX))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------------------
// Objective lifecycle update
// ---------------------------------------------------------------------------

fn apply_objective_lifecycle_update(
    host: &mut impl crate::app::SlashCommandHost,
    raw_status: &str,
    reason: Option<&str>,
) -> Result<CommandResult, AgentError> {
    let reason_owned = reason
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let updated = set_objective_contract_lifecycle_status(raw_status, reason_owned.as_deref())?;
    let status = canonical_objective_lifecycle_status(&updated.lifecycle_status);
    let objective_injected = objective_lifecycle_is_active(&status);
    if objective_injected {
        host.set_session_objective(Some(updated.objective_text.clone()));
    } else {
        host.set_session_objective(None);
    }
    let _ = append_objective_learning_entry(ObjectiveLearningLedgerEntry {
        recorded_at: String::new(),
        objective_id: updated.id.clone(),
        objective_state: status.clone(),
        decision: format!("objective_status_{}", status),
        evidence_files: vec!["alpha/objective_contract.json".to_string()],
        evidence_commands: vec![format!("/objective lifecycle {}", status)],
        notes: format!(
            "Objective lifecycle set to {}. reason={}",
            status, updated.status_reason
        ),
    });
    let mut out = String::new();
    out.push_str("Objective lifecycle updated\n");
    out.push_str("-------------------------\n");
    let _ = writeln!(out, "objective_id={}", updated.id);
    let _ = writeln!(out, "status={}", status);
    let _ = writeln!(out, "reason={}", updated.status_reason);
    let _ = writeln!(out, "objective_injected={}", yes_no(objective_injected));
    let _ = writeln!(
        out,
        "behavior_mode={}",
        canonical_objective_behavior_mode(&updated.behavior_mode)
    );
    emit_command_output(host, out.trim_end());
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /objective handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_objective_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let objective_usage = "Usage: `/objective <text>` or `/objective status|verify|plan|constraints|counterfactual <scenario> | <expected_delta>|lifecycle [status|active|pause|resume|budget-limited|achieved|unmet]|behavior [status|list|balanced|strict|autonomous|mission|minimal]|profile [status|list|general|me|set <id>]|context [status|list|max|balanced|fast]|simulator [status|balanced|strict|aggressive]|ensemble [status|committee|single|debate]|ledger [status|tail [n]|clear]|dag [status|rebuild|clear]|eval [status|tail [n]]|clear`.";

    if let Some(first) = args.first() {
        let cmd = first.trim().to_ascii_lowercase();

        let lifecycle_alias = match cmd.as_str() {
            "pause" => Some("paused"),
            "resume" => Some("active"),
            "active" | "pursuing" => Some("active"),
            "budget" | "budget-limited" | "budget_limited" | "limited" => Some("budget_limited"),
            "achieved" | "complete" | "done" => Some("complete"),
            "unmet" | "failed" => Some("unmet"),
            _ => None,
        };
        if let Some(status) = lifecycle_alias {
            let reason = if args.len() > 1 {
                Some(args[1..].join(" "))
            } else {
                None
            };
            return apply_objective_lifecycle_update(host, status, reason.as_deref());
        }

        if cmd == "lifecycle" || cmd == "state" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            if sub == "status" || sub == "show" {
                let Some(contract) = load_objective_contract()? else {
                    emit_command_output(
                        host,
                        "No objective contract. Set one with `/objective <text>`.",
                    );
                    return Ok(CommandResult::Handled);
                };
                let status = canonical_objective_lifecycle_status(&contract.lifecycle_status);
                let objective_injected = objective_lifecycle_is_active(&status);
                let mut out = String::new();
                out.push_str("Objective lifecycle\n");
                out.push_str("-------------------\n");
                let _ = writeln!(out, "objective_id={}", contract.id);
                let _ = writeln!(out, "status={}", status);
                let _ = writeln!(out, "reason={}", contract.status_reason);
                let _ = writeln!(out, "objective_injected={}", yes_no(objective_injected));
                let _ = writeln!(
                    out,
                    "behavior_mode={}",
                    canonical_objective_behavior_mode(&contract.behavior_mode)
                );
                emit_command_output(host, out.trim_end());
                return Ok(CommandResult::Handled);
            }
            if sub == "list" {
                emit_command_output(
                    host,
                    "Lifecycle states:\n- active (alias: pursuing, resume)\n- paused (alias: pause)\n- budget_limited (alias: budget, limited)\n- complete (alias: achieved, done)\n- unmet (hard-blocked objective)",
                );
                return Ok(CommandResult::Handled);
            }
            if matches!(
                sub.as_str(),
                "active"
                    | "pursuing"
                    | "pause"
                    | "paused"
                    | "resume"
                    | "budget"
                    | "budget-limited"
                    | "budget_limited"
                    | "limited"
                    | "complete"
                    | "achieved"
                    | "done"
                    | "unmet"
                    | "failed"
            ) {
                let reason = if args.len() > 2 {
                    Some(args[2..].join(" "))
                } else {
                    None
                };
                return apply_objective_lifecycle_update(host, &sub, reason.as_deref());
            }
            emit_command_output(
                host,
                "Usage: /objective lifecycle [status|list|active|pause|resume|budget-limited|achieved|unmet] [reason...]",
            );
            return Ok(CommandResult::Handled);
        }

        if cmd == "behavior" || cmd == "mode" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            if sub == "status" || sub == "show" {
                let Some(contract) = load_objective_contract()? else {
                    emit_command_output(
                        host,
                        "No objective contract. Set one with `/objective <text>`.",
                    );
                    return Ok(CommandResult::Handled);
                };
                let mut out = String::new();
                out.push_str("Objective behavior mode\n");
                out.push_str("-----------------------\n");
                let _ = writeln!(out, "objective_id={}", contract.id);
                let _ = writeln!(
                    out,
                    "mode={}",
                    canonical_objective_behavior_mode(&contract.behavior_mode)
                );
                if !contract.behavior_directives.is_empty() {
                    out.push_str("directives:\n");
                    for directive in &contract.behavior_directives {
                        let _ = writeln!(out, "- {}", directive);
                    }
                }
                if !contract.success_criteria.is_empty() {
                    out.push_str("success_criteria:\n");
                    for criterion in &contract.success_criteria {
                        let _ = writeln!(out, "- {}", criterion);
                    }
                }
                emit_command_output(host, out.trim_end());
                return Ok(CommandResult::Handled);
            }
            if sub == "list" {
                emit_command_output(
                    host,
                    "Behavior modes:\n- balanced: generalized execution with evidence checkpoints\n- strict: strongest evidence-first + contradiction discipline\n- autonomous: proactive loop execution until blocked\n- mission (aliases: sigma, sota, perpetual): closed-loop perpetual objective improvement with concrete execution each cycle\n- minimal: concise operator-facing output with decisive actions",
                );
                return Ok(CommandResult::Handled);
            }
            let canonical_mode = canonical_objective_behavior_mode(&sub);
            if !matches!(
                canonical_mode.as_str(),
                "balanced" | "strict" | "autonomous" | "mission" | "minimal"
            ) {
                emit_command_output(
                    host,
                    "Usage: /objective behavior [status|list|balanced|strict|autonomous|mission|minimal|sigma|sota]",
                );
                return Ok(CommandResult::Handled);
            }
            let updated = set_objective_contract_behavior_mode(&sub)?;
            let _ = append_objective_learning_entry(ObjectiveLearningLedgerEntry {
                recorded_at: String::new(),
                objective_id: updated.id.clone(),
                objective_state: canonical_objective_lifecycle_status(&updated.lifecycle_status),
                decision: format!(
                    "objective_behavior_{}",
                    canonical_objective_behavior_mode(&updated.behavior_mode)
                ),
                evidence_files: vec!["alpha/objective_contract.json".to_string()],
                evidence_commands: vec![format!("/objective behavior {}", sub)],
                notes: "Objective behavior mode updated by operator command.".to_string(),
            });
            let mut out = String::new();
            out.push_str("Objective behavior updated\n");
            out.push_str("-------------------------\n");
            let _ = writeln!(out, "objective_id={}", updated.id);
            let _ = writeln!(
                out,
                "mode={}",
                canonical_objective_behavior_mode(&updated.behavior_mode)
            );
            out.push_str("directives:\n");
            for directive in &updated.behavior_directives {
                let _ = writeln!(out, "- {}", directive);
            }
            emit_command_output(host, out.trim_end());
            return Ok(CommandResult::Handled);
        }

        if cmd == "context" || cmd == "contextlattice" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            match sub.as_str() {
                "status" | "show" => {
                    let p = load_contextlattice_policy()?;
                    let mut out = String::new();
                    out.push_str("ContextLattice policy\n");
                    out.push_str("--------------------\n");
                    let _ = writeln!(out, "mode_hint: {}", p.preferred_retrieval_mode);
                    let _ = writeln!(out, "preflight_required: {}", p.preflight_required);
                    let _ = writeln!(
                        out,
                        "auto_context_pack_on_mission_start: {}",
                        p.auto_context_pack_on_mission_start
                    );
                    let _ = writeln!(
                        out,
                        "degradation_aware_planning: {}",
                        p.degradation_aware_planning
                    );
                    let _ = writeln!(
                        out,
                        "include_grounding_required: {}",
                        p.include_grounding_required
                    );
                    let _ = writeln!(
                        out,
                        "include_retrieval_debug_for_execution: {}",
                        p.include_retrieval_debug_for_execution
                    );
                    let _ = writeln!(
                        out,
                        "broaden_scope_on_zero_hits: {}",
                        p.broaden_scope_on_zero_hits
                    );
                    let _ = writeln!(
                        out,
                        "scoped_recency_pass_before_finalize: {}",
                        p.scoped_recency_pass_before_finalize
                    );
                    let _ = writeln!(
                        out,
                        "objective_analytics_writeback_required: {}",
                        p.objective_analytics_writeback_required
                    );
                    let _ = writeln!(
                        out,
                        "contradiction_check_across_layers: {}",
                        p.contradiction_check_across_layers
                    );
                    let _ = writeln!(
                        out,
                        "numeric_fact_verbatim_copy: {}",
                        p.numeric_fact_verbatim_copy
                    );
                    let _ = writeln!(
                        out,
                        "required_project_scoping: {}",
                        p.required_project_scoping
                    );
                    let _ = writeln!(
                        out,
                        "checkpoint_payload_requires_project_file_topic: {}",
                        p.checkpoint_payload_requires_project_file_topic
                    );
                    let _ = writeln!(
                        out,
                        "readback_verification_required: {}",
                        p.readback_verification_required
                    );
                    let _ = writeln!(
                        out,
                        "conflict_resolution_mode: {}",
                        p.conflict_resolution_mode
                    );
                    let _ = writeln!(
                        out,
                        "deep_retry_budget_secs: {:?}",
                        p.deep_retry_budget_secs
                    );
                    let _ = writeln!(
                        out,
                        "regular_retry_budget_secs: {:?}",
                        p.regular_retry_budget_secs
                    );
                    let _ = writeln!(
                        out,
                        "summary_sink_order: {}",
                        p.summary_sink_order.join(",")
                    );
                    emit_command_output(host, out.trim_end());
                    return Ok(CommandResult::Handled);
                }
                "list" => {
                    emit_command_output(
                        host,
                        "ContextLattice policy presets:\n- max: full evidence + deep retrieval + strict recency/readback gates\n- balanced: full evidence with moderate deep/regular retry budgets\n- fast: grounded but lower retrieval-debug overhead for speed-sensitive loops",
                    );
                    return Ok(CommandResult::Handled);
                }
                "max" | "strict" | "balanced" | "fast" | "speed" => {
                    let p = set_contextlattice_policy_mode(&sub)?;
                    emit_command_output(
                        host,
                        format!(
                            "ContextLattice policy updated.\nmode={} preflight={} retrieval_mode={} deep_retries={:?} regular_retries={:?}",
                            sub,
                            p.preflight_required,
                            p.preferred_retrieval_mode,
                            p.deep_retry_budget_secs,
                            p.regular_retry_budget_secs
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                _ => {
                    emit_command_output(
                        host,
                        "Usage: /objective context [status|list|max|balanced|fast]",
                    );
                    return Ok(CommandResult::Handled);
                }
            }
        }

        if cmd == "profile" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            match sub.as_str() {
                "status" | "show" => {
                    let p = load_objective_profile()?;
                    let mut out = String::new();
                    out.push_str("Objective profile\n");
                    out.push_str("-----------------\n");
                    let _ = writeln!(out, "profile_id: {}", p.profile_id);
                    let _ = writeln!(out, "operator_hint: {}", p.operator_hint);
                    let _ = writeln!(out, "default_shell: {}", p.default_shell);
                    let _ = writeln!(out, "memory_backend: {}", p.memory_backend);
                    let _ = writeln!(out, "specialization_note: {}", p.specialization_note);
                    if !p.preferred_repos.is_empty() {
                        out.push_str("preferred_repos:\n");
                        for repo in p.preferred_repos {
                            let _ = writeln!(out, "- {}", repo);
                        }
                    }
                    if !p.preferred_languages.is_empty() {
                        out.push_str("preferred_languages:\n");
                        for lang in p.preferred_languages {
                            let _ = writeln!(out, "- {}", lang);
                        }
                    }
                    emit_command_output(host, out.trim_end());
                    return Ok(CommandResult::Handled);
                }
                "list" => {
                    emit_command_output(
                        host,
                        "Objective profile presets:\n- repo-general: generalized defaults for any operator/repo\n- sheawinkler: specialized ContextLattice+zsh profile\n- operator-custom: generated when using `/objective profile set <name>`",
                    );
                    return Ok(CommandResult::Handled);
                }
                "general" | "repo-general" | "reset" => {
                    let profile = reset_objective_profile_generalized()?;
                    emit_command_output(
                        host,
                        format!(
                            "Objective profile reset to generalized defaults.\nprofile_id={} memory_backend={} shell={}",
                            profile.profile_id, profile.memory_backend, profile.default_shell
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                "me" | "sheawinkler" => {
                    let profile = set_objective_profile(objective_profile_specialized_for(
                        std::env::var("USER")
                            .unwrap_or_else(|_| "sheawinkler".to_string())
                            .as_str(),
                    ))?;
                    emit_command_output(
                        host,
                        format!(
                            "Objective profile specialized for operator.\nprofile_id={} memory_backend={} shell={}",
                            profile.profile_id, profile.memory_backend, profile.default_shell
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                "set" => {
                    let Some(name) = args.get(2) else {
                        emit_command_output(
                            host,
                            "Usage: /objective profile set <name> (or use /objective profile me|general)",
                        );
                        return Ok(CommandResult::Handled);
                    };
                    let profile = set_objective_profile(objective_profile_specialized_for(name))?;
                    emit_command_output(
                        host,
                        format!(
                            "Objective profile set.\nprofile_id={} operator_hint={} shell={} memory_backend={}",
                            profile.profile_id,
                            profile.operator_hint,
                            profile.default_shell,
                            profile.memory_backend
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                _ => {
                    emit_command_output(
                        host,
                        "Usage: /objective profile [status|list|general|me|set <id>]",
                    );
                    return Ok(CommandResult::Handled);
                }
            }
        }

        if cmd == "simulator" || cmd == "simulation" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            if sub == "status" || sub == "show" {
                let p = load_objective_simulation_policy()?;
                emit_command_output(
                    host,
                    format!(
                        "Objective simulation policy\nmode={}\nrequire_shadow_pass={}\nmin_shadow_samples={}\nrequire_replay_validation={}\nmax_live_capital_fraction={:.4}\nupdated_at={}",
                        p.mode,
                        p.require_shadow_pass,
                        p.min_shadow_samples,
                        p.require_replay_validation,
                        p.max_live_capital_fraction,
                        p.updated_at
                    ),
                );
                return Ok(CommandResult::Handled);
            }
            if !matches!(sub.as_str(), "balanced" | "strict" | "aggressive") {
                emit_command_output(
                    host,
                    "Usage: /objective simulator [status|balanced|strict|aggressive]",
                );
                return Ok(CommandResult::Handled);
            }
            let p = set_objective_simulation_mode(&sub)?;
            emit_command_output(
                host,
                format!(
                    "Objective simulation policy updated.\nmode={} shadow_pass={} replay_validation={} max_live_capital_fraction={:.4}",
                    p.mode,
                    p.require_shadow_pass,
                    p.require_replay_validation,
                    p.max_live_capital_fraction
                ),
            );
            return Ok(CommandResult::Handled);
        }

        if cmd == "ensemble" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            if sub == "status" || sub == "show" {
                let p = load_objective_ensemble_policy()?;
                emit_command_output(
                    host,
                    format!(
                        "Objective ensemble policy\nmode={}\narbitration={}\nmin_voters={}\nrequire_disagreement_explainer={}\nallow_fast_path_single_model={}\nupdated_at={}",
                        p.mode,
                        p.arbitration,
                        p.min_voters,
                        p.require_disagreement_explainer,
                        p.allow_fast_path_single_model,
                        p.updated_at
                    ),
                );
                return Ok(CommandResult::Handled);
            }
            if !matches!(sub.as_str(), "committee" | "single" | "debate") {
                emit_command_output(
                    host,
                    "Usage: /objective ensemble [status|committee|single|debate]",
                );
                return Ok(CommandResult::Handled);
            }
            let p = set_objective_ensemble_mode(&sub)?;
            emit_command_output(
                host,
                format!(
                    "Objective ensemble policy updated.\nmode={} arbitration={} min_voters={} disagreement_explainer={}",
                    p.mode, p.arbitration, p.min_voters, p.require_disagreement_explainer
                ),
            );
            return Ok(CommandResult::Handled);
        }

        if cmd == "ledger" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            if sub == "clear" {
                clear_objective_learning_ledger()?;
                emit_command_output(host, "Objective learning ledger cleared.");
                return Ok(CommandResult::Handled);
            }
            let ledger = load_objective_learning_ledger()?;
            if sub == "status" || sub == "show" {
                let last = ledger
                    .entries
                    .last()
                    .map(|v| format!("{} {} {}", v.recorded_at, v.objective_state, v.decision))
                    .unwrap_or_else(|| "none".to_string());
                emit_command_output(
                    host,
                    format!(
                        "Objective learning ledger\nentries={}\nupdated_at={}\nlast_entry={}",
                        ledger.entries.len(),
                        ledger.updated_at,
                        last
                    ),
                );
                return Ok(CommandResult::Handled);
            }
            if sub == "tail" {
                let n = args
                    .get(2)
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(8)
                    .clamp(1, 64);
                let mut out = String::new();
                out.push_str("Objective learning ledger tail\n");
                out.push_str("-----------------------------\n");
                let start = ledger.entries.len().saturating_sub(n);
                for row in &ledger.entries[start..] {
                    let _ = writeln!(
                        out,
                        "- {} id={} state={} decision={} notes={}",
                        row.recorded_at,
                        row.objective_id,
                        row.objective_state,
                        row.decision,
                        row.notes
                    );
                }
                if ledger.entries.is_empty() {
                    out.push_str("(empty)\n");
                }
                emit_command_output(host, out.trim_end());
                return Ok(CommandResult::Handled);
            }
            emit_command_output(host, "Usage: /objective ledger [status|tail [n]|clear]");
            return Ok(CommandResult::Handled);
        }

        if cmd == "dag" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            if sub == "rebuild" || sub == "build" {
                let dag = build_objective_dag_from_contract()?;
                emit_command_output(
                    host,
                    format!(
                        "Objective DAG rebuilt.\nobjective_id={}\nnodes={}\nauto_resume_checkpoint={}",
                        dag.objective_id,
                        dag.nodes.len(),
                        dag.auto_resume_checkpoint
                    ),
                );
                return Ok(CommandResult::Handled);
            }
            if sub == "clear" {
                clear_objective_dag()?;
                emit_command_output(host, "Objective DAG cleared.");
                return Ok(CommandResult::Handled);
            }
            let dag = load_objective_dag()?;
            let mut out = String::new();
            out.push_str("Objective DAG\n");
            out.push_str("-------------\n");
            let _ = writeln!(out, "objective_id: {}", dag.objective_id);
            let _ = writeln!(out, "updated_at: {}", dag.updated_at);
            let _ = writeln!(
                out,
                "auto_resume_checkpoint: {}",
                dag.auto_resume_checkpoint
            );
            if dag.nodes.is_empty() {
                out.push_str("nodes: (empty)\n");
            } else {
                for node in dag.nodes {
                    let _ = writeln!(
                        out,
                        "- {} [{}] depends_on=[{}] rollback={}",
                        node.id,
                        node.status,
                        node.depends_on.join(","),
                        node.rollback
                    );
                    let _ = writeln!(out, "  title: {}", node.title);
                }
            }
            emit_command_output(host, out.trim_end());
            return Ok(CommandResult::Handled);
        }

        if cmd == "eval" {
            let sub = args
                .get(1)
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "status".to_string());
            let trend = load_objective_eval_trend()?;
            if sub == "tail" {
                let n = args
                    .get(2)
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(12)
                    .clamp(1, 100);
                let start = trend.samples.len().saturating_sub(n);
                let mut out = String::new();
                out.push_str("Objective eval trend tail\n");
                out.push_str("------------------------\n");
                for sample in &trend.samples[start..] {
                    let _ = writeln!(
                        out,
                        "- {} id={} state={} score={:.3} note={}",
                        sample.recorded_at,
                        sample.objective_id,
                        sample.objective_state,
                        sample.score,
                        sample.note
                    );
                }
                if trend.samples.is_empty() {
                    out.push_str("(empty)\n");
                }
                emit_command_output(host, out.trim_end());
                return Ok(CommandResult::Handled);
            }
            let latest = trend.samples.last().map(|s| s.score).unwrap_or(0.0);
            let avg = if trend.samples.is_empty() {
                0.0
            } else {
                trend.samples.iter().map(|s| s.score).sum::<f64>() / trend.samples.len() as f64
            };
            emit_command_output(
                host,
                format!(
                    "Objective eval trend\nsamples={}\nlatest_score={:.3}\navg_score={:.3}\nupdated_at={}",
                    trend.samples.len(),
                    latest,
                    avg,
                    trend.updated_at
                ),
            );
            return Ok(CommandResult::Handled);
        }

        if cmd == "verify" {
            let Some(contract) = load_objective_contract()? else {
                emit_command_output(
                    host,
                    "No objective contract. Set one with `/objective <text>` before verify.",
                );
                return Ok(CommandResult::Handled);
            };
            let trend = load_objective_eval_trend()?;
            let ledger = load_objective_learning_ledger()?;
            let latest = trend.samples.last().map(|s| s.score).unwrap_or(0.0);
            let prev = if trend.samples.len() >= 2 {
                trend
                    .samples
                    .get(trend.samples.len().saturating_sub(2))
                    .map(|s| s.score)
                    .unwrap_or(latest)
            } else {
                latest
            };
            let delta = latest - prev;
            let avg = if trend.samples.is_empty() {
                0.0
            } else {
                trend.samples.iter().map(|s| s.score).sum::<f64>() / trend.samples.len() as f64
            };
            let ledger_tail = ledger.entries.last();
            let last_ledger_state = ledger_tail
                .map(|entry| entry.objective_state.as_str())
                .unwrap_or("unknown");
            let has_contract = !contract.id.trim().is_empty();
            let outcome = if !has_contract {
                "unproven"
            } else if latest >= 0.75 && delta >= -0.01 {
                "advancing"
            } else if latest <= 0.35 || delta < -0.05 {
                "regressing"
            } else if trend.samples.len() < 2 {
                "unproven"
            } else {
                "flat"
            };
            let mut evidence_files: Vec<String> = Vec::new();
            let mut verified_existing = 0usize;
            if let Some(last_assistant) = host
                .messages()
                .iter()
                .rev()
                .find(|m| m.role == hermes_core::MessageRole::Assistant)
                .and_then(|m| m.content.as_deref())
            {
                if let Ok(path_re) = Regex::new(r"path=([^\s]+)") {
                    for cap in path_re.captures_iter(last_assistant) {
                        if let Some(path) = cap.get(1).map(|m| m.as_str().trim()) {
                            if path.is_empty() {
                                continue;
                            }
                            if !evidence_files.iter().any(|v| v == path) {
                                let exists = Path::new(path).exists();
                                if exists {
                                    verified_existing += 1;
                                }
                                evidence_files.push(path.to_string());
                            }
                        }
                    }
                }
            }
            let mut out = String::new();
            out.push_str("Objective outcome verifier\n");
            out.push_str("-------------------------\n");
            let _ = writeln!(out, "objective_id={}", contract.id);
            let _ = writeln!(out, "objective_state={}", outcome);
            let _ = writeln!(out, "latest_score={:.3}", latest);
            let _ = writeln!(out, "delta_vs_prev={:+.3}", delta);
            let _ = writeln!(out, "avg_score={:.3}", avg);
            let _ = writeln!(out, "trend_samples={}", trend.samples.len());
            let _ = writeln!(out, "ledger_entries={}", ledger.entries.len());
            let _ = writeln!(out, "ledger_last_state={}", last_ledger_state);
            let _ = writeln!(out, "verified_files_present={}", verified_existing);
            let _ = writeln!(out, "verified_files_total={}", evidence_files.len());
            if evidence_files.is_empty() {
                let _ = writeln!(
                    out,
                    "note=no PATCH_VERIFIED path markers found in last assistant turn; file verification is unproven."
                );
            } else {
                out.push_str("verified_paths:\n");
                for path in evidence_files.iter().take(12) {
                    let _ = writeln!(
                        out,
                        "- {} exists_now={}",
                        path,
                        yes_no(Path::new(path).exists())
                    );
                }
            }
            emit_command_output(host, out.trim_end());
            return Ok(CommandResult::Handled);
        }

        if cmd == "status" || cmd == "show" {
            let mut out = String::new();
            match host.session_objective() {
                Some(v) => {
                    let _ = writeln!(out, "Current objective:\n{}", v);
                }
                None => {
                    let _ = writeln!(out, "No session objective set.");
                }
            }
            if let Some(contract) = load_objective_contract()? {
                let _ = writeln!(out, "\nObjective contract");
                let _ = writeln!(out, "------------------");
                let _ = writeln!(out, "{}", summarize_objective_contract(&contract));
                let _ = writeln!(
                    out,
                    "status_reason: {}",
                    if contract.status_reason.trim().is_empty() {
                        "(none)"
                    } else {
                        contract.status_reason.trim()
                    }
                );
                if !contract.behavior_directives.is_empty() {
                    let _ = writeln!(
                        out,
                        "behavior_directives: {}",
                        contract.behavior_directives.join(" | ")
                    );
                }
            } else {
                let _ = writeln!(out, "\nNo persisted objective contract yet.");
            }
            if let Ok(profile) = load_objective_profile() {
                let _ = writeln!(
                    out,
                    "\nObjective profile\n-----------------\nprofile_id: {}\noperator_hint: {}\nmemory_backend: {}\ndefault_shell: {}",
                    profile.profile_id,
                    profile.operator_hint,
                    profile.memory_backend,
                    profile.default_shell
                );
            }
            if let Ok(ctx_policy) = load_contextlattice_policy() {
                let _ = writeln!(
                    out,
                    "\nContextLattice policy\n---------------------\nmode_hint: {}\npreflight_required: {}\nretrieval_debug: {}\nreadback_required: {}\ndeep_retries: {:?}\nregular_retries: {:?}",
                    ctx_policy.preferred_retrieval_mode,
                    ctx_policy.preflight_required,
                    ctx_policy.include_retrieval_debug_for_execution,
                    ctx_policy.readback_verification_required,
                    ctx_policy.deep_retry_budget_secs,
                    ctx_policy.regular_retry_budget_secs
                );
            }
            if let Ok(sim) = load_objective_simulation_policy() {
                let _ = writeln!(
                    out,
                    "\nSimulation policy\n-----------------\nmode: {} (shadow_pass={} replay_validation={} cap={:.4})",
                    sim.mode,
                    sim.require_shadow_pass,
                    sim.require_replay_validation,
                    sim.max_live_capital_fraction
                );
            }
            if let Ok(ensemble) = load_objective_ensemble_policy() {
                let _ = writeln!(
                    out,
                    "\nEnsemble policy\n---------------\nmode: {} (arbitration={} min_voters={})",
                    ensemble.mode, ensemble.arbitration, ensemble.min_voters
                );
            }
            emit_command_output(host, out.trim_end());
            return Ok(CommandResult::Handled);
        }

        if cmd == "plan" {
            let Some(contract) = load_objective_contract()? else {
                emit_command_output(
                    host,
                    "No objective contract. Set one with `/objective <text>`.",
                );
                return Ok(CommandResult::Handled);
            };
            let mut out = String::new();
            out.push_str("Objective horizon plan\n");
            out.push_str("----------------------\n");
            for horizon in contract.horizons {
                let _ = writeln!(out, "- {}:", horizon.horizon);
                for goal in horizon.goals {
                    let _ = writeln!(out, "  - {}", goal);
                }
            }
            let terms = utility_terms_from_contract()?;
            if !terms.is_empty() {
                let mut rows: Vec<(String, f64)> = terms.into_iter().collect();
                rows.sort_by(|a, b| b.1.total_cmp(&a.1));
                out.push_str("\nUtility weights:\n");
                for (name, weight) in rows {
                    let _ = writeln!(out, "- {}: {:.2}", name, weight);
                }
            }
            emit_command_output(host, out.trim_end());
            return Ok(CommandResult::Handled);
        }

        if cmd == "constraints" {
            let Some(contract) = load_objective_contract()? else {
                emit_command_output(
                    host,
                    "No objective contract. Set one with `/objective <text>`.",
                );
                return Ok(CommandResult::Handled);
            };
            let mut out = String::new();
            out.push_str("Objective hard constraints\n");
            out.push_str("--------------------------\n");
            for c in contract.utility.hard_constraints {
                let _ = writeln!(out, "- {}", c.expression);
            }
            emit_command_output(host, out.trim_end());
            return Ok(CommandResult::Handled);
        }

        if cmd == "counterfactual" {
            if args.len() < 2 {
                emit_command_output(
                    host,
                    "Usage: /objective counterfactual <scenario> | <expected_delta>",
                );
                return Ok(CommandResult::Handled);
            }
            let joined = args[1..].join(" ");
            let (scenario, expected_delta) = joined
                .split_once('|')
                .map(|(a, b)| (a.trim(), b.trim()))
                .unwrap_or((joined.trim(), "impact not specified"));
            if scenario.is_empty() {
                emit_command_output(
                    host,
                    "Counterfactual scenario cannot be empty. Use: /objective counterfactual <scenario> | <expected_delta>",
                );
                return Ok(CommandResult::Handled);
            }
            let updated = append_counterfactual(scenario, expected_delta)?;
            emit_command_output(
                host,
                format!(
                    "Counterfactual saved (journal entries={}).",
                    updated.counterfactual_journal.len()
                ),
            );
            return Ok(CommandResult::Handled);
        }
    }

    if args.is_empty() {
        let msg = match host.session_objective() {
            Some(v) => format!(
                "Current objective:\n{}\n\nUse `/objective clear` to remove, `/objective <text>` to replace, or `/objective status` for contract details.",
                v
            ),
            None => format!("No objective set.\n{}", objective_usage),
        };
        emit_command_output(host, msg);
        return Ok(CommandResult::Handled);
    }

    let first = args[0].trim();
    if first.eq_ignore_ascii_case("clear")
        || first.eq_ignore_ascii_case("off")
        || first.eq_ignore_ascii_case("none")
        || first.eq_ignore_ascii_case("reset")
    {
        let previous_id = load_objective_contract()?
            .map(|c| c.id)
            .unwrap_or_else(|| "none".to_string());
        host.set_session_objective(None);
        clear_objective_contract()?;
        let _ = append_objective_learning_entry(ObjectiveLearningLedgerEntry {
            recorded_at: String::new(),
            objective_id: previous_id,
            objective_state: "cleared".to_string(),
            decision: "objective_clear".to_string(),
            evidence_files: vec![],
            evidence_commands: vec!["/objective clear".to_string()],
            notes: "Objective contract cleared by operator command.".to_string(),
        });
        emit_command_output(host, "Session objective cleared.");
        return Ok(CommandResult::Handled);
    }

    let objective = args.join(" ").trim().to_string();
    if objective.is_empty() {
        emit_command_output(host, objective_usage);
        return Ok(CommandResult::Handled);
    }
    let objective_lc = objective.to_ascii_lowercase();
    let trading_sensitive = [
        "trading", "sol", "kraken", "wallet", "pnl", "strategy", "market",
    ]
    .iter()
    .any(|needle| objective_lc.contains(needle));
    let contract = upsert_objective_contract(&objective, trading_sensitive)?;
    let _ = build_objective_dag_from_contract();
    let lifecycle = canonical_objective_lifecycle_status(&contract.lifecycle_status);
    if objective_lifecycle_is_active(&lifecycle) {
        host.set_session_objective(Some(objective.clone()));
    } else {
        host.set_session_objective(None);
    }
    let _ = append_objective_learning_entry(ObjectiveLearningLedgerEntry {
        recorded_at: String::new(),
        objective_id: contract.id.clone(),
        objective_state: lifecycle.clone(),
        decision: "objective_set".to_string(),
        evidence_files: vec!["alpha/objective_contract.json".to_string()],
        evidence_commands: vec!["/objective <text>".to_string()],
        notes: if trading_sensitive {
            "Trading-sensitive objective configured.".to_string()
        } else {
            "General objective configured.".to_string()
        },
    });
    emit_command_output(
        host,
        format!(
            "Session objective set:\n{}\n\nObjective contract persisted:\n{}\n\nlifecycle_status={}\nbehavior_mode={}\nobjective_injected={}\n\nThis objective is now injected as system context for future turns when lifecycle is active.",
            objective,
            summarize_objective_contract(&contract),
            lifecycle,
            canonical_objective_behavior_mode(&contract.behavior_mode),
            yes_no(objective_lifecycle_is_active(&lifecycle))
        ),
    );
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /steer handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_steer_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        let message = current_session_steer(host).map_or_else(
            || "No active steering instruction. Use `/steer <instruction>`.".to_string(),
            |v| format!("Active steering instruction:\n{}", v),
        );
        emit_command_output(host, message);
        return Ok(CommandResult::Handled);
    }

    if args[0].eq_ignore_ascii_case("clear") {
        set_session_steer(host, None);
        emit_command_output(host, "Cleared session steering instruction.");
        return Ok(CommandResult::Handled);
    }

    let steer = args.join(" ");
    set_session_steer(host, Some(steer.clone()));
    emit_command_output(
        host,
        format!(
            "Steering instruction set.\nThis is injected as system context on subsequent turns.\n\n{}",
            steer.trim()
        ),
    );
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /btw handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_btw_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        emit_command_output(
            host,
            "Usage: /btw <side-question>\nRuns an ephemeral side-question as a background task.",
        );
        return Ok(CommandResult::Handled);
    }
    let question = args.join(" ").trim().to_string();
    if question.is_empty() {
        emit_command_output(host, "Usage: /btw <side-question>");
        return Ok(CommandResult::Handled);
    }
    let task = format!(
        "Ephemeral side question (do not alter objective/contracts unless explicitly asked): {}",
        question
    );
    let job = background::queue_background_job(&task)?;
    emit_command_output(
        host,
        format!(
            "[/btw queued]\nQuestion: {}\nJob ID: {}\nStatus: {}\nLogs:   {}",
            question,
            job.id,
            job.status_path.display(),
            job.log_path.display()
        ),
    );
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /handoff handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_handoff_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let mut configured: Vec<_> = host.config().platforms.keys().cloned().collect();
    configured.sort();
    if args.is_empty() {
        let configured_text = if configured.is_empty() {
            "(none configured)".to_string()
        } else {
            configured.join(", ")
        };
        emit_command_output(
            host,
            format!(
                "Usage: /handoff <platform>\nConfigured platforms: {}\nThis queues a handoff request under ~/.hermes-agent-ultra/handoff_requests for gateway pickup.",
                configured_text
            ),
        );
        return Ok(CommandResult::Handled);
    }

    let platform = args[0].trim().to_ascii_lowercase();
    let Some(platform_cfg) = host.config().platforms.get(&platform) else {
        emit_command_output(
            host,
            format!(
                "Unknown platform '{}'. Configured platforms: {}",
                platform,
                if configured.is_empty() {
                    "(none configured)".to_string()
                } else {
                    configured.join(", ")
                }
            ),
        );
        return Ok(CommandResult::Handled);
    };
    if !platform_cfg.enabled {
        emit_command_output(
            host,
            format!(
                "Platform '{}' is configured but disabled. Enable it in config.yaml before handoff.",
                platform
            ),
        );
        return Ok(CommandResult::Handled);
    }

    let home_channel = platform_cfg
        .home_channel
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            load_home_session_marker().and_then(|value| {
                value
                    .get("home")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .filter(|v| !v.trim().is_empty())
            })
        });
    let Some(home_channel) = home_channel else {
        emit_command_output(
            host,
            format!(
                "No home channel marker for '{}'. Run `/sethome <channel-or-thread>` first, then retry `/handoff {}`.",
                platform, platform
            ),
        );
        return Ok(CommandResult::Handled);
    };

    let dir = hermes_config::hermes_home().join(HANDOFF_REQUESTS_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", dir.display(), e)))?;
    let request_path = dir.join(format!("{}-{}.json", host.session_id(), platform));
    let payload = serde_json::json!({
        "session_id": host.session_id(),
        "platform": platform,
        "home_channel": home_channel,
        "requested_at": chrono::Utc::now().to_rfc3339(),
        "requested_by": "cli",
        "state": "pending",
    });
    std::fs::write(
        &request_path,
        serde_json::to_string_pretty(&payload)
            .map_err(|e| AgentError::Config(format!("serialize handoff request: {e}")))?,
    )
    .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", request_path.display(), e)))?;

    emit_command_output(
        host,
        format!(
            "Queued handoff request.\n  session: {}\n  platform: {}\n  home_channel: {}\n  request_file: {}\n\nGateway workers can pick this up immediately when running.",
            host.session_id(),
            payload["platform"].as_str().unwrap_or_default(),
            payload["home_channel"].as_str().unwrap_or_default(),
            request_path.display(),
        ),
    );
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /subgoal handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_subgoal_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let objective = host
        .session_objective()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut checklist = load_subgoal_checklist(host.session_id())
        .unwrap_or_else(|| SubgoalChecklist::for_session(host.session_id(), objective));
    checklist.objective = objective.map(ToOwned::to_owned);

    if args.is_empty()
        || matches!(
            args[0].to_ascii_lowercase().as_str(),
            "show" | "status" | "list"
        )
    {
        checklist.updated_at = chrono::Utc::now().to_rfc3339();
        let _ = save_subgoal_checklist(&checklist)?;
        emit_command_output(host, render_subgoal_checklist(&checklist));
        return Ok(CommandResult::Handled);
    }

    let action = args[0].to_ascii_lowercase();
    if action == "clear" {
        checklist.items.clear();
        checklist.updated_at = chrono::Utc::now().to_rfc3339();
        let path = save_subgoal_checklist(&checklist)?;
        emit_command_output(
            host,
            format!(
                "Subgoal checklist cleared.\nPath: {}\nUse `/subgoal <text>` to add a new item.",
                path.display()
            ),
        );
        return Ok(CommandResult::Handled);
    }

    if matches!(
        action.as_str(),
        "complete" | "done" | "impossible" | "undo" | "remove"
    ) {
        let Some(raw_idx) = args.get(1) else {
            emit_command_output(host, format!("Usage: /subgoal {} <n>", action));
            return Ok(CommandResult::Handled);
        };
        let Ok(idx_one_based) = raw_idx.trim().parse::<usize>() else {
            emit_command_output(
                host,
                format!(
                    "/subgoal {}: <n> must be an integer (1-based index).",
                    action
                ),
            );
            return Ok(CommandResult::Handled);
        };
        if idx_one_based == 0 || idx_one_based > checklist.items.len() {
            emit_command_output(
                host,
                format!(
                    "/subgoal {}: index {} is out of range (1..={}).",
                    action,
                    idx_one_based,
                    checklist.items.len()
                ),
            );
            return Ok(CommandResult::Handled);
        }
        let idx = idx_one_based - 1;
        let now = chrono::Utc::now().to_rfc3339();

        if action == "remove" {
            let removed = checklist.items.remove(idx);
            checklist.updated_at = now;
            let _ = save_subgoal_checklist(&checklist)?;
            emit_command_output(
                host,
                format!(
                    "Removed subgoal {}: {}\n\n{}",
                    idx_one_based,
                    removed.text,
                    render_subgoal_checklist(&checklist)
                ),
            );
            return Ok(CommandResult::Handled);
        }

        checklist.items[idx].status = match action.as_str() {
            "complete" | "done" => "completed".to_string(),
            "impossible" => "impossible".to_string(),
            "undo" => "pending".to_string(),
            _ => checklist.items[idx].status.clone(),
        };
        checklist.items[idx].updated_at = now.clone();
        checklist.updated_at = now;
        let _ = save_subgoal_checklist(&checklist)?;
        emit_command_output(
            host,
            format!(
                "Updated subgoal {} -> {}\n\n{}",
                idx_one_based,
                checklist.items[idx].status,
                render_subgoal_checklist(&checklist)
            ),
        );
        return Ok(CommandResult::Handled);
    }

    let text = args.join(" ").trim().to_string();
    if text.is_empty() {
        emit_command_output(host, "Usage: /subgoal <text>");
        return Ok(CommandResult::Handled);
    }
    let now = chrono::Utc::now().to_rfc3339();
    checklist.items.push(SubgoalItem {
        text,
        status: "pending".to_string(),
        created_at: now.clone(),
        updated_at: now.clone(),
        source: "user".to_string(),
    });
    checklist.updated_at = now;
    let _ = save_subgoal_checklist(&checklist)?;
    emit_command_output(host, render_subgoal_checklist(&checklist));
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// /sethome handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_sethome_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let marker_path = home_session_marker_path();
    if args.is_empty() || args[0].eq_ignore_ascii_case("status") {
        if let Some(marker) = load_home_session_marker() {
            emit_command_output(
                host,
                format!(
                    "Home marker file: {}\n{}",
                    marker_path.display(),
                    serde_json::to_string_pretty(&marker).unwrap_or_else(|_| "{}".to_string())
                ),
            );
        } else {
            emit_command_output(
                host,
                format!(
                    "No home marker set. Use `/sethome <name>`.\nMarker path: {}",
                    marker_path.display()
                ),
            );
        }
        return Ok(CommandResult::Handled);
    }

    if args[0].eq_ignore_ascii_case("clear") {
        if marker_path.exists() {
            std::fs::remove_file(&marker_path).map_err(|e| {
                AgentError::Io(format!("Failed to remove {}: {}", marker_path.display(), e))
            })?;
            emit_command_output(host, "Cleared home marker.");
        } else {
            emit_command_output(host, "Home marker already clear.");
        }
        return Ok(CommandResult::Handled);
    }

    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", parent.display(), e)))?;
    }
    let value = serde_json::json!({
        "session_id": host.session_id(),
        "home": args.join(" ").trim(),
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(
        &marker_path,
        serde_json::to_string_pretty(&value)
            .map_err(|e| AgentError::Config(format!("serialize home marker: {}", e)))?,
    )
    .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", marker_path.display(), e)))?;
    emit_command_output(
        host,
        format!(
            "Home marker updated.\nPath: {}\nHome: {}",
            marker_path.display(),
            args.join(" ").trim()
        ),
    );
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::app::App;
    use crate::app::SessionRuntime;
    use crate::commands::{SLASH_COMMANDS, autocomplete, canonical_command};
    use crate::test_env_lock;
    use clap::Parser;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

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
        let mut host = App::new(cli).await.expect("build host");
        let (tx, _rx) = mpsc::unbounded_channel::<crate::tui::Event>();
        host.set_stream_handle(Some(tx.into()));
        host
    }

    fn latest_ui_assistant_text(host: &impl crate::app::SessionRuntime) -> String {
        host.ui_messages()
            .iter()
            .rev()
            .find(|row| row.message.role == hermes_core::MessageRole::Assistant)
            .and_then(|row| row.message.content.clone())
            .unwrap_or_default()
    }

    #[test]
    fn test_objective_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/objective"));
        let results = autocomplete("/obj");
        assert!(results.contains(&"/objective"));
    }

    #[test]
    fn test_handoff_and_subgoal_commands_are_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/handoff"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/subgoal"));
        let handoff_results = autocomplete("/han");
        assert!(handoff_results.contains(&"/handoff"));
        let subgoal_results = autocomplete("/sub");
        assert!(subgoal_results.contains(&"/subgoal"));
    }

    #[test]
    fn test_goal_alias_maps_to_objective() {
        assert_eq!(canonical_command("/goal"), "/objective");
    }

    #[tokio::test]
    async fn promoted_steer_command_sets_and_clears_instruction() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut host = build_test_app_with_stream(tmp.path()).await;

        handle_steer_command(&mut host, &["focus", "on", "repo", "map"]).expect("set steer");
        assert_eq!(
            current_session_steer(&host).as_deref(),
            Some("focus on repo map")
        );
        assert!(latest_ui_assistant_text(&host).contains("Steering instruction set."));

        handle_steer_command(&mut host, &["clear"]).expect("clear steer");
        assert!(current_session_steer(&host).is_none());
        assert!(latest_ui_assistant_text(&host).contains("Cleared session steering instruction."));
    }

    #[tokio::test]
    async fn promoted_btw_command_queues_ephemeral_background_task() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut host = build_test_app_with_stream(tmp.path()).await;

        let result =
            handle_btw_command(&mut host, &["why", "is", "latency", "high?"]).expect("btw command");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&host);
        assert!(output.contains("[/btw queued]"));
        assert!(output.contains("Question: why is latency high?"));
    }

    #[tokio::test]
    async fn promoted_sethome_command_sets_status_and_clears_marker() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut host = build_test_app_with_stream(tmp.path()).await;

        handle_sethome_command(&mut host, &["alpha-room"]).expect("set home");
        assert!(latest_ui_assistant_text(&host).contains("Home marker updated."));
        let marker = load_home_session_marker().expect("home marker");
        assert_eq!(
            marker.get("home").and_then(|v| v.as_str()),
            Some("alpha-room")
        );

        handle_sethome_command(&mut host, &["status"]).expect("home status");
        assert!(latest_ui_assistant_text(&host).contains("Home marker file:"));

        handle_sethome_command(&mut host, &["clear"]).expect("home clear");
        assert!(latest_ui_assistant_text(&host).contains("Cleared home marker."));
        assert!(load_home_session_marker().is_none());
    }

    #[tokio::test]
    async fn objective_lifecycle_pause_resume_updates_session_injection() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut host = build_test_app_with_stream(tmp.path()).await;

        let set_result = crate::commands::handle_slash_command(
            &mut host,
            "/objective",
            &["stabilize", "indexing"],
        )
        .await
        .expect("set objective");
        assert_eq!(set_result, CommandResult::Handled);
        assert_eq!(host.session_objective(), Some("stabilize indexing"));

        let pause_result = crate::commands::handle_slash_command(
            &mut host,
            "/objective",
            &["pause", "manual", "hold"],
        )
        .await
        .expect("pause objective");
        assert_eq!(pause_result, CommandResult::Handled);
        assert!(host.session_objective().is_none());
        assert!(latest_ui_assistant_text(&host).contains("status=paused"));

        let resume_result =
            crate::commands::handle_slash_command(&mut host, "/objective", &["resume", "continue"])
                .await
                .expect("resume objective");
        assert_eq!(resume_result, CommandResult::Handled);
        assert_eq!(host.session_objective(), Some("stabilize indexing"));
        assert!(latest_ui_assistant_text(&host).contains("status=active"));
    }

    #[tokio::test]
    async fn objective_behavior_mode_can_be_switched() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut host = build_test_app_with_stream(tmp.path()).await;

        crate::commands::handle_slash_command(
            &mut host,
            "/objective",
            &["improve", "planner", "quality"],
        )
        .await
        .expect("set objective");

        let mode_result =
            crate::commands::handle_slash_command(&mut host, "/objective", &["behavior", "strict"])
                .await
                .expect("set behavior");
        assert_eq!(mode_result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&host);
        assert!(output.contains("mode=strict"));
        assert!(output.contains("directives:"));

        let mission_result =
            crate::commands::handle_slash_command(&mut host, "/objective", &["behavior", "sigma"])
                .await
                .expect("set behavior sigma");
        assert_eq!(mission_result, CommandResult::Handled);
        let mission_output = latest_ui_assistant_text(&host);
        assert!(mission_output.contains("mode=mission"));
    }

    #[tokio::test]
    async fn promoted_subgoal_command_supports_add_update_and_clear() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut host = build_test_app_with_stream(tmp.path()).await;
        host.set_session_objective(Some("stabilize alpha".to_string()));

        handle_subgoal_command(&mut host, &["inspect", "wallet", "drift"]).expect("subgoal add");
        let output = latest_ui_assistant_text(&host);
        assert!(output.contains("Subgoal checklist"));
        assert!(output.contains("inspect wallet drift"));
        assert!(output.contains("[ ] 1."));

        handle_subgoal_command(&mut host, &["complete", "1"]).expect("subgoal complete");
        let output = latest_ui_assistant_text(&host);
        assert!(output.contains("Updated subgoal 1 -> completed"));
        assert!(output.contains("[x] 1."));

        handle_subgoal_command(&mut host, &["clear"]).expect("subgoal clear");
        let output = latest_ui_assistant_text(&host);
        assert!(output.contains("Subgoal checklist cleared."));
    }

    #[tokio::test]
    async fn promoted_handoff_command_surfaces_usage_and_unknown_platform() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut host = build_test_app_with_stream(tmp.path()).await;

        handle_handoff_command(&mut host, &[]).expect("handoff usage");
        let usage = latest_ui_assistant_text(&host);
        assert!(usage.contains("Usage: /handoff <platform>"));

        handle_handoff_command(&mut host, &["not-a-real-platform"])
            .expect("handoff unknown platform");
        let unknown = latest_ui_assistant_text(&host);
        assert!(unknown.contains("Unknown platform 'not-a-real-platform'"));
    }
}
