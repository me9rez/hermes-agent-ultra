//! Slash command handler (Requirement 9.2).
//!
//! Defines and dispatches all supported `/` commands in the interactive
//! REPL, and provides auto-completion suggestions.

use std::process::Stdio;
use std::sync::Arc;
use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    time::SystemTime,
};

use bytes::Bytes;
use hermes_agent::{
    RunConversationParams, plugins::PluginManifest, split_messages_for_run_conversation,
};
use hermes_core::AgentError;
use hermes_intelligence::{SwarmExecutionMode, build_swarm_execution_plan, swarm_runtime_status};
use hermes_skills;
use hermes_tools::ToolPolicyEngine;
use hermes_tools::tools::messaging::MessagingSessionContext;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::alpha_runtime::{
    ObjectiveLearningLedgerEntry, append_counterfactual, append_objective_learning_entry,
    build_objective_dag_from_contract, canonical_objective_behavior_mode,
    canonical_objective_lifecycle_status, clear_objective_contract, clear_objective_dag,
    clear_objective_learning_ledger, enqueue_loop_event, ensure_alpha_runtime_bootstrap,
    ensure_trading_runtime_bootstrap, load_alpha_loops, load_claim_verifier_policy,
    load_contextlattice_policy, load_last_trading_alpha_report, load_objective_contract,
    load_objective_dag, load_objective_ensemble_policy, load_objective_eval_trend,
    load_objective_learning_ledger, load_objective_profile, load_objective_simulation_policy,
    load_quorum_policy, objective_lifecycle_is_active, objective_profile_specialized_for,
    recover_orphan_loop_events, refresh_trading_alpha_report, render_mission_board,
    render_trading_alpha_board, replay_loop_queue, reset_objective_profile_generalized,
    set_claim_verifier_enabled, set_contextlattice_policy_mode,
    set_objective_contract_behavior_mode, set_objective_contract_lifecycle_status,
    set_objective_ensemble_mode, set_objective_profile, set_objective_simulation_mode,
    set_quorum_policy, summarize_objective_contract, upsert_objective_contract,
    utility_terms_from_contract,
};
pub(crate) mod auth_cmd;
pub(crate) mod background;
pub(crate) mod browser;
pub(crate) mod compress;
pub(crate) mod kanban;
pub(crate) mod model;
pub(crate) mod objective;
pub(crate) mod policy;
pub(crate) mod session;
pub mod skills;

// Re-export background items still referenced from outside
pub use background::recover_queued_background_jobs;
pub use kanban::run_kanban_command;

// Re-export model utilities still referenced from this module
use model::{
    ModelCapabilityRequirements, default_client, rank_catalog_model_candidates,
    resolve_catalog_model_candidate, resolve_model_capabilities, split_provider_model,
    unmet_model_requirements,
};

use crate::app::{App, PetDock, PetSettings};
use crate::kanban::{
    KanbanActionInput, KanbanBoard, KanbanLane, NewKanbanTaskInput, add_task, archive_done,
    claim_task, create_or_select_board, ensure_board, find_task_mut, lane_counts, load_store,
    maybe_checkpoint_to_contextlattice, move_task, save_store, set_blocked,
};
use crate::model_switch::{curated_provider_slugs, normalize_provider_model, provider_model_ids};
use crate::pairing_store::{PairingStatus, PairingStore};
use crate::skin_engine::{BUILTIN_SKINS, canonical_skin_name};
use hermes_config::{GatewayConfig, LlmProviderConfig};

// ---------------------------------------------------------------------------
// CommandResult
// ---------------------------------------------------------------------------

/// Result of handling a slash command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandResult {
    /// The command was fully handled (no further action needed).
    Handled,
    /// The command requires the agent to process a follow-up message.
    NeedsAgent,
    /// The user requested to quit the application.
    Quit,
}

fn secret_stdout_allowed() -> bool {
    std::env::var("HERMES_ALLOW_SECRET_STDOUT")
        .ok()
        .is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn mask_secret_value(secret: &str) -> String {
    if secret.is_empty() {
        return "(empty)".to_string();
    }
    if secret.len() <= 8 {
        return "*".repeat(secret.len());
    }
    format!(
        "{}***{}",
        &secret[..4],
        &secret[secret.len().saturating_sub(4)..]
    )
}

// ---------------------------------------------------------------------------
// Slash commands
// ---------------------------------------------------------------------------

/// All supported slash commands and their descriptions.
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/new", "Start a new session"),
    (
        "/reset",
        "Start a new session (alias of /new; fresh session ID + history)",
    ),
    (
        "/clear",
        "Clear screen/session state and start a fresh session",
    ),
    ("/retry", "Retry the last user message"),
    ("/undo", "Undo the last exchange"),
    ("/history", "Show recent conversation history"),
    (
        "/recap",
        "Summarize recent session activity (`/recap [count]`)",
    ),
    (
        "/context",
        "Context breakdown (`status|breakdown|compress`)",
    ),
    ("/title", "Set or show session title metadata"),
    ("/topic", "Session topic metadata controls"),
    (
        "/branch",
        "Create a branch/fork marker for the current session",
    ),
    ("/fork", "Alias for /branch"),
    (
        "/timetravel",
        "Session time-travel controls (`list|latest|goto <snapshot>|undo [n]|branch [label]`)",
    ),
    ("/tt", "Alias for /timetravel"),
    ("/snapshot", "Create/list snapshot checkpoints"),
    ("/snap", "Alias for /snapshot"),
    ("/rollback", "List rollback checkpoints"),
    (
        "/model",
        "Show/switch models, run capability diagnostics (`/model explain`, `why-not`, `harness`, `backend`), or configure failover (`/model failover`)",
    ),
    (
        "/auth",
        "Auth lifecycle controls (`status|verify|refresh`) for active provider credentials",
    ),
    ("/provider", "List configured providers and availability"),
    (
        "/personality",
        "Show current personality, list built-ins, or switch mode",
    ),
    ("/profile", "Show active profile and Hermes home path"),
    ("/whoami", "Alias for /profile"),
    ("/fast", "Toggle fast-mode hints"),
    ("/skin", "Show available skin/theme options"),
    ("/skins", "Alias for /skin"),
    ("/voice", "Show voice mode status"),
    (
        "/pet",
        "Animated companion controls (`status|on|off|toggle|list|set|mood|dock|speed`)",
    ),
    ("/skills", "List available skills"),
    ("/skill", "Alias for /skills"),
    (
        "/curator",
        "Skill curator/control-plane compatibility surface",
    ),
    ("/tools", "List registered tools"),
    (
        "/toolcards",
        "Inline tool-card controls (e.g. `/toolcards export`)",
    ),
    ("/toolsets", "Show configured toolsets by platform"),
    ("/plugins", "List plugin bundles and status"),
    ("/mcp", "List configured MCP servers"),
    ("/reload", "Reload runtime env/config values"),
    ("/reload-skills", "Refresh installed skill index/registry"),
    ("/reload_skills", "Alias for /reload-skills"),
    ("/reload-mcp", "Reload MCP server metadata"),
    ("/reload_mcp", "Alias for /reload-mcp"),
    ("/cron", "Show cron scheduler status"),
    ("/scheduler", "Alias for /background"),
    (
        "/agents",
        "Show active/background task state (`status|pause|resume|doctor`)",
    ),
    ("/tasks", "Alias for /kanban"),
    ("/queue", "Queue a follow-up prompt"),
    ("/q", "Alias for /queue"),
    (
        "/handoff",
        "Queue a session handoff request to a configured gateway platform (`/handoff <platform>`)",
    ),
    (
        "/evolve",
        "Run or inspect the self-evolution intelligence loop",
    ),
    (
        "/subgoal",
        "Objective checklist controls (`show|<text>|complete|impossible|undo|remove|clear`)",
    ),
    (
        "/objective",
        "Set/show objective contract + profile/policies (`status|verify|plan|constraints|counterfactual|profile|context|simulator|ensemble|ledger|dag|eval|clear`)",
    ),
    (
        "/claims",
        "Claim verifier controls (`status|on|off`) for verified/inferred/unproven final tagging",
    ),
    (
        "/quorum",
        "Optional multi-voter deep-reasoning mode (`status|on|off|models|run`)",
    ),
    (
        "/swarm",
        "Swarm orchestration surface (`status|plan|run|cancel|artifact`) with quorum-compatible controls",
    ),
    ("/swarms", "Alias for /swarm"),
    (
        "/simulate",
        "Simulate tool-policy decisions without executing tools (`status|<tool> [json-params]`)",
    ),
    (
        "/specpatch",
        "Speculative patch executor (`/specpatch <verify_cmd> | <candidate_cmd_1> | ...`)",
    ),
    (
        "/heatmap",
        "Context coverage heatmap for repo files (`/heatmap [repo-path]`)",
    ),
    (
        "/studio",
        "Replay studio (`/studio replay status|verify|diff <export_a.json> <export_b.json>`)",
    ),
    ("/goal", "Alias for /objective"),
    (
        "/ask",
        "Open interactive question picker (`/ask <question> | <option 1> | <option 2> ...`)",
    ),
    ("/question", "Alias for /ask"),
    ("/steer", "Inject non-interrupt steering instruction"),
    ("/btw", "Run an ephemeral side-question"),
    (
        "/plan",
        "Queue planning work or inspect planner queue status (`/plan caps ...`, `/plan depth ...`)",
    ),
    ("/lsp", "Show code-index/LSP context status and controls"),
    (
        "/graph",
        "Show graph-memory, ContextLattice status, and embedding diagnostics",
    ),
    (
        "/qos",
        "Provider QoS router controls (`status|health|autotune [plan|apply]`)",
    ),
    (
        "/image",
        "Attach/clear an image hint consumed by next prompt",
    ),
    ("/config", "Show or modify configuration"),
    (
        "/autocompact",
        "Show auto-compaction status (`/autocompact status|now|governance`)",
    ),
    ("/autocompress", "Alias for /autocompact"),
    ("/compress", "Trigger context compression"),
    ("/compact", "Alias for /compress"),
    ("/clear-queue", "Clear queued background jobs"),
    ("/usage", "Show token usage statistics"),
    ("/insights", "Show local usage/session insights"),
    ("/stop", "Stop current agent execution"),
    ("/busy", "Busy/processing status compatibility surface"),
    (
        "/kanban",
        "Task board controls (`status|boards|init|use|add|move|claim|block|done|archive-done|dispatch|sync`)",
    ),
    ("/status", "Show session status (model, turns, token count)"),
    ("/agent", "Alias for /status"),
    (
        "/about",
        "Show build/parity/upstream snapshot and enabled Ultra features",
    ),
    ("/ops", "Operator control plane (status + quick controls)"),
    (
        "/telemetry",
        "Live telemetry snapshot (`status|lane`) for runtime health and gate signals",
    ),
    (
        "/runbook",
        "Failure-first remediation runbooks (`list|show <name>`)",
    ),
    (
        "/eval",
        "Run/show live session evaluation harness (`status|run|latest`)",
    ),
    (
        "/autopilot",
        "Adaptive intelligence-performance autopilot (`status|run|recommend|apply|profile|mode|clear`)",
    ),
    (
        "/mission",
        "Mission control board (`status|init|recover|replay|enqueue|trading ...`)",
    ),
    ("/dashboard", "Dashboard control (status|on|off|url)"),
    (
        "/platforms",
        "Show enabled gateway/messaging platform adapters",
    ),
    ("/gateway", "Alias for /platforms"),
    (
        "/integrations",
        "Integration control plane (`status|auth|providers|gateway|memory|all|repair|snapshot`)",
    ),
    ("/commands", "Show categorized slash command catalog"),
    (
        "/boot",
        "Startup readiness gate (`status|quick|profile`) with pass/warn/fail remediation",
    ),
    (
        "/walkthrough",
        "Guided onboarding walkthrough (`status|start|next|done|reset|insights`)",
    ),
    (
        "/triage",
        "External trigger triage (`status|list|eval|queue|feedback`)",
    ),
    (
        "/subconscious",
        "Background subconscious queue (`status|add|approve|reject|run|profile|clear`)",
    ),
    ("/log", "Show recent runtime log files"),
    ("/debug", "Generate local debug-report guidance"),
    ("/debug-dump", "Write local session diagnostics snapshot"),
    ("/dump-format", "Show concrete transcript snapshot schema"),
    ("/experiment", "Set/clear experiment steering context"),
    ("/feedback", "Record feedback note into local logs"),
    ("/copy", "Copy latest assistant message (if supported)"),
    ("/paste", "Attach clipboard payload (if supported)"),
    ("/gquota", "Show Google quota hint (if configured)"),
    ("/sethome", "Set home channel/session marker"),
    ("/set-home", "Alias for /sethome"),
    ("/restart", "Restart current interactive session"),
    ("/approve", "Approve pending action (gateway mode)"),
    ("/deny", "Deny pending action (gateway mode)"),
    ("/update", "Run update checker and report status"),
    ("/save", "Save current session to disk"),
    ("/load", "Load a saved session"),
    ("/resume", "Resume the most recent or named saved session"),
    (
        "/sessions",
        "Browse saved sessions, or resume one by name (`/sessions [name]`)",
    ),
    (
        "/background",
        "Run a task in the background (`status|tail <job-id> [N]`)",
    ),
    ("/bg", "Alias for /background"),
    ("/mouse", "Toggle mouse interactions in the TUI"),
    ("/verbose", "Toggle verbose mode"),
    ("/statusbar", "Toggle status bar visibility"),
    ("/footer", "Footer visibility compatibility surface"),
    ("/indicator", "Status indicator compatibility surface"),
    ("/sb", "Alias for /statusbar"),
    ("/yolo", "Toggle auto-approve mode"),
    (
        "/browser",
        "Manage local Chrome CDP bridge (`status|connect [ws/http-url]|disconnect`)",
    ),
    ("/redraw", "Force a local repaint pulse in the TUI"),
    (
        "/reasoning",
        "Reasoning controls (display + effort: status/on/off/set <low|medium|high|xhigh>)",
    ),
    (
        "/raw",
        "RTK raw-mode controls + deterministic trace controls (status/on/off/toggle/once/trace with tail/verify/export/path)",
    ),
    (
        "/policy",
        "Runtime policy profiles (`status|list|strict|standard|dev`) + live counters",
    ),
    ("/help", "Show help for available commands"),
    (
        "/acp_server",
        "ACP server (auto-start if not running; or start|stop|status|restart|connections)",
    ),
    ("/quit", "Quit the application"),
    ("/exit", "Alias for /quit"),
    ("/onboard", "Alias for /walkthrough"),
];

const DEFAULT_SKILL_TAPS: &[&str] = &[
    "https://github.com/NousResearch/hermes-agent::skills",
    "https://github.com/NousResearch/hermes-agent::optional-skills",
    "https://github.com/openai/skills::skills",
    "https://github.com/anthropics/skills::skills",
    "https://github.com/VoltAgent/awesome-agent-skills::skills",
    "https://github.com/mattpocock/skills::skills",
    "https://github.com/github/awesome-copilot::skills",
    "https://github.com/garrytan/gstack::",
    "https://github.com/MiniMax-AI/cli::skill",
];

const GITHUB_API_BASE: &str = "https://api.github.com";
const OFFICIAL_SKILLS_REPO: &str = "nousresearch/hermes-agent";
const HERMES_SKILLS_INDEX_URL: &str =
    "https://hermes-agent.nousresearch.com/docs/api/skills-index.json";
const SKILLS_SH_SEARCH_URL: &str = "https://skills.sh/api/search";
const CLAWHUB_API_BASE: &str = "https://clawhub.ai/api/v1";
const SKILLS_HUB_STATE_DIR: &str = hermes_skills::HUB_STATE_DIR;
const SKILLS_HUB_AUDIT_FILE: &str = "audit.log";
const SENTRUX_MCP_SERVER_NAME: &str = "sentrux";
const SENTRUX_MCP_COMMAND: &str = "sentrux";
const SENTRUX_MCP_ARG: &str = "--mcp";
const SKILL_BOOTSTRAP_ALLOWED_EXECUTABLES: &[&str] = &[
    "bash", "sh", "python", "python3", "pip", "pip3", "pipx", "uv", "uvx", "node", "npm", "npx",
    "pnpm", "yarn", "bun", "cargo", "rustup", "go", "make", "cmake", "git", "brew", "apt",
    "apt-get", "dnf", "yum", "pacman", "zypper", "apk",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillTapSpec {
    repo: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSkillSource {
    repo: String,
    branch: String,
    skill_dir: String,
}

#[derive(Debug, Clone)]
enum RegistryInstallSource {
    GitHub(ResolvedSkillSource),
    LobeHub {
        slug: String,
    },
    ClawHub {
        slug: String,
        version: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct RegistrySkillRecord {
    identifier: String,
    description: String,
    source: String,
    score: i32,
    install_source: RegistryInstallSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallFallbackSource {
    SkillsSh,
    Tap,
}

type SkillHubInstalledEntry = hermes_skills::SkillHubInstalledEntry;
type SkillsHubLockFile = hermes_skills::SkillsHubLock;

#[derive(Debug, Clone)]
struct SkillInstallProvenance {
    source: String,
    identifier: String,
    trust_level: String,
    metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
struct SkillBootstrapPlan {
    commands: Vec<String>,
}

#[derive(Debug, Clone)]
struct ParsedBootstrapCommand {
    display: String,
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HermesSkillsIndexResponse {
    #[serde(default)]
    skills: Vec<HermesSkillsIndexEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct HermesSkillsIndexEntry {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    identifier: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    resolved_github_id: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SkillsShSearchResponse {
    #[serde(default)]
    skills: Vec<SkillsShSearchEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillsShSearchEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    #[serde(rename = "skillId")]
    skill_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LobeHubMeta {
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
struct LobeHubAgentResponse {
    #[serde(default)]
    author: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    meta: LobeHubMeta,
    #[serde(default)]
    config: LobeHubConfig,
}

#[derive(Debug, Default, Deserialize)]
struct LobeHubConfig {
    #[serde(default)]
    #[serde(rename = "systemRole")]
    system_role: String,
}

#[derive(Debug, Deserialize)]
struct ClawHubSkillDetailResponse {
    #[serde(default)]
    #[serde(rename = "latestVersion")]
    latest_version: ClawHubLatestVersion,
}

#[derive(Debug, Default, Deserialize)]
struct ClawHubLatestVersion {
    #[serde(default)]
    version: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRepoInfo {
    default_branch: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct GitHubTreeResponse {
    tree: Vec<GitHubTreeEntry>,
}

fn parse_skill_tap_spec(raw: &str) -> Option<SkillTapSpec> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (base, override_path) = if let Some((lhs, rhs)) = trimmed.split_once("::") {
        (lhs.trim(), Some(rhs.trim()))
    } else {
        (trimmed, None)
    };

    let (repo, mut path) = if let Some(rest) = base
        .strip_prefix("https://github.com/")
        .or_else(|| base.strip_prefix("http://github.com/"))
    {
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() < 2 {
            return None;
        }
        let path = if segments.len() >= 5 && segments[2] == "tree" {
            segments[4..].join("/")
        } else {
            "skills".to_string()
        };
        (format!("{}/{}", segments[0], segments[1]), path)
    } else {
        let segments: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() < 2 {
            return None;
        }
        let path = if segments.len() > 2 {
            segments[2..].join("/")
        } else {
            "skills".to_string()
        };
        (format!("{}/{}", segments[0], segments[1]), path)
    };

    if let Some(override_path) = override_path {
        path = override_path.to_string();
    }

    Some(SkillTapSpec {
        repo,
        path: path.trim_matches('/').to_string(),
    })
}

fn parse_skill_name_and_version(spec: &str) -> (String, Option<String>) {
    let trimmed = spec.trim();
    if let Some((name, version)) = trimmed.rsplit_once('@') {
        if !name.is_empty() && !version.is_empty() && !name.starts_with("https://") {
            return (name.to_string(), Some(version.to_string()));
        }
    }
    (trimmed.to_string(), None)
}

fn looks_like_github_repo_slug(token: &str) -> bool {
    let parts: Vec<&str> = token.split('/').filter(|s| !s.is_empty()).collect();
    parts.len() == 2
}

fn parse_explicit_github_skill(spec: &str) -> Option<(String, Option<String>, String)> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Registry-prefixed identifiers (official/..., skills.sh/..., etc.)
    // must not be treated as direct GitHub owner/repo/path slugs.
    if parse_registry_prefixed_skill(trimmed).is_some() {
        return None;
    }

    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() < 2 {
            return None;
        }
        let repo = format!("{}/{}", segments[0], segments[1]);
        if segments.len() >= 5 && segments[2] == "tree" {
            let branch = segments[3].to_string();
            let path = segments[4..].join("/");
            if path.is_empty() {
                return None;
            }
            return Some((repo, Some(branch), path));
        }
        if segments.len() > 2 {
            let path = segments[2..].join("/");
            if path.is_empty() {
                return None;
            }
            return Some((repo, None, path));
        }
        return None;
    }

    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() >= 3 {
        let repo = format!("{}/{}", segments[0], segments[1]);
        let path = segments[2..].join("/");
        if path.is_empty() {
            return None;
        }
        return Some((repo, None, path));
    }

    None
}

fn sanitize_skill_install_name(source: &str) -> String {
    let raw = source
        .trim()
        .split('/')
        .next_back()
        .unwrap_or(source)
        .trim();
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch);
        } else if out.ends_with('_') {
            continue;
        } else {
            out.push('_');
        }
    }
    let normalized = out.trim_matches('_').to_string();
    if normalized.is_empty() {
        "skill".to_string()
    } else {
        normalized
    }
}

fn ensure_safe_relative_path(path: &str) -> Result<(), AgentError> {
    if path.is_empty() {
        return Err(AgentError::Config("Empty path in skill bundle.".into()));
    }
    if path.starts_with('/') || path.contains('\\') {
        return Err(AgentError::Config(format!(
            "Unsafe path in skill bundle: {}",
            path
        )));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(AgentError::Config(format!(
                "Unsafe path segment in skill bundle: {}",
                path
            )));
        }
    }
    Ok(())
}

fn parse_registry_prefixed_skill(spec: &str) -> Option<(String, String)> {
    let (prefix, rest) = spec.split_once('/')?;
    let normalized = prefix.trim().to_ascii_lowercase();
    let source = match normalized.as_str() {
        "official" => "official",
        "github" => "github",
        "skills.sh" | "skills-sh" => "skills.sh",
        "lobehub" => "lobehub",
        "clawhub" => "clawhub",
        "claude-marketplace" => "claude-marketplace",
        _ => return None,
    };
    let key = rest.trim();
    if key.is_empty() {
        return None;
    }
    Some((source.to_string(), key.to_string()))
}

fn score_registry_match(entry: &HermesSkillsIndexEntry, query: &str) -> i32 {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return 0;
    }

    let name = entry.name.to_ascii_lowercase();
    let id = entry.identifier.to_ascii_lowercase();
    let desc = entry.description.to_ascii_lowercase();
    let tags = entry
        .tags
        .iter()
        .map(|t| t.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    if id == q || name == q {
        return 1000;
    }
    if id.starts_with(&q) || name.starts_with(&q) {
        return 900;
    }
    if id.contains(&q) || name.contains(&q) {
        return 700;
    }
    if tags.contains(&q) {
        return 550;
    }
    if desc.contains(&q) {
        return 450;
    }
    0
}

fn skill_source_priority(source: &str) -> usize {
    match source.trim().to_ascii_lowercase().as_str() {
        "official" => 0,
        "skills.sh" | "skills-sh" => 1,
        "well-known" => 2,
        "url" => 3,
        "github" => 4,
        "clawhub" => 5,
        "claude-marketplace" => 6,
        "lobehub" => 7,
        _ => 99,
    }
}

fn sort_registry_skill_records(records: &mut [RegistrySkillRecord]) {
    records.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| skill_source_priority(&a.source).cmp(&skill_source_priority(&b.source)))
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.identifier.cmp(&b.identifier))
    });
}

async fn fetch_hermes_skills_index(
    client: &reqwest::Client,
) -> Result<Vec<HermesSkillsIndexEntry>, AgentError> {
    let resp = client
        .get(HERMES_SKILLS_INDEX_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "hermes-agent-ultra")
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("Skills index request failed: {}", e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "Skills index lookup failed ({}): {}",
            status, body
        )));
    }
    let payload = resp
        .json::<HermesSkillsIndexResponse>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid skills index response: {}", e)))?;
    Ok(payload.skills)
}

fn resolved_source_from_index(entry: &HermesSkillsIndexEntry) -> Option<RegistryInstallSource> {
    let source = entry.source.to_ascii_lowercase();
    if source == "lobehub" {
        let slug = entry
            .identifier
            .strip_prefix("lobehub/")
            .unwrap_or(entry.identifier.as_str())
            .trim()
            .to_string();
        if slug.is_empty() {
            return None;
        }
        return Some(RegistryInstallSource::LobeHub { slug });
    }
    if source == "clawhub" {
        let slug = entry.identifier.trim().to_string();
        if slug.is_empty() {
            return None;
        }
        return Some(RegistryInstallSource::ClawHub {
            slug,
            version: None,
        });
    }
    if source == "official" {
        let path = entry.path.trim().trim_matches('/');
        if path.is_empty() {
            return None;
        }
        return Some(RegistryInstallSource::GitHub(ResolvedSkillSource {
            repo: OFFICIAL_SKILLS_REPO.to_string(),
            branch: "main".to_string(),
            skill_dir: path.to_string(),
        }));
    }

    if let Some(resolved) = entry.resolved_github_id.as_deref() {
        if let Some((repo, _, skill_dir)) = parse_explicit_github_skill(resolved) {
            return Some(RegistryInstallSource::GitHub(ResolvedSkillSource {
                repo,
                branch: "main".to_string(),
                skill_dir,
            }));
        }
    }

    if !entry.repo.trim().is_empty() {
        let dir = if !entry.path.trim().is_empty() {
            entry.path.trim_matches('/').to_string()
        } else {
            // claude-marketplace entries often point at repo root collections.
            "skills".to_string()
        };
        return Some(RegistryInstallSource::GitHub(ResolvedSkillSource {
            repo: entry.repo.trim().to_string(),
            branch: "main".to_string(),
            skill_dir: dir,
        }));
    }

    None
}

async fn search_multi_registry(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<RegistrySkillRecord>, AgentError> {
    let entries = fetch_hermes_skills_index(client).await?;
    let mut matches: Vec<RegistrySkillRecord> = Vec::new();
    for entry in entries {
        let score = score_registry_match(&entry, query);
        if score <= 0 {
            continue;
        }
        let Some(install_source) = resolved_source_from_index(&entry) else {
            continue;
        };
        matches.push(RegistrySkillRecord {
            identifier: entry.identifier.clone(),
            description: entry.description.clone(),
            source: entry.source.clone(),
            score,
            install_source,
        });
    }

    sort_registry_skill_records(&mut matches);
    if matches.len() > limit {
        matches.truncate(limit);
    }
    Ok(matches)
}

async fn resolve_skill_via_registry_index(
    client: &reqwest::Client,
    requested: &str,
    source_hint: Option<&str>,
) -> Result<RegistrySkillRecord, AgentError> {
    let entries = fetch_hermes_skills_index(client).await?;
    let requested_l = requested.trim().to_ascii_lowercase();
    let source_hint = source_hint.map(|s| s.to_ascii_lowercase());

    let mut exact: Vec<RegistrySkillRecord> = Vec::new();
    let mut fuzzy: Vec<RegistrySkillRecord> = Vec::new();
    for entry in entries {
        if let Some(ref hint) = source_hint {
            if entry.source.to_ascii_lowercase() != *hint {
                continue;
            }
        }
        let Some(install_source) = resolved_source_from_index(&entry) else {
            continue;
        };
        let source_l = entry.source.to_ascii_lowercase();
        let identifier_l = entry.identifier.to_ascii_lowercase();
        let name_l = entry.name.to_ascii_lowercase();
        let source_scoped = format!("{}/{}", source_l, name_l);
        let source_scoped_id = format!("{}/{}", source_l, identifier_l);
        let rec = RegistrySkillRecord {
            identifier: entry.identifier.clone(),
            description: entry.description.clone(),
            source: entry.source.clone(),
            score: score_registry_match(&entry, requested),
            install_source,
        };
        if requested_l == identifier_l
            || requested_l == name_l
            || requested_l == source_scoped
            || requested_l == source_scoped_id
        {
            exact.push(rec);
        } else if identifier_l.contains(&requested_l) || name_l.contains(&requested_l) {
            fuzzy.push(rec);
        }
    }

    sort_registry_skill_records(&mut exact);
    sort_registry_skill_records(&mut fuzzy);

    if let Some(first) = exact.into_iter().next() {
        return Ok(first);
    }
    if let Some(first) = fuzzy.into_iter().next() {
        return Ok(first);
    }
    Err(AgentError::Config(format!(
        "Skill '{}' was not found in multi-registry index.",
        requested
    )))
}

fn build_lobehub_skill_markdown(payload: &LobeHubAgentResponse, slug: &str) -> String {
    let title = if payload.meta.title.trim().is_empty() {
        slug.to_string()
    } else {
        payload.meta.title.trim().to_string()
    };
    let description = if payload.meta.description.trim().is_empty() {
        payload.summary.trim().to_string()
    } else {
        payload.meta.description.trim().to_string()
    };
    let role = payload.config.system_role.trim();

    let mut md = String::new();
    md.push_str("---\n");
    md.push_str(&format!("name: {}\n", slug));
    if !description.is_empty() {
        md.push_str(&format!(
            "description: {}\n",
            description.replace('\n', " ")
        ));
    }
    md.push_str("category: lobehub\n");
    md.push_str("---\n\n");
    md.push_str(&format!("# {}\n\n", title));
    if !description.is_empty() {
        md.push_str(&format!("{}\n\n", description));
    }
    md.push_str("## Source\n");
    md.push_str(&format!("- Registry: lobehub\n- Identifier: {}\n", slug));
    if !payload.author.trim().is_empty() {
        md.push_str(&format!("- Author: {}\n", payload.author.trim()));
    }
    if !payload.homepage.trim().is_empty() {
        md.push_str(&format!("- Homepage: {}\n", payload.homepage.trim()));
    }
    md.push_str("\n## Instructions\n");
    if role.is_empty() {
        md.push_str("No system role provided by source registry.\n");
    } else {
        md.push_str(role);
        md.push('\n');
    }
    md
}

fn default_trust_level_for_source(source: &str) -> &'static str {
    match source {
        "official" => "builtin",
        "skills.sh" | "hermes-index" | "claude-marketplace" | "github" | "tap" => "trusted",
        "lobehub" | "clawhub" => "community",
        _ => "community",
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn skills_hub_state_dir(skills_dir: &Path) -> PathBuf {
    skills_dir.join(SKILLS_HUB_STATE_DIR)
}

fn skills_hub_lock_path(skills_dir: &Path) -> PathBuf {
    hermes_skills::hub_lock_path(skills_dir)
}

fn skills_hub_audit_path(skills_dir: &Path) -> PathBuf {
    skills_hub_state_dir(skills_dir).join(SKILLS_HUB_AUDIT_FILE)
}

fn read_skills_hub_lock(skills_dir: &Path) -> SkillsHubLockFile {
    hermes_skills::read_hub_lock(skills_dir)
}

fn write_skills_hub_lock(skills_dir: &Path, lock: &SkillsHubLockFile) -> Result<(), AgentError> {
    let state_dir = skills_hub_state_dir(skills_dir);
    std::fs::create_dir_all(&state_dir).map_err(|e| {
        AgentError::Io(format!(
            "Failed to create skills hub state dir '{}': {}",
            state_dir.display(),
            e
        ))
    })?;
    let path = skills_hub_lock_path(skills_dir);
    let body = serde_json::to_string_pretty(lock)
        .map_err(|e| AgentError::Config(format!("Failed to serialize skills hub lock: {}", e)))?;
    std::fs::write(&path, body).map_err(|e| {
        AgentError::Io(format!(
            "Failed to write skills hub lock '{}': {}",
            path.display(),
            e
        ))
    })
}

fn append_skills_hub_audit(
    skills_dir: &Path,
    action: &str,
    entry: &SkillHubInstalledEntry,
) -> Result<(), AgentError> {
    let state_dir = skills_hub_state_dir(skills_dir);
    std::fs::create_dir_all(&state_dir).map_err(|e| {
        AgentError::Io(format!(
            "Failed to create skills hub state dir '{}': {}",
            state_dir.display(),
            e
        ))
    })?;
    let path = skills_hub_audit_path(skills_dir);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| {
            AgentError::Io(format!(
                "Failed to open skills hub audit log '{}': {}",
                path.display(),
                e
            ))
        })?;
    let line = serde_json::json!({
        "timestamp": now_rfc3339(),
        "action": action,
        "name": entry.name,
        "source": entry.source,
        "identifier": entry.identifier,
        "trust_level": entry.trust_level,
        "scan_verdict": entry.scan_verdict,
        "content_hash": entry.content_hash,
    });
    use std::io::Write as _;
    writeln!(file, "{}", line)
        .map_err(|e| AgentError::Io(format!("Failed to append skills hub audit log: {}", e)))
}

fn hash_skill_bundle(files: &[(String, Bytes)]) -> String {
    let mut sorted: Vec<_> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (rel_path, bytes) in sorted {
        h.update(rel_path.as_bytes());
        h.update([0]);
        h.update(bytes.as_ref());
        h.update([0xFF]);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn collect_skill_files_recursive(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), AgentError> {
    for entry in std::fs::read_dir(dir)
        .map_err(|e| AgentError::Io(format!("Failed to read dir '{}': {}", dir.display(), e)))?
    {
        let entry = entry.map_err(|e| {
            AgentError::Io(format!(
                "Failed to read dir entry '{}': {}",
                dir.display(),
                e
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            AgentError::Io(format!(
                "Failed to get file type for '{}': {}",
                path.display(),
                e
            ))
        })?;
        if file_type.is_dir() {
            collect_skill_files_recursive(root, &path, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|e| AgentError::Io(format!("Failed to compute relative path: {}", e)))?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&path)
            .map_err(|e| AgentError::Io(format!("Failed to read '{}': {}", path.display(), e)))?;
        out.push((rel, bytes));
    }
    Ok(())
}

fn hash_installed_skill_dir(skill_dir: &Path) -> Result<String, AgentError> {
    if !skill_dir.exists() {
        return Err(AgentError::Config(format!(
            "Installed skill path does not exist: {}",
            skill_dir.display()
        )));
    }
    let mut files = Vec::new();
    collect_skill_files_recursive(skill_dir, skill_dir, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (rel_path, bytes) in files {
        h.update(rel_path.as_bytes());
        h.update([0]);
        h.update(&bytes);
        h.update([0xFF]);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("sha256:{hex}"))
}

fn record_skill_install_in_hub_lock(
    skills_dir: &Path,
    installed_name: &str,
    install_path: &Path,
    files: &[(String, Bytes)],
    provenance: &SkillInstallProvenance,
) -> Result<(), AgentError> {
    let mut lock = read_skills_hub_lock(skills_dir);
    let now = now_rfc3339();
    let install_path_rel = install_path
        .strip_prefix(skills_dir)
        .unwrap_or(install_path)
        .to_string_lossy()
        .replace('\\', "/");
    let content_hash = hash_installed_skill_dir(install_path)?;
    let files_rel: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
    let entry = SkillHubInstalledEntry {
        name: installed_name.to_string(),
        source: provenance.source.clone(),
        identifier: provenance.identifier.clone(),
        trust_level: provenance.trust_level.clone(),
        scan_verdict: "clean".to_string(),
        content_hash,
        install_path: install_path_rel,
        files: files_rel,
        metadata: provenance.metadata.clone(),
        installed_at: now.clone(),
        updated_at: now,
    };
    lock.installed.retain(|item| item.name != installed_name);
    lock.installed.push(entry.clone());
    lock.installed.sort_by(|a, b| a.name.cmp(&b.name));
    write_skills_hub_lock(skills_dir, &lock)?;
    append_skills_hub_audit(skills_dir, "INSTALL", &entry)?;
    Ok(())
}

fn record_skill_uninstall_in_hub_lock(
    skills_dir: &Path,
    skill_name: &str,
) -> Result<Option<SkillHubInstalledEntry>, AgentError> {
    let mut lock = read_skills_hub_lock(skills_dir);
    let mut removed: Option<SkillHubInstalledEntry> = None;
    lock.installed.retain(|entry| {
        if entry.name == skill_name {
            removed = Some(entry.clone());
            false
        } else {
            true
        }
    });
    write_skills_hub_lock(skills_dir, &lock)?;
    if let Some(ref removed_entry) = removed {
        append_skills_hub_audit(skills_dir, "UNINSTALL", removed_entry)?;
    }
    Ok(removed)
}

fn skills_install_force(extra: Option<&str>) -> bool {
    if extra
        .map(|e| e.split_whitespace().any(|t| t == "--force"))
        .unwrap_or(false)
    {
        return true;
    }
    std::env::var("HERMES_SKILLS_INSTALL_FORCE")
        .ok()
        .map(|v| {
            let v = v.trim();
            v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes")
        })
        .unwrap_or(false)
}

fn skill_guard_enforce_bundle(
    install_name: &str,
    source: &str,
    files: &[(String, Bytes)],
    force: bool,
) -> Result<(), AgentError> {
    let file_vec: Vec<(String, Vec<u8>)> =
        files.iter().map(|(p, b)| (p.clone(), b.to_vec())).collect();
    hermes_skills::SkillGuard::enforce_install_bundle(install_name, source, &file_vec, force)
        .map_err(|e| AgentError::Config(e.to_string()))
}

fn github_request(client: &reqwest::Client, url: &str, accept: &str) -> reqwest::RequestBuilder {
    let mut req = client
        .get(url)
        .header("Accept", accept)
        .header("User-Agent", "hermes-agent-ultra");
    if let Ok(token) = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .map(|v| v.trim().to_string())
    {
        if !token.is_empty() {
            req = req.bearer_auth(token);
        }
    }
    req
}

async fn github_default_branch(client: &reqwest::Client, repo: &str) -> Result<String, AgentError> {
    let url = format!("{}/repos/{}", GITHUB_API_BASE, repo);
    let resp = github_request(client, &url, "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("GitHub request failed for {}: {}", repo, e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "GitHub repo lookup failed for {} ({}): {}",
            repo, status, body
        )));
    }
    let payload = resp
        .json::<GitHubRepoInfo>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid GitHub repo response: {}", e)))?;
    Ok(payload.default_branch)
}

async fn github_repo_tree(
    client: &reqwest::Client,
    repo: &str,
    branch: &str,
) -> Result<Vec<GitHubTreeEntry>, AgentError> {
    let encoded_branch = urlencoding::encode(branch);
    let url = format!(
        "{}/repos/{}/git/trees/{}?recursive=1",
        GITHUB_API_BASE, repo, encoded_branch
    );
    let resp = github_request(client, &url, "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("GitHub tree request failed: {}", e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "GitHub tree lookup failed for {repo}@{branch} ({status}): {body}"
        )));
    }
    let payload = resp
        .json::<GitHubTreeResponse>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid GitHub tree response: {}", e)))?;
    Ok(payload.tree)
}

async fn resolve_skill_via_taps(
    client: &reqwest::Client,
    taps: &[String],
    requested_skill: &str,
) -> Result<ResolvedSkillSource, AgentError> {
    let mut suggestions: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for tap in taps {
        let Some(spec) = parse_skill_tap_spec(tap) else {
            continue;
        };
        let branch = match github_default_branch(client, &spec.repo).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tree = match github_repo_tree(client, &spec.repo, &branch).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path_prefix = if spec.path.is_empty() {
            String::new()
        } else {
            format!("{}/", spec.path.trim_matches('/'))
        };
        for entry in tree {
            if entry.kind != "blob" || !entry.path.ends_with("/SKILL.md") {
                continue;
            }
            if !path_prefix.is_empty() && !entry.path.starts_with(&path_prefix) {
                continue;
            }
            let skill_dir = entry.path.trim_end_matches("/SKILL.md");
            let skill_name = skill_dir
                .split('/')
                .next_back()
                .unwrap_or(skill_dir)
                .to_string();
            if skill_name.eq_ignore_ascii_case(requested_skill) {
                return Ok(ResolvedSkillSource {
                    repo: spec.repo.clone(),
                    branch,
                    skill_dir: skill_dir.to_string(),
                });
            }
            if skill_name
                .to_ascii_lowercase()
                .contains(&requested_skill.to_ascii_lowercase())
            {
                suggestions.insert(skill_name);
            }
        }
    }

    let suggestion_text = if suggestions.is_empty() {
        "none".to_string()
    } else {
        suggestions
            .into_iter()
            .take(8)
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(AgentError::Config(format!(
        "Skill '{}' not found in configured taps. Suggestions: {}",
        requested_skill, suggestion_text
    )))
}

async fn resolve_skill_in_repo(
    client: &reqwest::Client,
    repo: &str,
    requested_skill: &str,
    preferred_prefix: Option<&str>,
) -> Result<ResolvedSkillSource, AgentError> {
    let branch = github_default_branch(client, repo).await?;
    let tree = github_repo_tree(client, repo, &branch).await?;

    let preferred_prefix = preferred_prefix
        .map(|v| v.trim_matches('/').to_string())
        .unwrap_or_default();
    let mut exact_candidates: Vec<String> = Vec::new();
    let mut fuzzy_candidates: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for entry in tree {
        if entry.kind != "blob" || !entry.path.ends_with("/SKILL.md") {
            continue;
        }
        let skill_dir = entry.path.trim_end_matches("/SKILL.md").to_string();
        let skill_name = skill_dir
            .split('/')
            .next_back()
            .unwrap_or(skill_dir.as_str())
            .to_string();
        if skill_name.eq_ignore_ascii_case(requested_skill) {
            exact_candidates.push(skill_dir.clone());
        } else if skill_name
            .to_ascii_lowercase()
            .contains(&requested_skill.to_ascii_lowercase())
        {
            fuzzy_candidates.insert(skill_name);
        }
    }

    if exact_candidates.is_empty() {
        let suggestion_text = if fuzzy_candidates.is_empty() {
            "none".to_string()
        } else {
            fuzzy_candidates
                .into_iter()
                .take(8)
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(AgentError::Config(format!(
            "Skill '{}' not found in repo {}. Suggestions: {}",
            requested_skill, repo, suggestion_text
        )));
    }

    exact_candidates.sort_by_key(|candidate| {
        let preferred = if preferred_prefix.is_empty() {
            1usize
        } else if candidate.starts_with(&format!("{}/", preferred_prefix)) {
            0usize
        } else {
            1usize
        };
        (preferred, candidate.len(), candidate.clone())
    });
    let skill_dir = exact_candidates
        .into_iter()
        .next()
        .ok_or_else(|| AgentError::Config("No matching skill path found.".into()))?;

    Ok(ResolvedSkillSource {
        repo: repo.to_string(),
        branch,
        skill_dir,
    })
}

async fn search_skills_via_taps(
    client: &reqwest::Client,
    taps: &[String],
    query: &str,
    limit: usize,
) -> Result<Vec<(String, String)>, AgentError> {
    let query_l = query.to_ascii_lowercase();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut out: Vec<(String, String)> = Vec::new();

    for tap in taps {
        let Some(spec) = parse_skill_tap_spec(tap) else {
            continue;
        };
        let branch = match github_default_branch(client, &spec.repo).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let tree = match github_repo_tree(client, &spec.repo, &branch).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path_prefix = if spec.path.is_empty() {
            String::new()
        } else {
            format!("{}/", spec.path.trim_matches('/'))
        };
        for entry in tree {
            if entry.kind != "blob" || !entry.path.ends_with("/SKILL.md") {
                continue;
            }
            if !path_prefix.is_empty() && !entry.path.starts_with(&path_prefix) {
                continue;
            }
            let skill_dir = entry.path.trim_end_matches("/SKILL.md");
            let skill_name = skill_dir.split('/').next_back().unwrap_or(skill_dir);
            if !skill_name.to_ascii_lowercase().contains(&query_l) {
                continue;
            }
            let key = format!("{}/{}", spec.repo, skill_dir);
            if seen.insert(key.clone()) {
                out.push((skill_name.to_string(), key));
                if out.len() >= limit {
                    return Ok(out);
                }
            }
        }
    }

    Ok(out)
}

async fn fetch_skill_files_from_github(
    client: &reqwest::Client,
    source: &ResolvedSkillSource,
) -> Result<Vec<(String, Bytes)>, AgentError> {
    let tree = github_repo_tree(client, &source.repo, &source.branch).await?;
    let prefix = format!("{}/", source.skill_dir.trim_matches('/'));
    let mut files = Vec::new();

    for entry in tree {
        if entry.kind != "blob" || !entry.path.starts_with(&prefix) {
            continue;
        }
        let rel_path = entry.path[prefix.len()..].to_string();
        ensure_safe_relative_path(&rel_path)?;
        let raw_path = entry
            .path
            .split('/')
            .map(|segment| urlencoding::encode(segment).to_string())
            .collect::<Vec<_>>()
            .join("/");
        let raw_url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}",
            source.repo, source.branch, raw_path
        );
        let bytes = match client
            .get(&raw_url)
            .header("User-Agent", "hermes-agent-ultra")
            .timeout(std::time::Duration::from_secs(25))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp
                .bytes()
                .await
                .map_err(|e| AgentError::Config(format!("Invalid file payload: {}", e)))?,
            _ => {
                let encoded_path = entry
                    .path
                    .split('/')
                    .map(urlencoding::encode)
                    .collect::<Vec<_>>()
                    .join("/");
                let api_url = format!(
                    "{}/repos/{}/contents/{}?ref={}",
                    GITHUB_API_BASE,
                    source.repo,
                    encoded_path,
                    urlencoding::encode(&source.branch)
                );
                let resp = github_request(client, &api_url, "application/vnd.github.v3.raw")
                    .timeout(std::time::Duration::from_secs(25))
                    .send()
                    .await
                    .map_err(|e| {
                        AgentError::Config(format!("GitHub file download failed: {}", e))
                    })?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(AgentError::Config(format!(
                        "Failed to download {} from {} ({}): {}",
                        rel_path, source.repo, status, body
                    )));
                }
                resp.bytes()
                    .await
                    .map_err(|e| AgentError::Config(format!("Invalid file payload: {}", e)))?
            }
        };
        files.push((rel_path, bytes));
    }

    if !files.iter().any(|(path, _)| path == "SKILL.md") {
        return Err(AgentError::Config(format!(
            "Resolved source {}/{} is missing SKILL.md",
            source.repo, source.skill_dir
        )));
    }
    if files.is_empty() {
        return Err(AgentError::Config(format!(
            "No files found at {}/{}",
            source.repo, source.skill_dir
        )));
    }
    Ok(files)
}

async fn fetch_lobehub_skill_files(
    client: &reqwest::Client,
    slug: &str,
) -> Result<Vec<(String, Bytes)>, AgentError> {
    let url = format!("https://chat-agents.lobehub.com/{}.json", slug);
    let resp = client
        .get(&url)
        .header("Accept", "application/json,text/plain,*/*")
        .header("User-Agent", "Mozilla/5.0 hermes-agent-ultra")
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("LobeHub request failed: {}", e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "LobeHub lookup failed for '{}' ({}): {}",
            slug, status, body
        )));
    }
    let payload = resp
        .json::<LobeHubAgentResponse>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid LobeHub payload: {}", e)))?;
    let md = build_lobehub_skill_markdown(&payload, slug);
    Ok(vec![("SKILL.md".to_string(), Bytes::from(md))])
}

fn detect_archive_format(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 4
        && bytes[0] == 0x50
        && bytes[1] == 0x4B
        && bytes[2] == 0x03
        && bytes[3] == 0x04
    {
        return "zip";
    }
    if bytes.len() >= 2 && bytes[0] == 0x1F && bytes[1] == 0x8B {
        return "tar.gz";
    }
    "unknown"
}

fn extract_clawhub_archive(bytes: &[u8]) -> Result<Vec<(String, Bytes)>, AgentError> {
    match detect_archive_format(bytes) {
        "zip" => {
            let cursor = std::io::Cursor::new(bytes);
            let mut zip = zip::ZipArchive::new(cursor).map_err(|e| {
                AgentError::Config(format!("Failed to parse ClawHub zip payload: {}", e))
            })?;
            let mut out = Vec::new();
            for i in 0..zip.len() {
                let mut file = zip.by_index(i).map_err(|e| {
                    AgentError::Config(format!("Failed to read ClawHub zip entry: {}", e))
                })?;
                if file.is_dir() {
                    continue;
                }
                let raw_name = file.name().replace('\\', "/");
                let segments: Vec<&str> = raw_name.split('/').filter(|s| !s.is_empty()).collect();
                let normalized = if segments.is_empty() {
                    file.name().to_string()
                } else if segments.len() == 1 {
                    segments[0].to_string()
                } else {
                    // Drop top-level archive folder if present.
                    segments[1..].join("/")
                };
                ensure_safe_relative_path(&normalized)?;
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut file, &mut buf).map_err(|e| {
                    AgentError::Config(format!("Failed to read ClawHub file payload: {}", e))
                })?;
                out.push((normalized, Bytes::from(buf)));
            }
            Ok(out)
        }
        "tar.gz" => {
            let decoder = flate2::read::GzDecoder::new(bytes);
            let mut archive = tar::Archive::new(decoder);
            let mut out = Vec::new();
            let entries = archive.entries().map_err(|e| {
                AgentError::Config(format!("Failed to parse ClawHub tar payload: {}", e))
            })?;
            for entry in entries {
                let mut entry = entry.map_err(|e| {
                    AgentError::Config(format!("Failed to read ClawHub tar entry: {}", e))
                })?;
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                let path = entry
                    .path()
                    .map_err(|e| AgentError::Config(format!("Invalid tar entry path: {}", e)))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let normalized = path.split('/').skip(1).collect::<Vec<_>>().join("/");
                if normalized.is_empty() {
                    continue;
                }
                ensure_safe_relative_path(&normalized)?;
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut buf).map_err(|e| {
                    AgentError::Config(format!("Failed to read ClawHub tar payload: {}", e))
                })?;
                out.push((normalized, Bytes::from(buf)));
            }
            Ok(out)
        }
        _ => Err(AgentError::Config(
            "Unsupported ClawHub archive format (expected zip or tar.gz).".to_string(),
        )),
    }
}

async fn fetch_clawhub_skill_files(
    client: &reqwest::Client,
    slug: &str,
    version_hint: Option<&str>,
) -> Result<Vec<(String, Bytes)>, AgentError> {
    let detail_url = format!("{}/skills/{}", CLAWHUB_API_BASE, slug);
    let detail = client
        .get(&detail_url)
        .header("Accept", "application/json")
        .header("User-Agent", "Mozilla/5.0 hermes-agent-ultra")
        .timeout(std::time::Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("ClawHub detail request failed: {}", e)))?;
    if !detail.status().is_success() {
        let status = detail.status();
        let body = detail.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "ClawHub detail lookup failed for '{}' ({}): {}",
            slug, status, body
        )));
    }
    let payload = detail
        .json::<ClawHubSkillDetailResponse>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid ClawHub detail payload: {}", e)))?;
    let version = version_hint
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.to_string())
        .or_else(|| {
            let v = payload.latest_version.version.trim();
            if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            }
        })
        .ok_or_else(|| {
            AgentError::Config(format!("No ClawHub version available for '{}'.", slug))
        })?;

    let download_url = format!(
        "{}/download?slug={}&version={}",
        CLAWHUB_API_BASE,
        urlencoding::encode(slug),
        urlencoding::encode(&version)
    );
    let mut last_err = String::new();
    for attempt in 1..=4 {
        let resp = client
            .get(&download_url)
            .header("Accept", "*/*")
            .header("User-Agent", "Mozilla/5.0 hermes-agent-ultra")
            .timeout(std::time::Duration::from_secs(40))
            .send()
            .await
            .map_err(|e| AgentError::Config(format!("ClawHub download request failed: {}", e)))?;
        if resp.status().is_success() {
            let bytes = resp.bytes().await.map_err(|e| {
                AgentError::Config(format!("Invalid ClawHub download payload: {}", e))
            })?;
            return extract_clawhub_archive(&bytes);
        }
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let wait_secs = attempt * 2;
            tokio::time::sleep(std::time::Duration::from_secs(wait_secs as u64)).await;
            last_err = "rate limited (429)".to_string();
            continue;
        }
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "ClawHub download failed for '{}@{}' ({}): {}",
            slug, version, status, body
        )));
    }
    Err(AgentError::Config(format!(
        "ClawHub download for '{}@{}' failed after retries: {}",
        slug, version, last_err
    )))
}

#[derive(Debug, Deserialize)]
struct ClaudeMarketplaceManifest {
    #[serde(default)]
    plugins: Vec<ClaudeMarketplacePlugin>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClaudeMarketplacePlugin {
    #[serde(default)]
    name: String,
    #[serde(default)]
    skills: Vec<String>,
}

async fn fetch_claude_marketplace_manifest(
    client: &reqwest::Client,
) -> Result<ClaudeMarketplaceManifest, AgentError> {
    let url = format!(
        "{}/repos/anthropics/skills/contents/.claude-plugin/marketplace.json",
        GITHUB_API_BASE
    );
    let resp = github_request(client, &url, "application/vnd.github.v3.raw")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("Claude marketplace request failed: {}", e)))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "Claude marketplace lookup failed ({}): {}",
            status, body
        )));
    }
    resp.json::<ClaudeMarketplaceManifest>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid Claude marketplace payload: {}", e)))
}

async fn resolve_claude_marketplace_skill(
    client: &reqwest::Client,
    requested: &str,
) -> Result<ResolvedSkillSource, AgentError> {
    let manifest = fetch_claude_marketplace_manifest(client).await?;
    let req = requested.trim().trim_matches('/').to_ascii_lowercase();
    let mut candidate_paths: Vec<String> = Vec::new();
    for plugin in manifest.plugins {
        let plugin_name = plugin.name.to_ascii_lowercase();
        for skill_path in plugin.skills {
            let normalized = skill_path
                .trim()
                .trim_start_matches("./")
                .trim_start_matches('/')
                .to_string();
            if normalized.is_empty() {
                continue;
            }
            let basename = normalized
                .split('/')
                .next_back()
                .unwrap_or(normalized.as_str())
                .to_ascii_lowercase();
            if req == basename
                || req == normalized.to_ascii_lowercase()
                || req == format!("{}/{}", plugin_name, basename)
                || req == format!("{}/{}", plugin_name, normalized.to_ascii_lowercase())
            {
                return Ok(ResolvedSkillSource {
                    repo: "anthropics/skills".to_string(),
                    branch: "main".to_string(),
                    skill_dir: normalized,
                });
            }
            if basename.contains(&req) || normalized.to_ascii_lowercase().contains(&req) {
                candidate_paths.push(normalized);
            }
        }
    }
    candidate_paths.sort();
    candidate_paths.dedup();
    Err(AgentError::Config(format!(
        "Claude marketplace skill '{}' not found. Suggestions: {}",
        requested,
        if candidate_paths.is_empty() {
            "none".to_string()
        } else {
            candidate_paths
                .into_iter()
                .take(8)
                .collect::<Vec<_>>()
                .join(", ")
        }
    )))
}

async fn resolve_official_skill_source(
    client: &reqwest::Client,
    requested: &str,
) -> Result<ResolvedSkillSource, AgentError> {
    let req = requested.trim().trim_matches('/');
    if req.is_empty() {
        return Err(AgentError::Config(
            "Missing official skill identifier (e.g., official/security/1password).".to_string(),
        ));
    }

    let normalized = canonicalize_official_skill_dir(req.trim_start_matches("official/"));
    if normalized.is_empty() {
        return Err(AgentError::Config(
            "Missing official skill identifier (e.g., official/security/1password).".to_string(),
        ));
    }

    let branch = github_default_branch(client, OFFICIAL_SKILLS_REPO).await?;
    let tree = github_repo_tree(client, OFFICIAL_SKILLS_REPO, &branch).await?;
    let has_skill_dir = |dir: &str| -> bool {
        let target = format!("{}/SKILL.md", dir.trim_matches('/'));
        tree.iter()
            .any(|entry| entry.kind == "blob" && entry.path == target)
    };

    let mut candidate_queries = vec![
        req.to_string(),
        normalized.clone(),
        format!("official/{}", normalized),
    ];
    let basename = normalized
        .split('/')
        .next_back()
        .unwrap_or(normalized.as_str())
        .to_string();
    if !basename.is_empty() {
        candidate_queries.push(basename);
    }
    candidate_queries.sort();
    candidate_queries.dedup();

    for query in candidate_queries {
        if let Ok(record) = resolve_skill_via_registry_index(client, &query, Some("official")).await
        {
            if let RegistryInstallSource::GitHub(source) = record.install_source {
                let mut candidates = official_skill_path_candidates(&source.skill_dir);
                for c in official_skill_path_candidates(&normalized) {
                    if !candidates.iter().any(|existing| existing == &c) {
                        candidates.push(c);
                    }
                }
                for candidate in candidates {
                    if has_skill_dir(&candidate) {
                        return Ok(ResolvedSkillSource {
                            repo: OFFICIAL_SKILLS_REPO.to_string(),
                            branch: branch.clone(),
                            skill_dir: candidate,
                        });
                    }
                }
            }
        }
    }

    for candidate in official_skill_path_candidates(&normalized) {
        if has_skill_dir(&candidate) {
            return Ok(ResolvedSkillSource {
                repo: OFFICIAL_SKILLS_REPO.to_string(),
                branch: branch.clone(),
                skill_dir: candidate,
            });
        }
    }

    Err(AgentError::Config(format!(
        "Official skill '{}' not found in upstream skills or optional-skills catalogs.",
        requested
    )))
}

fn canonicalize_official_skill_dir(path: &str) -> String {
    path.trim().trim_matches('/').to_string()
}

fn official_skill_path_candidates(path_like: &str) -> Vec<String> {
    let normalized = canonicalize_official_skill_dir(path_like);
    if normalized.is_empty() {
        return Vec::new();
    }
    if normalized.starts_with("skills/") || normalized.starts_with("optional-skills/") {
        return vec![normalized];
    }
    vec![
        format!("skills/{}", normalized),
        format!("optional-skills/{}", normalized),
    ]
}

async fn resolve_skills_sh_source(
    client: &reqwest::Client,
    requested: &str,
) -> Result<ResolvedSkillSource, AgentError> {
    let req = requested.trim().trim_matches('/');
    if req.is_empty() {
        return Err(AgentError::Config(
            "Missing skills.sh skill identifier.".to_string(),
        ));
    }
    if let Some((repo, _, skill_dir)) = parse_explicit_github_skill(req) {
        let branch = github_default_branch(client, &repo).await?;
        return Ok(ResolvedSkillSource {
            repo,
            branch,
            skill_dir,
        });
    }

    if let Ok(resolved) = resolve_skill_via_registry_index(client, req, Some("skills.sh")).await {
        if let RegistryInstallSource::GitHub(source) = resolved.install_source {
            let branch = github_default_branch(client, &source.repo).await?;
            return Ok(ResolvedSkillSource { branch, ..source });
        }
    }

    let search_resp = client
        .get(SKILLS_SH_SEARCH_URL)
        .query(&[("q", req), ("limit", "20")])
        .header("Accept", "application/json")
        .header("User-Agent", "hermes-agent-ultra")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("skills.sh search request failed: {}", e)))?;
    if !search_resp.status().is_success() {
        let status = search_resp.status();
        let body = search_resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "skills.sh search failed ({}): {}",
            status, body
        )));
    }
    let payload = search_resp
        .json::<SkillsShSearchResponse>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid skills.sh payload: {}", e)))?;
    let req_l = req.to_ascii_lowercase();
    for hit in payload.skills {
        let source = hit.source.trim();
        if source.is_empty() {
            continue;
        }
        let skill_id = if hit.skill_id.trim().is_empty() {
            hit.name.trim().to_string()
        } else {
            hit.skill_id.trim().to_string()
        };
        let repo = source.to_string();
        let branch = github_default_branch(client, &repo).await?;
        if let Ok(found) = resolve_skill_in_repo(client, &repo, &skill_id, Some("skills")).await {
            return Ok(found);
        }
        if let Ok(found) = resolve_skill_in_repo(client, &repo, &req_l, Some("skills")).await {
            return Ok(found);
        }
        if let Some((repo2, _, dir)) = parse_explicit_github_skill(&hit.id) {
            return Ok(ResolvedSkillSource {
                repo: repo2,
                branch,
                skill_dir: dir,
            });
        }
    }

    Err(AgentError::Config(format!(
        "Unable to resolve skills.sh skill '{}'.",
        requested
    )))
}

async fn search_skills_sh_registry(
    client: &reqwest::Client,
    query: &str,
    limit: usize,
) -> Result<Vec<(String, String)>, AgentError> {
    let capped_limit = limit.clamp(1, 50).to_string();
    let search_resp = client
        .get(SKILLS_SH_SEARCH_URL)
        .query(&[("q", query), ("limit", capped_limit.as_str())])
        .header("Accept", "application/json")
        .header("User-Agent", "hermes-agent-ultra")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| AgentError::Config(format!("skills.sh search request failed: {}", e)))?;
    if !search_resp.status().is_success() {
        let status = search_resp.status();
        let body = search_resp.text().await.unwrap_or_default();
        return Err(AgentError::Config(format!(
            "skills.sh search failed ({}): {}",
            status, body
        )));
    }
    let payload = search_resp
        .json::<SkillsShSearchResponse>()
        .await
        .map_err(|e| AgentError::Config(format!("Invalid skills.sh payload: {}", e)))?;

    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for hit in payload.skills {
        let id = hit.id.trim();
        if id.is_empty() {
            continue;
        }
        let identifier = format!("skills.sh/{}", id);
        if !seen.insert(identifier.clone()) {
            continue;
        }
        let display_name = if hit.name.trim().is_empty() {
            id.to_string()
        } else {
            hit.name.trim().to_string()
        };
        out.push((display_name, identifier));
    }
    Ok(out)
}

async fn resolve_install_via_fallback_router(
    client: &reqwest::Client,
    skill_name: &str,
    taps: &[String],
) -> Result<(ResolvedSkillSource, InstallFallbackSource), AgentError> {
    if let Ok(resolved) = resolve_skills_sh_source(client, skill_name).await {
        return Ok((resolved, InstallFallbackSource::SkillsSh));
    }
    let resolved = resolve_skill_via_taps(client, taps, skill_name).await?;
    Ok((resolved, InstallFallbackSource::Tap))
}

fn parse_repo_skill_identifier(identifier: &str) -> Option<(String, String)> {
    let trimmed = identifier.trim().trim_start_matches("github/");
    let pieces: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    if pieces.len() < 3 {
        return None;
    }
    let repo = format!("{}/{}", pieces[0], pieces[1]);
    let skill_dir = pieces[2..].join("/");
    if skill_dir.is_empty() {
        None
    } else {
        Some((repo, skill_dir))
    }
}

fn canonicalize_skills_sh_identifier(identifier: &str) -> String {
    identifier
        .trim()
        .trim_start_matches("skills.sh/")
        .trim_start_matches("skills-sh/")
        .to_string()
}

async fn fetch_bundle_for_lock_entry(
    client: &reqwest::Client,
    entry: &SkillHubInstalledEntry,
    taps: &[String],
) -> Result<Vec<(String, Bytes)>, AgentError> {
    match entry.source.as_str() {
        "official" => {
            let resolved = resolve_official_skill_source(client, &entry.identifier).await?;
            fetch_skill_files_from_github(client, &resolved).await
        }
        "skills.sh" | "skills-sh" => {
            let id = canonicalize_skills_sh_identifier(&entry.identifier);
            let resolved = resolve_skills_sh_source(client, &id).await?;
            fetch_skill_files_from_github(client, &resolved).await
        }
        "lobehub" => fetch_lobehub_skill_files(client, &entry.identifier).await,
        "clawhub" => fetch_clawhub_skill_files(client, &entry.identifier, None).await,
        "claude-marketplace" => {
            let resolved = resolve_claude_marketplace_skill(client, &entry.identifier).await?;
            fetch_skill_files_from_github(client, &resolved).await
        }
        "tap" => {
            if let Some((repo, skill_dir)) = parse_repo_skill_identifier(&entry.identifier) {
                let branch = github_default_branch(client, &repo).await?;
                return fetch_skill_files_from_github(
                    client,
                    &ResolvedSkillSource {
                        repo,
                        branch,
                        skill_dir,
                    },
                )
                .await;
            }
            let resolved = resolve_skill_via_taps(client, taps, &entry.name).await?;
            fetch_skill_files_from_github(client, &resolved).await
        }
        "github" => {
            if let Some((repo, maybe_branch, skill_dir)) =
                parse_explicit_github_skill(&entry.identifier)
            {
                let branch = if let Some(branch) = maybe_branch {
                    branch
                } else {
                    github_default_branch(client, &repo).await?
                };
                return fetch_skill_files_from_github(
                    client,
                    &ResolvedSkillSource {
                        repo,
                        branch,
                        skill_dir,
                    },
                )
                .await;
            }
            if let Some((repo, skill_dir)) = parse_repo_skill_identifier(&entry.identifier) {
                let branch = github_default_branch(client, &repo).await?;
                return fetch_skill_files_from_github(
                    client,
                    &ResolvedSkillSource {
                        repo,
                        branch,
                        skill_dir,
                    },
                )
                .await;
            }
            let resolved =
                resolve_skill_in_repo(client, &entry.identifier, &entry.name, None).await?;
            fetch_skill_files_from_github(client, &resolved).await
        }
        other => {
            if let Ok(hit) =
                resolve_skill_via_registry_index(client, &entry.identifier, Some(other)).await
            {
                return match hit.install_source {
                    RegistryInstallSource::GitHub(source) => {
                        let branch = github_default_branch(client, &source.repo).await?;
                        fetch_skill_files_from_github(
                            client,
                            &ResolvedSkillSource { branch, ..source },
                        )
                        .await
                    }
                    RegistryInstallSource::LobeHub { slug } => {
                        fetch_lobehub_skill_files(client, &slug).await
                    }
                    RegistryInstallSource::ClawHub { slug, version } => {
                        fetch_clawhub_skill_files(client, &slug, version.as_deref()).await
                    }
                };
            }
            Err(AgentError::Config(format!(
                "Unknown hub source '{}' for installed skill '{}'",
                entry.source, entry.name
            )))
        }
    }
}

fn install_skill_files(
    skills_dir: &std::path::Path,
    install_name: &str,
    files: &[(String, Bytes)],
    source: &str,
    force: bool,
) -> Result<std::path::PathBuf, AgentError> {
    skill_guard_enforce_bundle(install_name, source, files, force)?;

    std::fs::create_dir_all(skills_dir)
        .map_err(|e| AgentError::Io(format!("Failed to create skills dir: {}", e)))?;

    let target = skills_dir.join(install_name);
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|e| AgentError::Io(format!("Failed to remove existing skill dir: {}", e)))?;
    }
    std::fs::create_dir_all(&target)
        .map_err(|e| AgentError::Io(format!("Failed to create skill dir: {}", e)))?;

    for (rel_path, bytes) in files {
        ensure_safe_relative_path(rel_path)?;
        let output = target.join(rel_path);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AgentError::Io(format!("Failed to create parent dirs: {}", e)))?;
        }
        std::fs::write(&output, bytes)
            .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", output.display(), e)))?;
    }

    let skill_md = target.join("SKILL.md");
    if !skill_md.exists() {
        return Err(AgentError::Config(format!(
            "Installed skill is missing SKILL.md at {}",
            skill_md.display()
        )));
    }

    Ok(target)
}

fn skill_auto_bootstrap_enabled() -> bool {
    !std::env::var("HERMES_SKILL_AUTO_BOOTSTRAP")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
}

fn skill_bootstrap_force_confirmed() -> bool {
    std::env::var("HERMES_SKILL_BOOTSTRAP_ASSUME_YES")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        || std::env::var("HERMES_SKILL_BOOTSTRAP_FORCE")
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
}

fn prompt_bootstrap_yes_no(prompt: &str, default_yes: bool) -> bool {
    use std::io::Write as _;
    print!("{}", prompt);
    let _ = std::io::stdout().flush();
    let mut buf = String::new();
    if std::io::stdin().read_line(&mut buf).is_err() {
        return default_yes;
    }
    let answer = buf.trim().to_ascii_lowercase();
    if answer.is_empty() {
        return default_yes;
    }
    matches!(answer.as_str(), "y" | "yes")
}

fn push_bootstrap_command_if_present(commands: &mut Vec<String>, raw: Option<&str>) {
    if let Some(raw) = raw {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            commands.push(trimmed.to_string());
        }
    }
}

fn collect_bootstrap_commands_from_value(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => push_bootstrap_command_if_present(out, Some(s)),
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(s) = item.as_str() {
                    push_bootstrap_command_if_present(out, Some(s));
                }
            }
        }
        serde_json::Value::Object(map) => {
            push_bootstrap_command_if_present(out, map.get("command").and_then(|v| v.as_str()));
            if let Some(commands) = map.get("commands") {
                collect_bootstrap_commands_from_value(commands, out);
            }
            if let Some(script) = map.get("script").and_then(|v| v.as_str()) {
                let script = script.trim();
                if !script.is_empty() {
                    if script.ends_with(".py") {
                        out.push(format!("python3 {}", script));
                    } else {
                        out.push(format!("bash {}", script));
                    }
                }
            }
            if let Some(scripts) = map.get("scripts").and_then(|v| v.as_array()) {
                for script in scripts {
                    if let Some(script) = script.as_str() {
                        let script = script.trim();
                        if script.is_empty() {
                            continue;
                        }
                        if script.ends_with(".py") {
                            out.push(format!("python3 {}", script));
                        } else {
                            out.push(format!("bash {}", script));
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn parse_skill_bootstrap_plan(
    files: &[(String, Bytes)],
) -> Result<Option<SkillBootstrapPlan>, AgentError> {
    let skill_md = files
        .iter()
        .find_map(|(path, bytes)| {
            if path == "SKILL.md" {
                Some(bytes)
            } else {
                None
            }
        })
        .ok_or_else(|| AgentError::Config("Installed skill payload is missing SKILL.md".into()))?;

    let content = std::str::from_utf8(skill_md)
        .map_err(|e| AgentError::Config(format!("Installed SKILL.md is not valid UTF-8: {}", e)))?;
    let (frontmatter, _body) = hermes_tools::tools::skill_utils::parse_frontmatter(content);

    let mut commands = Vec::new();
    for key in [
        "bootstrap",
        "setup",
        "install",
        "bootstrap_command",
        "setup_command",
        "install_command",
        "bootstrap_commands",
        "setup_commands",
        "install_commands",
    ] {
        if let Some(value) = frontmatter.get(key) {
            collect_bootstrap_commands_from_value(value, &mut commands);
        }
    }

    let mut dedup = HashSet::new();
    let normalized: Vec<String> = commands
        .into_iter()
        .filter_map(|cmd| {
            let trimmed = cmd.trim().to_string();
            if trimmed.is_empty() || !dedup.insert(trimmed.clone()) {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect();

    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(SkillBootstrapPlan {
            commands: normalized,
        }))
    }
}

fn is_allowed_bootstrap_executable(executable: &str) -> bool {
    let normalized = executable
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(executable)
        .trim()
        .to_ascii_lowercase();
    SKILL_BOOTSTRAP_ALLOWED_EXECUTABLES
        .iter()
        .any(|allowed| *allowed == normalized)
}

fn parse_bootstrap_command(raw: &str) -> Result<ParsedBootstrapCommand, AgentError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AgentError::Config(
            "Bootstrap command cannot be empty".to_string(),
        ));
    }
    if trimmed.len() > 2048 {
        return Err(AgentError::Config(
            "Bootstrap command is too long (>2048 bytes)".to_string(),
        ));
    }

    // Deliberately block shell control operators and substitutions.
    let forbidden = Regex::new(r"[`\n\r;]|&&|\|\||\||>>?|<<?|\$\(").expect("valid regex");
    if forbidden.is_match(trimmed) {
        return Err(AgentError::Config(format!(
            "Blocked bootstrap command (contains forbidden shell operators): {}",
            trimmed
        )));
    }

    let mut tokens = shlex::split(trimmed).ok_or_else(|| {
        AgentError::Config(format!(
            "Unable to parse bootstrap command safely: {}",
            trimmed
        ))
    })?;
    if tokens.is_empty() {
        return Err(AgentError::Config(
            "Bootstrap command parsed to no executable".to_string(),
        ));
    }

    let executable = tokens.remove(0);
    if executable.contains('/') || executable.contains('\\') {
        let path = Path::new(&executable);
        if path.is_absolute() {
            return Err(AgentError::Config(format!(
                "Bootstrap executable must be relative (got absolute path): {}",
                executable
            )));
        }
        ensure_safe_relative_path(&executable)?;
        if executable.ends_with(".sh") {
            let mut args = vec![executable];
            args.extend(tokens);
            return Ok(ParsedBootstrapCommand {
                display: trimmed.to_string(),
                executable: "bash".to_string(),
                args,
            });
        }
        if executable.ends_with(".py") {
            let mut args = vec![executable];
            args.extend(tokens);
            return Ok(ParsedBootstrapCommand {
                display: trimmed.to_string(),
                executable: "python3".to_string(),
                args,
            });
        }
    } else if !is_allowed_bootstrap_executable(&executable) {
        return Err(AgentError::Config(format!(
            "Bootstrap executable '{}' is not in the allowlist",
            executable
        )));
    }

    Ok(ParsedBootstrapCommand {
        display: trimmed.to_string(),
        executable,
        args: tokens,
    })
}

async fn execute_bootstrap_command(
    skill_dir: &Path,
    command: &ParsedBootstrapCommand,
) -> Result<(), AgentError> {
    let exec_path = if command.executable.contains('/') || command.executable.contains('\\') {
        skill_dir.join(&command.executable)
    } else {
        PathBuf::from(&command.executable)
    };

    let mut process = tokio::process::Command::new(&exec_path);
    process
        .args(&command.args)
        .current_dir(skill_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = process.output().await.map_err(|e| {
        AgentError::Io(format!(
            "Failed to execute bootstrap command '{}': {}",
            command.display, e
        ))
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        if !stdout.is_empty() {
            println!(
                "    stdout: {}",
                stdout.lines().take(3).collect::<Vec<_>>().join(" | ")
            );
        }
        Ok(())
    } else {
        Err(AgentError::Config(format!(
            "Bootstrap command failed (exit={}): {}\n{}\n{}",
            output.status,
            command.display,
            if stdout.is_empty() { "" } else { "stdout:" },
            if stdout.is_empty() {
                stderr
            } else if stderr.is_empty() {
                stdout
            } else {
                format!("{}\nstderr:\n{}", stdout, stderr)
            }
        )))
    }
}

async fn maybe_run_skill_bootstrap(
    install_name: &str,
    skill_dir: &Path,
    files: &[(String, Bytes)],
) -> Result<(), AgentError> {
    if !skill_auto_bootstrap_enabled() {
        println!("Skill bootstrap skipped: HERMES_SKILL_AUTO_BOOTSTRAP=0.");
        return Ok(());
    }

    let Some(plan) = parse_skill_bootstrap_plan(files)? else {
        return Ok(());
    };

    let mut runnable: Vec<(ParsedBootstrapCommand, hermes_tools::ApprovalDecision)> = Vec::new();
    let mut blocked: Vec<(String, String)> = Vec::new();
    for raw in plan.commands {
        match parse_bootstrap_command(&raw) {
            Ok(parsed) => {
                let decision = hermes_tools::check_approval(&parsed.display);
                if matches!(decision, hermes_tools::ApprovalDecision::Denied) {
                    blocked.push((
                        parsed.display,
                        "blocked by command approval policy".to_string(),
                    ));
                } else {
                    runnable.push((parsed, decision));
                }
            }
            Err(err) => blocked.push((raw, err.to_string())),
        }
    }

    if runnable.is_empty() && blocked.is_empty() {
        return Ok(());
    }

    println!(
        "Detected bootstrap plan for '{}': {} runnable command(s), {} blocked.",
        install_name,
        runnable.len(),
        blocked.len()
    );
    for (cmd, reason) in &blocked {
        println!("  - blocked: `{}` ({})", cmd, reason);
    }
    if runnable.is_empty() {
        return Ok(());
    }

    let has_confirm = runnable.iter().any(|(_, decision)| {
        matches!(
            decision,
            hermes_tools::ApprovalDecision::RequiresConfirmation
        )
    });
    let force_yes = skill_bootstrap_force_confirmed();
    if has_confirm && !force_yes {
        let proceed = prompt_bootstrap_yes_no(
            "Run bootstrap commands that require installer confirmation now? [Y/n]: ",
            true,
        );
        if !proceed {
            println!("Skipped bootstrap execution.");
            return Ok(());
        }
    }

    for (command, decision) in runnable {
        if matches!(
            decision,
            hermes_tools::ApprovalDecision::RequiresConfirmation
        ) && !force_yes
        {
            println!("  • running (confirmed): {}", command.display);
        } else if matches!(decision, hermes_tools::ApprovalDecision::Approved) {
            println!("  • running: {}", command.display);
        } else if !force_yes {
            println!("  • skipped: {} (confirmation required)", command.display);
            continue;
        } else {
            println!("  • running (forced): {}", command.display);
        }
        execute_bootstrap_command(skill_dir, &command).await?;
    }

    Ok(())
}

fn normalize_tap_path_for_storage(path: &str) -> String {
    let normalized = path.trim_matches('/');
    if normalized.is_empty() {
        String::new()
    } else {
        format!("{}/", normalized)
    }
}

fn tap_object_to_string(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    if let Some(url) = obj
        .get("url")
        .and_then(|u| u.as_str())
        .or_else(|| obj.get("tap").and_then(|u| u.as_str()))
    {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let repo = obj.get("repo").and_then(|v| v.as_str())?;
    let repo = repo.trim().trim_matches('/');
    if repo.is_empty() {
        return None;
    }
    let path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("skills/")
        .trim()
        .trim_matches('/');
    if path.is_empty() {
        Some(format!("https://github.com/{}::", repo))
    } else {
        Some(format!("https://github.com/{}::{}", repo, path))
    }
}

fn tap_string_to_object(tap: &str) -> serde_json::Value {
    if let Some(spec) = parse_skill_tap_spec(tap) {
        let mut obj = serde_json::Map::new();
        obj.insert("repo".to_string(), serde_json::Value::String(spec.repo));
        obj.insert(
            "path".to_string(),
            serde_json::Value::String(normalize_tap_path_for_storage(&spec.path)),
        );
        obj.insert(
            "url".to_string(),
            serde_json::Value::String(tap.to_string()),
        );
        serde_json::Value::Object(obj)
    } else {
        serde_json::json!({ "url": tap })
    }
}

fn read_skill_taps(path: &std::path::Path) -> Vec<String> {
    if !path.exists() {
        return Vec::new();
    }
    let content = std::fs::read_to_string(path).unwrap_or_else(|_| "[]".to_string());
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    let Ok(value) = parsed else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(arr) => arr
            .into_iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        serde_json::Value::Object(map) => {
            let taps = map.get("taps").cloned().unwrap_or(serde_json::Value::Null);
            match taps {
                serde_json::Value::Array(arr) => arr
                    .into_iter()
                    .filter_map(|item| match item {
                        serde_json::Value::String(s) => Some(s),
                        serde_json::Value::Object(obj) => tap_object_to_string(&obj),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

fn subscription_entry_to_source(entry: &serde_json::Value) -> Option<String> {
    match entry {
        serde_json::Value::String(raw) => {
            let trimmed = raw.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Object(obj) => {
            let source = obj
                .get("source")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("tap").and_then(|v| v.as_str()))
                .or_else(|| obj.get("url").and_then(|v| v.as_str()))?;
            let trimmed = source.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        _ => None,
    }
}

fn read_skill_subscriptions(path: &std::path::Path) -> Vec<String> {
    if !path.exists() {
        return Vec::new();
    }
    let content = std::fs::read_to_string(path).unwrap_or_else(|_| "[]".to_string());
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
    let Ok(value) = parsed else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(subscription_entry_to_source)
            .collect(),
        serde_json::Value::Object(map) => {
            let subscriptions = map
                .get("subscriptions")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            match subscriptions {
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .filter_map(subscription_entry_to_source)
                    .collect(),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

fn write_skill_taps(path: &std::path::Path, taps: &[String]) -> Result<(), AgentError> {
    let serialized_taps: Vec<serde_json::Value> =
        taps.iter().map(|tap| tap_string_to_object(tap)).collect();
    let payload = serde_json::json!({
        "taps": serialized_taps
    });
    let json =
        serde_json::to_string_pretty(&payload).map_err(|e| AgentError::Config(e.to_string()))?;
    std::fs::write(path, format!("{}\n", json)).map_err(|e| AgentError::Io(e.to_string()))?;
    Ok(())
}

fn merged_skill_taps(custom_taps: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = Vec::new();
    for tap in DEFAULT_SKILL_TAPS {
        merged.push((*tap).to_string());
    }
    for tap in custom_taps {
        if !merged.iter().any(|existing| existing == tap) {
            merged.push(tap.clone());
        }
    }
    merged
}

fn subscription_source_to_tap(source: &str) -> Option<String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("https://github.com/") || lower.starts_with("http://github.com/") {
        return parse_skill_tap_spec(trimmed).map(|_| trimmed.to_string());
    }
    if lower.contains("://") {
        return None;
    }
    if let Some((prefix, _)) = trimmed.split_once('/') {
        let p = prefix.trim().to_ascii_lowercase();
        if matches!(
            p.as_str(),
            "official" | "skills.sh" | "lobehub" | "clawhub" | "claude-marketplace" | "github"
        ) {
            return None;
        }
    }
    parse_skill_tap_spec(trimmed).map(|_| trimmed.to_string())
}

fn effective_skill_taps(
    taps_file: &std::path::Path,
    subscriptions_file: &std::path::Path,
) -> Vec<String> {
    let custom_taps = read_skill_taps(taps_file);
    let mut merged = merged_skill_taps(&custom_taps);
    for sub in read_skill_subscriptions(subscriptions_file) {
        // Subscriptions may include non-tap registries; only include values that
        // can be interpreted as GitHub tap specs.
        let Some(tap) = subscription_source_to_tap(&sub) else {
            continue;
        };
        if !merged.iter().any(|existing| existing == &tap) {
            merged.push(tap);
        }
    }
    merged
}

/// Return auto-completion suggestions for a partial slash command.
pub fn autocomplete(partial: &str) -> Vec<&'static str> {
    let mut seen = HashSet::new();
    let mut ranked: Vec<(&'static str, i32)> = Vec::new();
    let query = partial.trim().to_ascii_lowercase();
    for (cmd, desc) in SLASH_COMMANDS {
        if !seen.insert(*cmd) {
            continue;
        }
        if let Some(score) = command_match_score(&query, cmd, desc) {
            ranked.push((cmd, score));
        }
    }
    ranked.sort_by(|(a_cmd, a_score), (b_cmd, b_score)| {
        b_score.cmp(a_score).then_with(|| a_cmd.cmp(b_cmd))
    });
    ranked.into_iter().map(|(cmd, _)| cmd).collect()
}

/// Return contextual auto-completion suggestions for slash commands.
///
/// Unlike [`autocomplete`], this understands command argument position and can
/// suggest nested values like `/swarm run <passes> <mode>`.
pub fn autocomplete_contextual(partial: &str) -> Vec<String> {
    let trimmed_start = partial.trim_start();
    if !trimmed_start.starts_with('/') {
        return Vec::new();
    }
    let trailing_space = trimmed_start
        .chars()
        .last()
        .is_some_and(char::is_whitespace);
    let tokens: Vec<&str> = trimmed_start.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    // First token only: preserve current fuzzy top-level behavior.
    if tokens.len() == 1 && !trailing_space {
        return autocomplete(trimmed_start)
            .into_iter()
            .map(ToString::to_string)
            .collect();
    }

    let Some(cmd) = resolve_completion_command(tokens[0]) else {
        return autocomplete(tokens[0])
            .into_iter()
            .map(ToString::to_string)
            .collect();
    };

    let args = if tokens.len() > 1 {
        tokens[1..].to_vec()
    } else {
        Vec::new()
    };

    let (arg_position, fragment) = if args.is_empty() {
        (0usize, "")
    } else if trailing_space {
        (args.len(), "")
    } else {
        (args.len() - 1, args[args.len() - 1])
    };

    let candidates = if arg_position == 0 {
        command_subcommand_candidates(&cmd)
    } else {
        command_nested_candidates(&cmd, args[0], arg_position)
    };

    if candidates.is_empty() {
        return Vec::new();
    }

    let fragment_lc = fragment.to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for candidate in candidates {
        if !fragment_lc.is_empty() && !candidate.to_ascii_lowercase().starts_with(&fragment_lc) {
            continue;
        }
        let mut parts: Vec<String> = Vec::with_capacity(1 + arg_position + 1);
        parts.push(cmd.clone());
        for i in 0..arg_position {
            if i < args.len() {
                parts.push(args[i].to_string());
            }
        }
        parts.push(candidate.to_string());
        let mut suggestion = parts.join(" ");
        if trailing_space {
            suggestion.push(' ');
        }
        if seen.insert(suggestion.clone()) {
            out.push(suggestion);
        }
    }
    out
}

fn resolve_completion_command(raw: &str) -> Option<String> {
    let canonical = canonical_command(raw);
    if SLASH_COMMANDS.iter().any(|(name, _)| *name == canonical) {
        return Some(canonical.to_string());
    }
    let exact = autocomplete(raw);
    if exact.len() == 1 {
        return exact
            .first()
            .copied()
            .map(canonical_command)
            .map(ToString::to_string);
    }
    None
}

fn command_subcommand_candidates(cmd: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for value in command_subcommand_overrides(cmd) {
        if seen.insert(value.to_string()) {
            out.push(value.to_string());
        }
    }
    for value in inferred_subcommands_from_description(cmd) {
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

fn command_nested_candidates(cmd: &str, subcommand: &str, arg_position: usize) -> Vec<String> {
    let sub = subcommand.to_ascii_lowercase();
    match (cmd, sub.as_str(), arg_position) {
        ("/swarm", "plan", 1) => ["concurrent", "sequential", "graph"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/swarm", "run", 1) => ["1", "2", "4", "8", "16", "32", "64"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/swarm", "run", 2) => ["concurrent", "sequential", "graph"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/swarm", "voters", 1) => ["2", "3", "4", "5", "6", "7", "8"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/quorum", "voters", 1) => ["2", "3", "4", "5", "6", "7", "8"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/objective", "lifecycle", 1) => [
            "status",
            "active",
            "pause",
            "resume",
            "budget-limited",
            "achieved",
            "unmet",
        ]
        .iter()
        .map(|v| (*v).to_string())
        .collect(),
        ("/objective", "behavior", 1) => [
            "status",
            "list",
            "balanced",
            "strict",
            "autonomous",
            "mission",
            "minimal",
            "sigma",
        ]
        .iter()
        .map(|v| (*v).to_string())
        .collect(),
        ("/objective", "profile", 1) => ["status", "list", "general", "me", "set"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/objective", "context", 1) => ["status", "list", "max", "balanced", "fast"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/objective", "simulator", 1) => ["status", "balanced", "strict", "aggressive"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/objective", "ensemble", 1) => ["status", "committee", "single", "debate"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/objective", "ledger", 1) => ["status", "tail", "clear"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/objective", "dag", 1) => ["status", "rebuild", "clear"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/objective", "eval", 1) => ["status", "tail"]
            .iter()
            .map(|v| (*v).to_string())
            .collect(),
        ("/model", "why-not", 1) => [
            "--cap",
            "--min-context",
            "--max-input-cost",
            "--max-output-cost",
            "--budget",
        ]
        .iter()
        .map(|v| (*v).to_string())
        .collect(),
        _ => Vec::new(),
    }
}

fn command_subcommand_overrides(cmd: &str) -> &'static [&'static str] {
    match cmd {
        "/auth" => &["status", "verify", "refresh"],
        "/context" => &["status", "breakdown", "compress"],
        "/pet" => &[
            "status", "on", "off", "toggle", "list", "set", "mood", "dock", "speed",
        ],
        "/agents" => &["status", "pause", "resume", "doctor"],
        "/objective" => &[
            "status",
            "verify",
            "plan",
            "constraints",
            "counterfactual",
            "profile",
            "context",
            "simulator",
            "ensemble",
            "ledger",
            "dag",
            "eval",
            "clear",
            "lifecycle",
            "behavior",
        ],
        "/quorum" => &["status", "on", "off", "voters", "models", "run"],
        "/swarm" => &[
            "status", "plan", "run", "cancel", "artifact", "on", "off", "voters", "models",
        ],
        "/simulate" => &["status"],
        "/timetravel" => &["list", "latest", "goto", "undo", "branch"],
        "/autocompact" => &["status", "now", "governance"],
        "/qos" => &["status", "health", "autotune"],
        "/claims" => &["status", "on", "off"],
        "/curator" => &[
            "status",
            "run",
            "pause",
            "resume",
            "pin",
            "unpin",
            "restore",
            "list-archived",
        ],
        _ => &[],
    }
}

fn inferred_subcommands_from_description(cmd: &str) -> Vec<String> {
    let Some((_, desc)) = SLASH_COMMANDS.iter().find(|(name, _)| *name == cmd) else {
        return Vec::new();
    };
    let mut segments: Vec<String> = Vec::new();
    let mut in_tick = false;
    let mut buf = String::new();
    for ch in desc.chars() {
        if ch == '`' {
            if in_tick && !buf.trim().is_empty() {
                segments.push(buf.clone());
            }
            buf.clear();
            in_tick = !in_tick;
            continue;
        }
        if in_tick {
            buf.push(ch);
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for seg in segments {
        for raw in seg.split('|') {
            let cleaned = raw
                .trim()
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                .trim_start_matches('/');
            if cleaned.is_empty() {
                continue;
            }
            let lc = cleaned.to_ascii_lowercase();
            if lc == cmd.trim_start_matches('/') {
                continue;
            }
            if !lc
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                continue;
            }
            if seen.insert(lc.clone()) {
                out.push(lc);
            }
        }
    }
    out
}

fn command_match_score(query: &str, cmd: &str, desc: &str) -> Option<i32> {
    if query.is_empty() || query == "/" {
        return Some(10);
    }
    let cmd_l = cmd.to_ascii_lowercase();
    let desc_l = desc.to_ascii_lowercase();
    if cmd_l == query {
        return Some(1200);
    }
    if cmd_l.starts_with(query) {
        return Some(1000 - (cmd_l.len().saturating_sub(query.len()) as i32));
    }
    if cmd_l.contains(query) {
        return Some(850 - (cmd_l.len().saturating_sub(query.len()) as i32));
    }
    if let Some(pos) = desc_l.find(query.trim_start_matches('/')) {
        return Some(700 - pos as i32);
    }
    let subseq = subsequence_score(query.trim_start_matches('/'), cmd_l.trim_start_matches('/'));
    if subseq > 0 {
        return Some(500 + subseq);
    }
    None
}

fn subsequence_score(needle: &str, haystack: &str) -> i32 {
    if needle.is_empty() || haystack.is_empty() {
        return 0;
    }
    let mut score = 0i32;
    let mut idx = 0usize;
    let chars: Vec<char> = haystack.chars().collect();
    for ch in needle.chars() {
        let mut found = false;
        while idx < chars.len() {
            if chars[idx] == ch {
                score += 2;
                if idx > 0 && chars[idx - 1] == '-' {
                    score += 1;
                }
                idx += 1;
                found = true;
                break;
            }
            idx += 1;
        }
        if !found {
            return 0;
        }
    }
    score
}

/// Return the help text for a specific slash command.
pub fn help_for(cmd: &str) -> Option<&'static str> {
    SLASH_COMMANDS
        .iter()
        .find(|(name, _)| *name == cmd)
        .map(|(_, desc)| *desc)
}

fn canonical_command(cmd: &str) -> &str {
    match cmd {
        "/clear" => "/new",
        "/reset" => "/new",
        "/compact" => "/compress",
        "/skill" => "/skills",
        "/agent" => "/status",
        "/tasks" => "/kanban",
        "/busy" => "/status",
        "/topic" => "/title",
        "/scheduler" => "/background",
        "/gateway" => "/platforms",
        "/onboard" => "/walkthrough",
        "/reload-skills" => "/reload",
        "/reload_skills" => "/reload",
        "/reload_mcp" => "/reload-mcp",
        "/fork" => "/branch",
        "/tt" => "/timetravel",
        "/snap" => "/snapshot",
        "/set-home" => "/sethome",
        "/footer" => "/statusbar",
        "/indicator" => "/statusbar",
        "/q" => "/queue",
        "/bg" => "/background",
        "/goal" => "/objective",
        "/swarms" => "/swarm",
        "/question" => "/ask",
        "/autocompress" => "/autocompact",
        "/skins" => "/skin",
        "/summary" => "/recap",
        "/whoami" => "/profile",
        "/sb" => "/statusbar",
        "/pilot" => "/autopilot",
        "/rb" => "/runbook",
        "/debug" => "/debug-dump",
        "/exit" => "/quit",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Command dispatcher
// ---------------------------------------------------------------------------

fn quick_command_key(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace('-', "_")
}

fn expand_quick_alias_command(
    quick_commands: &std::collections::BTreeMap<String, hermes_config::QuickCommandConfig>,
    cmd: &str,
    args: &[&str],
) -> Result<(String, Vec<String>), String> {
    let mut current_cmd = cmd.to_string();
    let mut current_args: Vec<String> = args.iter().map(|part| (*part).to_string()).collect();
    loop {
        let key = quick_command_key(&current_cmd);
        let Some(quick) = quick_commands.get(&key) else {
            return Ok((current_cmd, current_args));
        };
        match quick.kind.trim().to_ascii_lowercase().as_str() {
            "alias" => {
                let Some(target) = quick.target.as_deref().filter(|v| !v.trim().is_empty()) else {
                    return Err(format!("Quick command `{key}` has no target defined."));
                };
                let target = target.trim();
                let (target_cmd, embedded_args) = match target.find(char::is_whitespace) {
                    Some(idx) => (&target[..idx], target[idx..].trim()),
                    None => (target, ""),
                };
                let mut merged = Vec::new();
                if !embedded_args.is_empty() {
                    merged.extend(
                        embedded_args
                            .split_whitespace()
                            .map(|part| part.to_string()),
                    );
                }
                merged.extend(current_args);
                current_cmd = target_cmd.to_string();
                current_args = merged;
            }
            other => {
                return Err(format!(
                    "Quick command `{key}` has unsupported kind `{other}`."
                ));
            }
        }
    }
}

/// Handle a slash command.
///
/// `cmd` is the full command token including the `/` prefix
/// (e.g. `/model`, `/new`). `args` are the remaining tokens.
pub async fn handle_slash_command(
    app: &mut App,
    cmd: &str,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let (resolved_cmd, arg_storage) =
        match expand_quick_alias_command(&app.config.quick_commands, cmd, args) {
            Ok(expanded) => expanded,
            Err(message) => {
                emit_command_output(app, message);
                return Ok(CommandResult::Handled);
            }
        };
    let arg_refs: Vec<&str> = arg_storage.iter().map(|part| part.as_str()).collect();
    let args = arg_refs.as_slice();
    let cmd = resolved_cmd.as_str();
    match canonical_command(cmd) {
        "/new" => {
            app.new_session();
            let msg = if cmd.eq_ignore_ascii_case("/reset") {
                format!("[Session reset: {}]", app.session_id)
            } else {
                format!("[New session started: {}]", app.session_id)
            };
            emit_command_output(app, msg);
            Ok(CommandResult::Handled)
        }
        "/retry" => {
            app.retry_last().await?;
            Ok(CommandResult::Handled)
        }
        "/undo" => {
            app.undo_last();
            emit_command_output(app, "[Last exchange undone]");
            Ok(CommandResult::Handled)
        }
        "/history" => handle_history_command(app),
        "/recap" => handle_recap_command(app, args),
        "/context" => handle_context_command(app, args).await,
        "/title" => session::handle_session_compat_command(app, canonical_command(cmd), args),
        "/branch" => session::handle_branch_command(app, args),
        "/timetravel" => session::handle_timetravel_command(app, args),
        "/snapshot" => session::handle_snapshot_command(app, args),
        "/rollback" => session::handle_rollback_command(app, args),
        "/queue" => background::handle_queue_command(app, args),
        "/handoff" => objective::handle_handoff_command(app, args),
        "/steer" => objective::handle_steer_command(app, args),
        "/btw" => objective::handle_btw_command(app, args),
        "/subgoal" => objective::handle_subgoal_command(app, args),
        "/sethome" => objective::handle_sethome_command(app, args),
        "/evolve" => handle_ops_evolve_command(app, args).await,
        "/objective" => objective::handle_objective_command(app, args),
        "/claims" => handle_claims_command(app, args),
        "/quorum" => handle_quorum_command(app, args).await,
        "/swarm" => handle_swarm_command(app, args).await,
        "/simulate" => handle_simulate_command(app, args),
        "/specpatch" => handle_specpatch_command(app, args).await,
        "/heatmap" => handle_heatmap_command(app, args).await,
        "/studio" => handle_studio_command(app, args).await,
        "/ask" => handle_interactive_question_command(app, args),
        "/model" => model::handle_model_command(app, args).await,
        "/auth" => auth_cmd::handle_auth_command(app, args).await,
        "/provider" => handle_provider_command(app).await,
        "/personality" => handle_personality_command(app, args),
        "/profile" | "/whoami" => handle_profile_command(app),
        "/fast" | "/skin" | "/voice" => {
            handle_runtime_ui_mode_command(app, canonical_command(cmd), args)
        }
        "/pet" => handle_pet_command(app, args),
        "/skills" => skills::handle_skills_command(app, args).await,
        "/curator" => handle_curator_command(app, args).await,
        "/tools" => handle_tools_command(app, args),
        "/toolcards" => handle_toolcards_command(app, args),
        "/toolsets" => handle_toolsets_command(app),
        "/plugins" => handle_plugins_command(app),
        "/mcp" => handle_mcp_command(app),
        "/reload" | "/reload-mcp" => handle_reload_command(app, canonical_command(cmd)),
        "/cron" => handle_cron_command(app),
        "/agents" => handle_agents_command(app, args),
        "/kanban" => kanban::handle_kanban_command(app, args),
        "/plan" => handle_plan_command(app, args),
        "/lsp" => handle_lsp_command(app, args),
        "/graph" => handle_graph_command(app, args).await,
        "/qos" => handle_qos_command(app, args).await,
        "/image" => handle_image_command(app, args),
        "/config" => handle_config_command(app, args),
        "/autocompact" => compress::handle_autocompact_command(app, args).await,
        "/compress" => compress::handle_compress_command(app, args).await,
        "/clear-queue" => background::handle_clear_queue_command(app),
        "/usage" => handle_usage_command(app),
        "/insights" => handle_insights_command(app),
        "/stop" => handle_stop_command(app),
        "/status" => handle_status_command(app),
        "/about" => handle_about_command(app),
        "/ops" => handle_ops_command(app, args).await,
        "/telemetry" => auth_cmd::handle_telemetry_command(app, args),
        "/runbook" => handle_runbook_command(app, args),
        "/eval" => handle_ops_eval_command(app, args).await,
        "/autopilot" => handle_ops_autopilot_command(app, args).await,
        "/mission" => background::handle_mission_command(app, args).await,
        "/dashboard" => handle_dashboard_command(app, args).await,
        "/platforms" => handle_platforms_command(app),
        "/integrations" => handle_integrations_command(app, args).await,
        "/commands" => handle_commands_catalog_command(app, args),
        "/boot" => policy::handle_boot_command(app, args).await,
        "/walkthrough" => policy::handle_walkthrough_command(app, args),
        "/triage" => handle_trigger_triage_command(app, args),
        "/subconscious" => handle_subconscious_command(app, args),
        "/log" => handle_log_command(app),
        "/debug-dump" => handle_debug_dump_command(app, args),
        "/dump-format" => handle_dump_format_command(app),
        "/experiment" => handle_experiment_command(app, args),
        "/feedback" => handle_feedback_command(app, args),
        "/restart" => handle_restart_command(app, args),
        "/update" => handle_update_command(app, args).await,
        "/redraw" => handle_redraw_command(app),
        "/paste" => handle_paste_command(app, args),
        "/gquota" => handle_gquota_command(app, args).await,
        "/approve" => handle_approve_command(app, args),
        "/deny" => handle_deny_command(app, args),
        "/copy" => handle_copy_command(app),
        "/save" => session::handle_save_command(app, args),
        "/load" => session::handle_load_command(app, args),
        "/resume" => session::handle_resume_command(app, args),
        "/sessions" => session::handle_sessions_command(app, args),
        "/background" => background::handle_background_command(app, args),
        "/mouse" => handle_mouse_command(app, args),
        "/verbose" => handle_verbose_command(app),
        "/statusbar" => handle_statusbar_command(app),
        "/yolo" => handle_yolo_command(app),
        "/browser" => browser::handle_browser_command(app, args).await,
        "/reasoning" => handle_reasoning_command(app, args),
        "/raw" => handle_raw_command(app, args),
        "/policy" => policy::handle_policy_command(app, args),
        "/help" => {
            print_help(app);
            Ok(CommandResult::Handled)
        }
        "/acp_server" => crate::acp_command::handle_acp_command(app, args).await,
        "/quit" | "/exit" => {
            emit_command_output(app, "Goodbye!");
            Ok(CommandResult::Quit)
        }
        _ => {
            emit_command_output(
                app,
                format!(
                    "Unknown command: {}. Type /help for available commands.",
                    cmd
                ),
            );
            Ok(CommandResult::Handled)
        }
    }
}

fn handle_toolcards_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let action = args.first().copied().unwrap_or("help");
    let msg = match action {
        "export" => {
            "Tool-card export is handled by the interactive TUI modal loop. In TUI, run `/toolcards export` to write `~/.hermes-agent-ultra/logs/toolcards-export.txt`.".to_string()
        }
        _ => "Tool-card controls:\n  /toolcards export   Export current tool-card transcript".to_string(),
    };
    emit_command_output(app, msg);
    Ok(CommandResult::Handled)
}

pub(crate) fn emit_command_output(app: &mut App, text: impl Into<String>) {
    let rendered = text.into();
    if app.stream_handle.is_some() {
        app.push_ui_assistant(rendered);
    } else {
        println!("{}", rendered);
    }
}

fn format_personality_catalog(
    current_personality: Option<&str>,
    builtin_descriptions: &[(&str, &str)],
) -> String {
    let mut out = String::from("## Built-in personalities\n\n");
    if let Some(current) = current_personality.filter(|v| !v.trim().is_empty()) {
        out.push_str(&format!("Current: `{}`\n\n", current));
    } else {
        out.push_str("Current: `(none)`\n\n");
    }
    out.push_str("Use `/personality <name>` to switch.\n\n");
    for (name, usage) in builtin_descriptions {
        out.push_str(&format!("- `{}`\n  {}\n\n", name, usage));
    }
    out.trim_end().to_string()
}

fn handle_personality_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let builtin = hermes_agent::builtin_personality_names();
    let builtin_descriptions = hermes_agent::builtin_personality_descriptions();
    if args.is_empty() {
        emit_command_output(
            app,
            format_personality_catalog(app.current_personality.as_deref(), builtin_descriptions),
        );
    } else if args.len() == 1 && args[0].eq_ignore_ascii_case("list") {
        emit_command_output(
            app,
            format_personality_catalog(app.current_personality.as_deref(), builtin_descriptions),
        );
    } else {
        let name = args.join(" ");
        app.switch_personality(&name);
        let mut response = format!("Switched personality to `{}`.", name);
        if !name.contains(char::is_whitespace)
            && !name.eq_ignore_ascii_case("default")
            && !builtin.iter().any(|n| n.eq_ignore_ascii_case(&name))
        {
            response.push_str(&format!(
                "\n\nNote: `{}` is not built-in. Hermes will look for `personalities/{}.md` or treat inline text as compatibility mode.",
                name, name,
            ));
        }
        emit_command_output(app, response);
    }
    Ok(CommandResult::Handled)
}

// SkillsSlashInvocation + related fns moved to skills.rs

async fn handle_curator_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let skills_dir = hermes_config::hermes_home().join("skills");
    let curator_config = curator_config_from_app(app);

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();

    match sub.as_str() {
        "status" | "" => {
            let rows = hermes_skills::agent_created_report(&skills_dir);
            let state = hermes_skills::load_curator_state(&skills_dir);

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
                emit_command_output(app, &out);
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
                        out.push_str(&format!(
                            "  {pin_mark}`{name}` — {activity} uses\n",
                            name = row.name,
                            activity = row.activity_count
                        ));
                    }
                    out.push('\n');
                }

                // Least recently active top 5
                let mut with_last: Vec<_> = rows
                    .iter()
                    .filter(|r| r.last_activity_at.is_some())
                    .collect();
                with_last.sort_by_key(|r| r.last_activity_at.as_deref().unwrap_or(""));
                let least_active: Vec<_> = with_last.iter().take(5).collect();
                if !least_active.is_empty() {
                    out.push_str("**Least recently active:**\n");
                    for row in &least_active {
                        let pin_mark = if row.pinned { "📌 " } else { "  " };
                        let state_tag = match row.state.as_str() {
                            "archived" => " [archived]",
                            "stale" => " [stale]",
                            _ => "",
                        };
                        let last = row.last_activity_at.as_deref().unwrap_or("never");
                        out.push_str(&format!(
                            "  {pin_mark}`{name}`{state_tag} — last: {last}\n",
                            name = row.name
                        ));
                    }
                    out.push('\n');
                }

                // Full list
                out.push_str("**All skills:**\n");
                for row in &rows {
                    let pin_mark = if row.pinned { "📌 " } else { "   " };
                    let state_tag = match row.state.as_str() {
                        "archived" => " [archived]",
                        "stale" => " [stale]",
                        _ => "",
                    };
                    out.push_str(&format!(
                        "{pin_mark}`{name}`{state_tag} — activity: {activity}\n",
                        pin_mark = pin_mark,
                        name = row.name,
                        state_tag = state_tag,
                        activity = row.activity_count,
                    ));
                }

                // Config summary
                out.push('\n');
                out.push_str(&format!(
                    "curator: {}\n",
                    curator_status_label(&curator_config, &state)
                ));
                out.push_str(&format!("  runs: {}\n", state.run_count));
                if let Some(ref last) = state.last_run_at {
                    out.push_str(&format!("  last run: {}\n", last));
                }
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

                let pinned_count = rows.iter().filter(|r| r.pinned).count();
                if pinned_count > 0 {
                    out.push_str(&format!(
                        "\n📌 {pinned_count} pinned (exempt from automatic curation)"
                    ));
                }
                emit_command_output(app, out.trim_end());
            }
        }
        "pin" | "unpin" => {
            let skill_name = args.get(1).map(|s| s.trim()).unwrap_or("");
            if skill_name.is_empty() {
                emit_command_output(app, format!("Usage: /curator {} <skill-name>", sub));
                return Ok(CommandResult::Handled);
            }
            let pinned = sub == "pin";
            match hermes_skills::set_pinned(&skills_dir, skill_name, pinned) {
                Ok(()) => {
                    let action = if pinned { "📌 Pinned" } else { "Unpinned" };
                    emit_command_output(app, format!("{action} skill `{skill_name}`."));
                }
                Err(e) => {
                    emit_command_output(app, format!("Failed: {e}"));
                }
            }
        }
        "restore" => {
            let skill_name = args.get(1).map(|s| s.trim()).unwrap_or("");
            if skill_name.is_empty() {
                emit_command_output(app, "Usage: /curator restore <skill-name>");
                return Ok(CommandResult::Handled);
            }
            match hermes_skills::restore_skill(&skills_dir, skill_name) {
                Ok((true, msg)) => {
                    emit_command_output(app, format!("✅ {msg}"));
                }
                Ok((false, msg)) => {
                    emit_command_output(app, format!("❌ {msg}"));
                }
                Err(e) => {
                    emit_command_output(app, format!("Failed: {e}"));
                }
            }
        }
        "archive" => {
            let skill_name = args.get(1).map(|s| s.trim()).unwrap_or("");
            if skill_name.is_empty() {
                emit_command_output(app, "Usage: /curator archive <skill-name>");
                return Ok(CommandResult::Handled);
            }
            match hermes_skills::archive_skill(&skills_dir, skill_name) {
                Ok((true, msg)) => {
                    emit_command_output(app, format!("✅ {msg}"));
                }
                Ok((false, msg)) => {
                    emit_command_output(app, format!("❌ {msg}"));
                }
                Err(e) => {
                    emit_command_output(app, format!("Failed: {e}"));
                }
            }
        }
        "list-archived" => {
            let archive_dir = skills_dir.join(".archive");
            if !archive_dir.exists() {
                emit_command_output(app, "No archived skills found.");
                return Ok(CommandResult::Handled);
            }
            match std::fs::read_dir(&archive_dir) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .flatten()
                        .filter_map(|e| {
                            if e.path().is_dir() {
                                Some(e.file_name().to_string_lossy().to_string())
                            } else {
                                None
                            }
                        })
                        .collect();
                    names.sort();
                    if names.is_empty() {
                        emit_command_output(app, "No archived skills found.");
                    } else {
                        let mut out = format!("## Archived skills ({})\n\n", names.len());
                        for name in &names {
                            out.push_str(&format!("  - `{name}`\n"));
                        }
                        out.push_str("\nUse `/curator restore <name>` to restore a skill.");
                        emit_command_output(app, out.trim_end());
                    }
                }
                Err(e) => {
                    emit_command_output(app, format!("Failed to read archive directory: {e}"));
                }
            }
        }
        "pause" => match hermes_skills::set_paused(&skills_dir, true) {
            Ok(()) => {
                emit_command_output(
                    app,
                    "✓ Curator paused. Automatic runs will be skipped until resumed.",
                );
            }
            Err(e) => {
                emit_command_output(app, format!("Failed to pause curator: {e}"));
            }
        },
        "resume" => match hermes_skills::set_paused(&skills_dir, false) {
            Ok(()) => {
                emit_command_output(
                    app,
                    "✓ Curator resumed. Automatic runs will proceed on schedule.",
                );
            }
            Err(e) => {
                emit_command_output(app, format!("Failed to resume curator: {e}"));
            }
        },
        "run" => {
            // Parse flags
            let mut dry_run = false;
            let mut background = false;
            for arg in &args[1..] {
                match *arg {
                    "--dry-run" => dry_run = true,
                    "--background" => background = true,
                    _ => {}
                }
            }

            if !curator_config.enabled {
                emit_command_output(app, "Curator is disabled in configuration.");
                return Ok(CommandResult::Handled);
            }

            if background {
                // Background mode: spawn LLM pass in background, CLI returns immediately
                let skills_dir_clone = skills_dir.clone();
                let config_clone = curator_config.clone();
                let dry_run_clone = dry_run;
                let logs_dir = hermes_config::hermes_home().join("logs");
                tokio::spawn(async move {
                    let record = hermes_skills::run_curator_review::<
                        _,
                        std::future::Ready<
                            Result<hermes_skills::CuratorReviewResult, hermes_skills::CuratorError>,
                        >,
                    >(
                        &skills_dir_clone,
                        &config_clone,
                        dry_run_clone,
                        None::<
                            fn(
                                String,
                            ) -> std::future::Ready<
                                Result<
                                    hermes_skills::CuratorReviewResult,
                                    hermes_skills::CuratorError,
                                >,
                            >,
                        >,
                    )
                    .await;
                    if let Ok(ref record) = record {
                        // Write report
                        let report = build_curator_run_report(record, None, None);
                        if let Ok(report_dir) =
                            hermes_skills::write_curator_report(&report, &logs_dir)
                        {
                            let mut state = hermes_skills::load_curator_state(&skills_dir_clone);
                            state.last_report_path = Some(report_dir.to_string_lossy().to_string());
                            let _ = hermes_skills::save_curator_state(&skills_dir_clone, &state);
                        }
                    }
                });
                emit_command_output(
                    app,
                    "Curator LLM pass running in background — check `/curator status` later.",
                );
                return Ok(CommandResult::Handled);
            }

            // Synchronous mode: execute auto transitions (LLM pass deferred to future update)
            let result = hermes_skills::apply_automatic_transitions(&skills_dir, &curator_config);

            // Format output
            let mut lines = Vec::new();
            if dry_run {
                lines.push("── curator dry-run ──".to_string());
            } else {
                lines.push("── curator run ──".to_string());
            }
            lines.push(format!("  checked:      {}", result.checked));
            lines.push(format!("  marked stale: {}", result.marked_stale));
            lines.push(format!("  archived:     {}", result.archived));
            lines.push(format!("  reactivated:  {}", result.reactivated));

            // Update state and write report (non dry-run)
            if !dry_run {
                let mut state = hermes_skills::load_curator_state(&skills_dir);
                state.last_run_at = Some(chrono::Utc::now().to_rfc3339());
                state.run_count += 1;
                state.last_run_summary = Some(format!(
                    "auto: {} checked, {} marked stale, {} archived, {} reactivated",
                    result.checked, result.marked_stale, result.archived, result.reactivated
                ));

                // Write curator report
                let report = build_curator_run_report_from_transitions(&result);
                let logs_dir = hermes_config::hermes_home().join("logs");
                if let Ok(report_dir) = hermes_skills::write_curator_report(&report, &logs_dir) {
                    state.last_report_path = Some(report_dir.to_string_lossy().to_string());
                    lines.push(format!("  report: {}", report_dir.display()));
                }

                let _ = hermes_skills::save_curator_state(&skills_dir, &state);
                lines.push(String::new());
                lines.push(
                    "State updated. (LLM review pass will be available in a future update)"
                        .to_string(),
                );
            } else {
                lines.push(String::new());
                lines.push("(dry-run: no state changes persisted)".to_string());
            }

            let output = lines.join("\n");
            emit_command_output(app, &output);
        }
        "backup" => match backup_skills(&skills_dir) {
            Ok(path) => {
                emit_command_output(app, format!("✅ Skills backed up to: {}", path.display()));
            }
            Err(e) => {
                emit_command_output(app, format!("Failed to backup skills: {e}"));
            }
        },
        "rollback" => {
            let backup_name = args.get(1).map(|s| s.trim()).unwrap_or("");
            if backup_name.is_empty() {
                // List available backups
                match list_backups(&skills_dir) {
                    Ok(backups) => {
                        if backups.is_empty() {
                            emit_command_output(
                                app,
                                "No backups found.\n\nUse `/curator backup` to create a snapshot first.",
                            );
                        } else {
                            let mut out = format!("## Available backups ({})\n\n", backups.len());
                            for (name, path) in &backups {
                                out.push_str(&format!("  - `{name}` ({})\n", path.display()));
                            }
                            out.push_str("\nUse `/curator rollback <name>` to restore a backup.");
                            emit_command_output(app, &out);
                        }
                    }
                    Err(e) => {
                        emit_command_output(app, format!("Failed to list backups: {e}"));
                    }
                }
            } else {
                match rollback_skills(&skills_dir, backup_name) {
                    Ok(()) => {
                        emit_command_output(
                            app,
                            format!("✅ Rolled back to backup: {backup_name}"),
                        );
                    }
                    Err(e) => {
                        emit_command_output(app, format!("Failed to rollback: {e}"));
                    }
                }
            }
        }
        "prune" => {
            let force = args.get(1).map_or(false, |a| *a == "--force");
            if !force {
                emit_command_output(
                    app,
                    "⚠️  This will permanently delete stale/archived skills.\nUse `/curator prune --force` to confirm.",
                );
                return Ok(CommandResult::Handled);
            }
            let result = hermes_skills::apply_automatic_transitions(&skills_dir, &curator_config);
            let mut lines = vec!["── curator prune ──".to_string()];
            lines.push(format!("  checked:      {}", result.checked));
            lines.push(format!("  archived:     {}", result.archived));
            lines.push(format!("  marked stale: {}", result.marked_stale));
            lines.push(format!("  reactivated:  {}", result.reactivated));
            emit_command_output(app, &lines.join("\n"));
        }
        _ => {
            let help = format!(
                "## /curator — Background skill maintenance\n\n\
                 Subcommands:\n\
                   `/curator status`      — Show agent-created skills report\n\
                   `/curator pin <name>`   — Pin a skill (exempt from auto-curation)\n\
                   `/curator unpin <name>` — Unpin a skill\n\
                   `/curator archive <name>` — Archive a skill to .archive/\n\
                   `/curator restore <name>` — Restore an archived skill\n\
                   `/curator list-archived` — List archived skills\n\
                   `/curator run`          — Trigger curator review\n\
                   `/curator run --dry-run` — Preview without persisting changes\n\
                   `/curator run --background` — Run curator in background\n\
                   `/curator pause`        — Pause automatic curator runs\n\
                   `/curator resume`       — Resume automatic curator runs\n\
                   `/curator backup`       — Create a backup snapshot\n\
                   `/curator rollback [name]` — List or restore a backup\n\
                   `/curator prune --force` — Prune stale/archived skills\n\
                 \nUnknown subcommand: `{unknown}`\n\
                 Try `/curator` (no args) for status overview.",
                unknown = sub,
            );
            emit_command_output(app, help);
        }
    }

    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Curator helper functions
// ---------------------------------------------------------------------------

/// Build `hermes_skills::CuratorConfig` from the app's gateway config.
fn curator_config_from_app(app: &App) -> hermes_skills::CuratorConfig {
    let gc = &app.config.curator;
    hermes_skills::CuratorConfig {
        enabled: gc.enabled,
        interval_hours: gc.interval_hours,
        min_idle_hours: gc.min_idle_hours,
        stale_after_days: gc.stale_after_days,
        archive_after_days: gc.archive_after_days,
        prune_builtins: gc.prune_builtins,
    }
}

/// Return a human-readable status label for the curator.
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

/// Calculate the remaining time until the next eligible curator run.
/// Returns `None` if the curator is paused, disabled, or has never been run.
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

/// Build a `CuratorRunReport` from a `CuratorRunRecord`.
fn build_curator_run_report(
    record: &hermes_skills::CuratorRunRecord,
    model: Option<String>,
    provider: Option<String>,
) -> hermes_skills::CuratorRunReport {
    let before_count = 0u64; // snapshot before run would require pre-count
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

/// Build a `CuratorRunReport` from just the auto-transition result.
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

/// Create a timestamped backup snapshot of the skills directory.
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

    // Copy the skills directory (excluding .curator_backups and .archive)
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

/// Recursively copy a directory.
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

/// List available backups, sorted by timestamp (newest first).
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
    // Sort by name descending (newest first)
    backups.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(backups)
}

/// Rollback skills from a backup snapshot.
fn rollback_skills(skills_dir: &std::path::Path, backup_name: &str) -> Result<(), std::io::Error> {
    let backup_dir = skills_dir.join(".curator_backups").join(backup_name);
    if !backup_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("backup not found: {}", backup_name),
        ));
    }

    // Remove current skills (except .curator_backups, .archive, .curator_state)
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

    // Restore from backup
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

fn handle_tools_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args
        .first()
        .is_some_and(|sub| sub.eq_ignore_ascii_case("trust"))
    {
        let counters = app.tool_registry.policy_counters();
        let tools = app.tool_registry.list_tools();
        let mut risk: Vec<(String, i32, String)> = tools
            .iter()
            .map(|tool| {
                let mut score = 100i32;
                if !tool.env_deps.is_empty() {
                    score -= 15;
                }
                if matches!(
                    tool.name.as_str(),
                    "terminal" | "execute_code" | "shell_exec" | "bash" | "python_exec"
                ) {
                    score -= 35;
                }
                if tool.toolset.eq_ignore_ascii_case("network")
                    || tool.name.contains("webhook")
                    || tool.name.contains("http")
                {
                    score -= 20;
                }
                if tool.name.contains("secrets")
                    || tool.name.contains("token")
                    || tool.name.contains("oauth")
                {
                    score -= 25;
                }
                score = score.clamp(0, 100);
                let tier = if score >= 80 {
                    "low-risk"
                } else if score >= 55 {
                    "moderate-risk"
                } else {
                    "high-risk"
                };
                (tool.name.clone(), score, tier.to_string())
            })
            .collect();
        risk.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut out = String::new();
        out.push_str("Tool trust scorecard (heuristic)\n");
        out.push_str("--------------------------------\n");
        let _ = writeln!(
            out,
            "policy_counters: allow={} deny={} audit_only={} simulate={} would_block={}",
            counters.allow,
            counters.deny,
            counters.audit_only,
            counters.simulate,
            counters.would_block
        );
        let _ = writeln!(out, "registered_tools={}", risk.len());
        for (name, score, tier) in risk.iter().take(20) {
            let _ = writeln!(out, "- {name:<28} score={score:>3} tier={tier}");
        }
        out.push_str("\nUse `/ops status` and `/raw trace verify` for live enforcement + trace integrity signals.");
        emit_command_output(app, out.trim_end());
        return Ok(CommandResult::Handled);
    }

    let tools = app.tool_registry.list_tools();
    if tools.is_empty() {
        emit_command_output(app, "No tools registered.");
    } else {
        let mut out = format!("Registered tools ({}):\n", tools.len());
        for tool in &tools {
            out.push_str(&format!("- `{}` — {}\n", tool.name, tool.description));
        }
        out.push_str("\n\nUse `/tools trust` for a risk/score summary.");
        emit_command_output(app, out.trim_end());
    }
    Ok(CommandResult::Handled)
}

fn handle_config_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        // Show full config
        let config_json = serde_json::to_string_pretty(&*app.config)
            .unwrap_or_else(|e| format!("<serialization error: {}>", e));
        emit_command_output(app, config_json);
    } else {
        match args[0] {
            "get" => {
                if args.len() < 2 {
                    emit_command_output(app, "Usage: /config get <key>");
                } else {
                    let key = args[1];
                    let value = get_config_value(app, key);
                    match value {
                        Some(v) => emit_command_output(app, format!("{} = {}", key, v)),
                        None => emit_command_output(
                            app,
                            format!("Key '{}' not found in configuration.", key),
                        ),
                    }
                }
            }
            "set" => {
                if args.len() < 3 {
                    emit_command_output(app, "Usage: /config set <key> <value>");
                } else {
                    let key = args[1];
                    let value = args[2..].join(" ");
                    if set_config_value(app, key, &value) {
                        emit_command_output(app, format!("Set {} = {}", key, value));
                    } else {
                        emit_command_output(app, format!("Unknown configuration key: {}", key));
                    }
                }
            }
            _ => {
                emit_command_output(
                    app,
                    format!("Unknown config action '{}'. Use 'get' or 'set'.", args[0]),
                );
            }
        }
    }
    Ok(CommandResult::Handled)
}

/// Get a configuration value by dotted key path.
fn get_config_value(app: &App, key: &str) -> Option<String> {
    match key {
        "model" => app.config.model.clone(),
        "personality" => app.config.personality.clone(),
        "max_turns" => Some(app.config.max_turns.to_string()),
        "system_prompt" => app.config.system_prompt.clone(),
        _ => None,
    }
}

/// Set a configuration value by dotted key path.
fn set_config_value(app: &mut App, key: &str, value: &str) -> bool {
    match key {
        "model" => {
            app.config = Arc::new({
                let mut cfg = (*app.config).clone();
                cfg.model = Some(value.to_string());
                cfg
            });
            app.switch_model(value);
            true
        }
        "personality" => {
            app.config = Arc::new({
                let mut cfg = (*app.config).clone();
                cfg.personality = Some(value.to_string());
                cfg
            });
            app.switch_personality(value);
            true
        }
        "max_turns" => {
            if let Ok(turns) = value.parse::<u32>() {
                app.config = Arc::new({
                    let mut cfg = (*app.config).clone();
                    cfg.max_turns = turns;
                    cfg
                });
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn handle_usage_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let display = app.agent.session_usage_display();
    let mut body = hermes_agent::format_usage_command_text(&display);
    if display.calls == 0 {
        let estimated_tokens: usize = app
            .messages
            .iter()
            .map(|m| m.content.as_ref().map_or(0, |c| c.len()) / 4)
            .sum();
        body.push_str(&format!(
            "\n\n(Transcript heuristic ~{} tokens — no provider usage yet.)",
            estimated_tokens
        ));
    }
    emit_command_output(app, body);
    Ok(CommandResult::Handled)
}

fn handle_stop_command(app: &mut App) -> Result<CommandResult, AgentError> {
    app.interrupt_controller.interrupt(None);
    emit_command_output(
        app,
        "[Stopping current agent execution]\nAgent execution halted. You can continue typing or use /retry.",
    );
    Ok(CommandResult::Handled)
}

fn handle_status_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let msg_count = app.messages.len();
    let turns = app
        .messages
        .iter()
        .filter(|m| m.role == hermes_core::MessageRole::User)
        .count();
    let usage = app.agent.session_usage_metrics();
    let token_line = if usage.api_calls > 0 {
        format!(
            "  Session tokens: {} total ({} in / {} out, {} API calls)",
            usage.total_tokens, usage.input_tokens, usage.output_tokens, usage.api_calls
        )
    } else {
        let estimated_tokens: usize = app
            .messages
            .iter()
            .map(|m| m.content.as_ref().map_or(0, |c| c.len()) / 4)
            .sum();
        format!("  Est. tokens:   ~{} (no API calls yet)", estimated_tokens)
    };

    emit_command_output(
        app,
        format!(
            "Session Status\n  ID:            {}\n  Model:         {}\n  Personality:   {}\n  Turns:         {}\n  Messages:      {}\n{}\n  Max turns:     {}",
            app.session_id,
            app.current_model,
            app.current_personality.as_deref().unwrap_or("(none)"),
            turns,
            msg_count,
            token_line,
            app.config.max_turns
        ),
    );
    Ok(CommandResult::Handled)
}

fn discover_repo_root_for_about() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("HERMES_REPO_ROOT") {
        let path = PathBuf::from(explicit.trim());
        if path.exists() {
            return Some(path);
        }
    }

    let mut probes: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        probes.push(cwd);
    }
    probes.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    for probe in probes {
        for candidate in probe.ancestors() {
            if candidate.join("docs/parity").exists() && candidate.join("README.md").exists() {
                return Some(candidate.to_path_buf());
            }
        }
    }
    None
}

fn read_json_file(path: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&raw).ok()
}

fn json_value_at_path<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn json_str_at_path(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    json_value_at_path(value, path)?
        .as_str()
        .map(|s| s.to_string())
}

fn json_u64_at_path(value: &serde_json::Value, path: &[&str]) -> Option<u64> {
    json_value_at_path(value, path)?.as_u64()
}

fn latest_upstream_sync_report(report_dir: &Path) -> Option<PathBuf> {
    let mut reports: Vec<PathBuf> = std::fs::read_dir(report_dir)
        .ok()?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_string_lossy();
            if name.starts_with("upstream-sync-") && name.ends_with(".txt") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    reports.sort();
    reports.into_iter().last()
}

fn parse_sync_report_metadata(path: &Path) -> (std::collections::HashMap<String, String>, usize) {
    let mut meta = std::collections::HashMap::new();
    let mut pending_commit_lines = 0usize;
    let raw = std::fs::read_to_string(path).unwrap_or_default();

    let mut in_pending_section = false;
    let mut in_pending_block = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if !in_pending_section {
            if trimmed.starts_with("## Pending Upstream Commits") {
                in_pending_section = true;
                continue;
            }
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim();
                if !key.is_empty()
                    && key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                {
                    meta.insert(key.to_string(), v.trim().to_string());
                }
            }
            continue;
        }

        if trimmed == "```" {
            if !in_pending_block {
                in_pending_block = true;
            } else {
                break;
            }
            continue;
        }
        if in_pending_block && !trimmed.is_empty() {
            pending_commit_lines = pending_commit_lines.saturating_add(1);
        }
    }

    (meta, pending_commit_lines)
}

pub(crate) fn yes_no(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

fn handle_about_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let mut out = String::new();
    let _ = writeln!(out, "Hermes Agent Ultra — About");
    let _ = writeln!(out, "  Version:         {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "  Session model:   {}", app.current_model);
    let _ = writeln!(
        out,
        "  Personality:     {}",
        app.current_personality.as_deref().unwrap_or("(none)")
    );
    if let Ok(exe) = std::env::current_exe() {
        let _ = writeln!(out, "  Binary:          {}", exe.display());
    }
    if let Ok(cwd) = std::env::current_dir() {
        let _ = writeln!(out, "  Current dir:     {}", cwd.display());
    }

    let raw_mode = app.tool_registry.raw_mode_state();
    let policy_mode = std::env::var("HERMES_TOOL_POLICY_MODE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "enforce".to_string());
    let policy_preset = std::env::var("HERMES_TOOL_POLICY_PRESET")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "off".to_string());

    let has_contextlattice_mcp = app.config.mcp_servers.iter().any(|entry| {
        let name_hit = entry.name.to_ascii_lowercase().contains("contextlattice");
        let url_hit = entry
            .url
            .as_ref()
            .map(|u| u.to_ascii_lowercase().contains("contextlattice"))
            .unwrap_or(false);
        name_hit || url_hit
    });

    let _ = writeln!(out);
    let _ = writeln!(out, "Enabled Ultra Features:");
    let _ = writeln!(
        out,
        "  - RTK raw-mode: enabled={} once={}",
        yes_no(raw_mode.enabled),
        yes_no(raw_mode.once_pending)
    );
    let _ = writeln!(
        out,
        "  - Tool policy: mode={} preset={}",
        policy_mode, policy_preset
    );
    let _ = writeln!(
        out,
        "  - Code indexing: {} (max_files={}, max_symbols={})",
        yes_no(app.config.agent.code_index_enabled),
        app.config.agent.code_index_max_files,
        app.config.agent.code_index_max_symbols
    );
    let _ = writeln!(
        out,
        "  - LSP context injection: {} (max_chars={})",
        yes_no(app.config.agent.lsp_context_enabled),
        app.config.agent.lsp_context_max_chars
    );
    let _ = writeln!(
        out,
        "  - Background review loop: {}",
        yes_no(app.config.agent.background_review_enabled)
    );
    let _ = writeln!(out, "  - Multi-registry skills: yes");
    let _ = writeln!(out, "  - Skill security scanning: yes");
    let _ = writeln!(
        out,
        "  - ContextLattice MCP configured: {}",
        yes_no(has_contextlattice_mcp)
    );

    if let Some(repo_root) = discover_repo_root_for_about() {
        let report_dir = repo_root.join(".sync-reports");
        let workstream_path = repo_root.join("docs/parity/workstream-status.json");
        let queue_path = repo_root.join("docs/parity/upstream-missing-queue.json");
        let proof_path = repo_root.join("docs/parity/global-parity-proof.json");

        let mut upstream_ref = String::from("unknown");
        let mut upstream_sha = String::from("unknown");
        let mut workstream_generated = String::from("unknown");
        if let Some(workstream) = read_json_file(&workstream_path) {
            if let Some(v) = json_str_at_path(&workstream, &["upstream_ref"]) {
                upstream_ref = v;
            }
            if let Some(v) = json_str_at_path(&workstream, &["upstream_sha"]) {
                upstream_sha = v;
            }
            if let Some(v) = json_str_at_path(&workstream, &["generated_at_utc"]) {
                workstream_generated = v;
            }
        }

        let mut queue_pending = 0u64;
        let mut queue_ported = 0u64;
        let mut queue_superseded = 0u64;
        if let Some(queue) = read_json_file(&queue_path) {
            queue_pending =
                json_u64_at_path(&queue, &["summary", "by_disposition", "pending"]).unwrap_or(0);
            queue_ported =
                json_u64_at_path(&queue, &["summary", "by_disposition", "ported"]).unwrap_or(0);
            queue_superseded =
                json_u64_at_path(&queue, &["summary", "by_disposition", "superseded"]).unwrap_or(0);
        }

        let mut release_gate_pass = String::from("unknown");
        let mut ci_gate_pass = String::from("unknown");
        if let Some(proof) = read_json_file(&proof_path) {
            if let Some(v) =
                json_value_at_path(&proof, &["release_gate", "pass"]).and_then(|v| v.as_bool())
            {
                release_gate_pass = yes_no(v).to_string();
            }
            if let Some(v) =
                json_value_at_path(&proof, &["ci_gate", "pass"]).and_then(|v| v.as_bool())
            {
                ci_gate_pass = yes_no(v).to_string();
            }
        }

        let mut latest_report_name = String::from("none");
        let mut latest_origin_sha = String::from("unknown");
        let mut latest_upstream_sha = String::from("unknown");
        let mut latest_timestamp = String::from("unknown");
        let mut latest_pending_count = 0usize;
        if let Some(report_path) = latest_upstream_sync_report(&report_dir) {
            latest_report_name = report_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| report_path.display().to_string());
            let (meta, pending_count) = parse_sync_report_metadata(&report_path);
            latest_pending_count = pending_count;
            if let Some(v) = meta.get("origin_sha") {
                latest_origin_sha = v.clone();
            }
            if let Some(v) = meta.get("upstream_sha") {
                latest_upstream_sha = v.clone();
            }
            if let Some(v) = meta.get("timestamp_utc") {
                latest_timestamp = v.clone();
            }
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "Parity Snapshot:");
        let _ = writeln!(out, "  - Repo root: {}", repo_root.display());
        let _ = writeln!(out, "  - Upstream ref: {}", upstream_ref);
        let _ = writeln!(out, "  - Upstream sha: {}", upstream_sha);
        let _ = writeln!(
            out,
            "  - Workstream report generated_at: {}",
            workstream_generated
        );
        let _ = writeln!(
            out,
            "  - Queue (pending/ported/superseded): {}/{}/{}",
            queue_pending, queue_ported, queue_superseded
        );
        let _ = writeln!(
            out,
            "  - Gate status (release/ci): {}/{}",
            release_gate_pass, ci_gate_pass
        );
        let _ = writeln!(out, "  - Latest sync report: {}", latest_report_name);
        let _ = writeln!(out, "  - Latest sync timestamp_utc: {}", latest_timestamp);
        let _ = writeln!(out, "  - Latest report origin_sha: {}", latest_origin_sha);
        let _ = writeln!(
            out,
            "  - Latest report upstream_sha: {}",
            latest_upstream_sha
        );
        let _ = writeln!(
            out,
            "  - Pending upstream commits in latest report: {}",
            latest_pending_count
        );
    } else {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Parity Snapshot: unavailable (run from a source checkout to load docs/parity + .sync-reports)."
        );
    }

    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

// SkillsExecutionTier + tier fns moved to skills.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepoReviewBudgetProfile {
    Balanced,
    Aggressive,
    Relaxed,
    Off,
}

impl RepoReviewBudgetProfile {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "balanced" => Some(Self::Balanced),
            "aggressive" => Some(Self::Aggressive),
            "relaxed" => Some(Self::Relaxed),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Aggressive => "aggressive",
            Self::Relaxed => "relaxed",
            Self::Off => "off",
        }
    }
}

const REPO_REVIEW_BUDGET_ENV_REPEAT_THRESHOLD: &str = "HERMES_REPO_REVIEW_REPEAT_STREAK_THRESHOLD";
const REPO_REVIEW_BUDGET_ENV_LOW_SIGNAL_THRESHOLD: &str =
    "HERMES_REPO_REVIEW_LOW_SIGNAL_STREAK_THRESHOLD";
const REPO_REVIEW_BUDGET_ENV_KEEP_REPEAT: &str = "HERMES_REPO_REVIEW_KEEP_LIMIT_REPEAT";
const REPO_REVIEW_BUDGET_ENV_KEEP_LOW_SIGNAL: &str = "HERMES_REPO_REVIEW_KEEP_LIMIT_LOW_SIGNAL";
const REPO_REVIEW_BUDGET_ENV_MIN_SIGNAL_SCORE: &str = "HERMES_REPO_REVIEW_MIN_SIGNAL_SCORE";
const REPO_REVIEW_BUDGET_ENV_PROFILE: &str = "HERMES_REPO_REVIEW_BUDGET_PROFILE";

#[derive(Debug, Clone, PartialEq)]
struct RepoReviewBudgetRuntime {
    repeat_threshold: usize,
    low_signal_threshold: usize,
    keep_repeat: usize,
    keep_low_signal: usize,
    min_signal_score: f64,
    profile: RepoReviewBudgetProfile,
}

impl RepoReviewBudgetRuntime {
    fn from_env() -> Self {
        let repeat_threshold = std::env::var(REPO_REVIEW_BUDGET_ENV_REPEAT_THRESHOLD)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(2)
            .clamp(1, 12);
        let low_signal_threshold = std::env::var(REPO_REVIEW_BUDGET_ENV_LOW_SIGNAL_THRESHOLD)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(2)
            .clamp(1, 12);
        let keep_repeat = std::env::var(REPO_REVIEW_BUDGET_ENV_KEEP_REPEAT)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(2)
            .clamp(1, 12);
        let keep_low_signal = std::env::var(REPO_REVIEW_BUDGET_ENV_KEEP_LOW_SIGNAL)
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 12);
        let min_signal_score = std::env::var(REPO_REVIEW_BUDGET_ENV_MIN_SIGNAL_SCORE)
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.22)
            .clamp(0.0, 1.0);
        let profile = std::env::var(REPO_REVIEW_BUDGET_ENV_PROFILE)
            .ok()
            .as_deref()
            .and_then(RepoReviewBudgetProfile::parse)
            .unwrap_or(RepoReviewBudgetProfile::Balanced);
        Self {
            repeat_threshold,
            low_signal_threshold,
            keep_repeat,
            keep_low_signal,
            min_signal_score,
            profile,
        }
    }
}

fn apply_repo_review_budget_profile(profile: RepoReviewBudgetProfile) {
    let (repeat_threshold, low_signal_threshold, keep_repeat, keep_low_signal, min_signal_score) =
        match profile {
            RepoReviewBudgetProfile::Balanced => (2usize, 2usize, 2usize, 1usize, 0.22f64),
            RepoReviewBudgetProfile::Aggressive => (1usize, 1usize, 1usize, 1usize, 0.35f64),
            RepoReviewBudgetProfile::Relaxed => (3usize, 3usize, 3usize, 2usize, 0.15f64),
            RepoReviewBudgetProfile::Off => (12usize, 12usize, 12usize, 12usize, 0.01f64),
        };
    crate::env_vars::set_var(
        REPO_REVIEW_BUDGET_ENV_REPEAT_THRESHOLD,
        repeat_threshold.to_string(),
    );
    crate::env_vars::set_var(
        REPO_REVIEW_BUDGET_ENV_LOW_SIGNAL_THRESHOLD,
        low_signal_threshold.to_string(),
    );
    crate::env_vars::set_var(REPO_REVIEW_BUDGET_ENV_KEEP_REPEAT, keep_repeat.to_string());
    crate::env_vars::set_var(
        REPO_REVIEW_BUDGET_ENV_KEEP_LOW_SIGNAL,
        keep_low_signal.to_string(),
    );
    crate::env_vars::set_var(
        REPO_REVIEW_BUDGET_ENV_MIN_SIGNAL_SCORE,
        format!("{:.3}", min_signal_score),
    );
    crate::env_vars::set_var(REPO_REVIEW_BUDGET_ENV_PROFILE, profile.as_str());
}

fn latest_json_report(report_dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut reports: Vec<PathBuf> = std::fs::read_dir(report_dir)
        .ok()?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_string_lossy();
            if name.starts_with(prefix) && name.ends_with(".json") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    reports.sort();
    reports.into_iter().last()
}

fn summarize_gate_report(path: &Path, key: &str) -> Option<String> {
    let report = read_json_file(path)?;
    let ok = report
        .get("ok")
        .and_then(|v| v.as_bool())
        .map(|v| if v { "pass" } else { "fail" })
        .unwrap_or("unknown");
    let generated = report
        .get("generated_at")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    Some(format!(
        "{}={} @ {} ({})",
        key,
        ok,
        generated,
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string())
    ))
}

fn summarize_self_evolution_report(path: &Path, key: &str) -> Option<String> {
    let report = read_json_file(path)?;
    let ok = report
        .get("ok")
        .and_then(|v| v.as_bool())
        .map(|v| if v { "pass" } else { "fail" })
        .unwrap_or("unknown");
    let generated = report
        .get("generated_at")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let idx = report
        .get("summary")
        .and_then(|s| s.get("intelligence_index"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let recs = report
        .get("recommendations")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    Some(format!(
        "{}={} idx={:.2} recs={} @ {} ({})",
        key,
        ok,
        idx,
        recs,
        generated,
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string())
    ))
}

fn self_evolution_recommendations(path: &Path) -> Vec<String> {
    let report = match read_json_file(path) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let Some(items) = report.get("recommendations").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
            let sev = obj.get("severity").and_then(|v| v.as_str()).unwrap_or("PX");
            let title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let cmd = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
            Some(format!("[{sev}] {id}: {title}\n  cmd: {cmd}"))
        })
        .collect()
}

const AUTOPILOT_ALLOWED_ENV_KEYS: &[&str] = &[
    "HERMES_TOOL_POLICY_PRESET",
    "HERMES_TOOL_POLICY_MODE",
    "HERMES_MODEL_CATALOG_GUARD",
    "HERMES_MODEL_AUTO_REMEDIATE",
    "HERMES_REPLAY_ENABLED",
    "HERMES_PERF_AUTOPILOT_STATUS",
    "HERMES_PERF_AUTOPILOT_PROFILE",
    "HERMES_PERF_AUTOPILOT_MODE",
];

fn summarize_performance_autopilot_report(path: &Path, key: &str) -> Option<String> {
    let report = read_json_file(path)?;
    let ok = report
        .get("ok")
        .and_then(|v| v.as_bool())
        .map(|v| if v { "pass" } else { "fail" })
        .unwrap_or("unknown");
    let generated = report
        .get("generated_at")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let recommendations = report
        .get("recommendations")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    let severe = report
        .get("recommendations")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|item| {
                    item.get("severity")
                        .and_then(|v| v.as_str())
                        .is_some_and(|sev| {
                            sev.eq_ignore_ascii_case("P0") || sev.eq_ignore_ascii_case("P1")
                        })
                })
                .count()
        })
        .unwrap_or(0);
    let adaptive_idx = report
        .get("adaptive_index")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let profile = report
        .get("profile_recommendation")
        .and_then(|v| v.as_str())
        .unwrap_or("balanced");
    Some(format!(
        "{}={} idx={:.2} profile={} recs={} severe={} @ {} ({})",
        key,
        ok,
        adaptive_idx,
        profile,
        recommendations,
        severe,
        generated,
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string())
    ))
}

fn performance_autopilot_recommendations(path: &Path) -> Vec<String> {
    let report = match read_json_file(path) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let Some(items) = report.get("recommendations").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            let id = obj.get("id").and_then(|v| v.as_str()).unwrap_or("UNKNOWN");
            let sev = obj.get("severity").and_then(|v| v.as_str()).unwrap_or("PX");
            let title = obj.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let rec = obj
                .get("recommendation")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!("[{sev}] {id}: {title}\n  recommendation: {rec}"))
        })
        .collect()
}

fn parse_env_file_kv(path: &Path) -> Vec<(String, String)> {
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let (k, v) = trimmed.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn write_autopilot_runtime_event(
    report_dir: &Path,
    session_id: &str,
    mode: &str,
    profile: &str,
    applied: &[(String, String)],
) {
    let path = report_dir.join("performance-autopilot-runtime.jsonl");
    let created_at = format!("{:?}", SystemTime::now());
    let payload = serde_json::json!({
        "created_at": created_at,
        "session_id": session_id,
        "mode": mode,
        "profile": profile,
        "applied": applied,
    });
    if let Ok(line) = serde_json::to_string(&payload) {
        if let Ok(mut fh) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(&mut fh, "{line}");
        }
    }
}

fn dashboard_status_line_from_payload(payload: &serde_json::Value) -> String {
    let enabled = payload
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("n/a");
    format!(
        "dashboard: {} ({})",
        if enabled { "ON" } else { "OFF" },
        url
    )
}

async fn handle_dashboard_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    let mut params = serde_json::json!({
        "action": action
    });
    if let Some(host) = args.get(1) {
        params["host"] = serde_json::Value::String((*host).to_string());
    }
    if let Some(port) = args.get(2).and_then(|raw| raw.parse::<u16>().ok()) {
        params["port"] = serde_json::json!(port);
    }
    if args
        .iter()
        .any(|arg| arg.eq_ignore_ascii_case("--insecure"))
    {
        params["insecure"] = serde_json::json!(true);
    }

    let raw = app
        .tool_registry
        .dispatch_async("dashboard_control", params)
        .await;
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({"result": raw}));

    if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
        emit_command_output(app, format!("Dashboard command failed: {err}"));
        return Ok(CommandResult::Handled);
    }

    let rendered = match action.as_str() {
        "enable" | "on" => format!(
            "Dashboard enabled at {}\nConfig: {}",
            parsed
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown"),
            parsed
                .get("config_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ),
        "disable" | "off" => format!(
            "Dashboard disabled (URL: {})\nConfig: {}",
            parsed
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown"),
            parsed
                .get("config_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ),
        "url" => format!(
            "{}\n{}",
            parsed
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown"),
            parsed
                .get("config_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ),
        _ => format!(
            "{}\nConfig: {}",
            dashboard_status_line_from_payload(&parsed),
            parsed
                .get("config_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        ),
    };
    emit_command_output(app, rendered);
    Ok(CommandResult::Handled)
}

async fn run_ops_shell_command(command: &str) -> Result<String, AgentError> {
    let output = tokio::process::Command::new("bash")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AgentError::Io(format!("ops shell command failed: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let mut msg = String::new();
    if !stdout.is_empty() {
        msg.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !msg.is_empty() {
            msg.push_str("\n\n");
        }
        msg.push_str("stderr:\n");
        msg.push_str(&stderr);
    }
    if msg.is_empty() {
        msg = format!("(exit: {})", output.status);
    } else if !output.status.success() {
        msg = format!("(exit: {})\n{}", output.status, msg);
    }
    Ok(msg)
}

async fn run_current_hermes_cli_command(args: &[&str]) -> Result<String, AgentError> {
    let exe = std::env::current_exe()
        .map_err(|e| AgentError::Io(format!("resolve current executable: {e}")))?;
    let output = tokio::process::Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AgentError::Io(format!("run current hermes command failed: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let mut msg = String::new();
    if !stdout.is_empty() {
        msg.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !msg.is_empty() {
            msg.push_str("\n\n");
        }
        msg.push_str("stderr:\n");
        msg.push_str(&stderr);
    }
    if msg.is_empty() {
        msg = format!("(exit: {})", output.status);
    } else if !output.status.success() {
        msg = format!("(exit: {})\n{}", output.status, msg);
    }
    Ok(msg)
}

fn handle_simulate_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() || args[0].eq_ignore_ascii_case("status") {
        let counters = app.tool_registry.policy_counters();
        emit_command_output(
            app,
            format!(
                "Tool-policy simulation\n\
                 usage: /simulate <tool_name> [json-params]\n\
                 examples:\n  /simulate terminal {{\"cmd\":\"ls\"}}\n  /simulate skill_manage {{\"action\":\"view\",\"skill\":\"contextlattice-agent-contract\"}}\n\
                 counters: allow={} deny={} audit_only={} simulate={} would_block={}",
                counters.allow,
                counters.deny,
                counters.audit_only,
                counters.simulate,
                counters.would_block
            ),
        );
        return Ok(CommandResult::Handled);
    }

    let tool_name = args[0].trim();
    if tool_name.is_empty() {
        emit_command_output(app, "Usage: /simulate <tool_name> [json-params]");
        return Ok(CommandResult::Handled);
    }
    let params = if args.len() > 1 {
        let raw = args[1..].join(" ");
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(v) if v.is_object() => v,
            Ok(_) => {
                emit_command_output(app, "simulate params must be a JSON object.");
                return Ok(CommandResult::Handled);
            }
            Err(err) => {
                emit_command_output(
                    app,
                    format!("simulate params parse error: {}\nraw={}", err, raw),
                );
                return Ok(CommandResult::Handled);
            }
        }
    } else {
        serde_json::json!({})
    };

    let decision = app
        .tool_registry
        .evaluate_policy_preview(tool_name, &params);
    let payload = serde_json::json!({
        "tool": tool_name,
        "params": params,
        "decision": {
            "allow": decision.allow,
            "mode": decision.mode.as_str(),
            "audited_only": decision.audited_only,
            "simulated": decision.simulated,
            "would_block": decision.would_block,
            "code": decision.code,
            "reason": decision.reason,
        }
    });
    emit_command_output(
        app,
        serde_json::to_string_pretty(&payload)
            .map_err(|e| AgentError::Config(format!("serialize simulate result: {e}")))?,
    );
    Ok(CommandResult::Handled)
}

fn route_learning_state_path() -> PathBuf {
    hermes_config::hermes_home().join("route-learning.json")
}

fn route_health_state_path() -> PathBuf {
    hermes_config::hermes_home().join("route-health.json")
}

fn route_autotune_state_path() -> PathBuf {
    hermes_config::hermes_home().join("route-autotune.json")
}

fn route_autotune_env_path() -> PathBuf {
    hermes_config::hermes_home().join("route-autotune.env")
}

fn summarize_route_health_state(path: &Path) -> String {
    let Some(report) = read_json_file(path) else {
        return "route_health=unknown".to_string();
    };
    let overall = report
        .get("overall")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let score = report
        .get("summary")
        .and_then(|v| v.get("health_score"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let generated = report
        .get("generated_at")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    format!(
        "route_health={} score={:.2} @ {}",
        overall, score, generated
    )
}

fn summarize_route_health_details(path: &Path) -> Option<String> {
    let report = read_json_file(path)?;
    let entries = report.get("entries")?.as_array()?;
    if entries.is_empty() {
        return Some("route_health_trace=no_entries".to_string());
    }
    let mut ranked = entries
        .iter()
        .filter_map(|entry| {
            let key = entry.get("key").and_then(|v| v.as_str())?;
            let tier = entry
                .get("tier")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let health = entry
                .get("health_score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let reasons = entry
                .get("reasons")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some((key.to_string(), tier.to_string(), health, reasons))
        })
        .collect::<Vec<_>>();
    if ranked.is_empty() {
        return Some("route_health_trace=no_parseable_entries".to_string());
    }
    ranked.sort_by(|a, b| a.2.total_cmp(&b.2));
    let hottest = ranked
        .iter()
        .take(3)
        .map(|(key, tier, health, reasons)| {
            let reason_text = if reasons.is_empty() {
                "no_reasons".to_string()
            } else {
                reasons.join("|")
            };
            format!("{key} tier={tier} score={health:.2} reasons={reason_text}")
        })
        .collect::<Vec<_>>()
        .join(" ; ");
    Some(format!("route_health_trace={}", hottest))
}

fn handle_ops_budget_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() || args[0].eq_ignore_ascii_case("status") {
        let budget = RepoReviewBudgetRuntime::from_env();
        emit_command_output(
            app,
            format!(
                "repo_review_budget profile={}\n\
                 repeat_threshold={} low_signal_threshold={} keep_repeat={} keep_low_signal={} min_signal_score={:.2}",
                budget.profile.as_str(),
                budget.repeat_threshold,
                budget.low_signal_threshold,
                budget.keep_repeat,
                budget.keep_low_signal,
                budget.min_signal_score
            ),
        );
        return Ok(CommandResult::Handled);
    }
    match args[0].to_ascii_lowercase().as_str() {
        "list" => emit_command_output(
            app,
            "Repo-review budget profiles:\n- balanced: default trim cadence\n- aggressive: trim repetitive discovery quickly\n- relaxed: allow broader exploration before trimming\n- off: effectively disable trimming",
        ),
        "clear" => {
            for key in [
                REPO_REVIEW_BUDGET_ENV_REPEAT_THRESHOLD,
                REPO_REVIEW_BUDGET_ENV_LOW_SIGNAL_THRESHOLD,
                REPO_REVIEW_BUDGET_ENV_KEEP_REPEAT,
                REPO_REVIEW_BUDGET_ENV_KEEP_LOW_SIGNAL,
                REPO_REVIEW_BUDGET_ENV_MIN_SIGNAL_SCORE,
                REPO_REVIEW_BUDGET_ENV_PROFILE,
            ] {
                crate::env_vars::remove_var(key);
            }
            emit_command_output(app, "Cleared repo-review budget runtime overrides.");
        }
        profile_raw => {
            let Some(profile) = RepoReviewBudgetProfile::parse(profile_raw) else {
                emit_command_output(
                    app,
                    "Usage: /ops budget [status|list|balanced|aggressive|relaxed|off|clear]",
                );
                return Ok(CommandResult::Handled);
            };
            apply_repo_review_budget_profile(profile);
            let budget = RepoReviewBudgetRuntime::from_env();
            emit_command_output(
                app,
                format!(
                    "repo_review_budget set to '{}' (repeat={} low_signal={} keep_repeat={} keep_low_signal={} min_signal={:.2})",
                    profile.as_str(),
                    budget.repeat_threshold,
                    budget.low_signal_threshold,
                    budget.keep_repeat,
                    budget.keep_low_signal,
                    budget.min_signal_score
                ),
            );
        }
    }
    Ok(CommandResult::Handled)
}

fn handle_ops_tool_profile_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let mode = std::env::var("HERMES_REPO_REVIEW_TOOL_PROFILE_MODE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "off".to_string());
    if args.is_empty()
        || args
            .first()
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "status" | "show"))
    {
        emit_command_output(
            app,
            format!(
                "repo_review_tool_profile mode={}\nUse `/ops tool-profile [off|balanced|focus]`.\nEscape hatch: include `allow all tools` or `disable narrowing` in your request.",
                mode
            ),
        );
        return Ok(CommandResult::Handled);
    }
    if args[0].eq_ignore_ascii_case("list") {
        emit_command_output(
            app,
            "Repo-review tool profile modes:\n- off: disable narrowing (open tool lane)\n- balanced: filter messaging/non-repo noise only\n- focus: balanced + stricter repo-first filtering",
        );
        return Ok(CommandResult::Handled);
    }
    if args[0].eq_ignore_ascii_case("clear") {
        crate::env_vars::remove_var("HERMES_REPO_REVIEW_TOOL_PROFILE_MODE");
        emit_command_output(
            app,
            "Cleared repo-review tool profile override (default=balanced).",
        );
        return Ok(CommandResult::Handled);
    }
    let next = args[0].to_ascii_lowercase();
    if !matches!(next.as_str(), "off" | "balanced" | "focus") {
        emit_command_output(
            app,
            "Usage: /ops tool-profile [status|list|off|balanced|focus|clear]",
        );
        return Ok(CommandResult::Handled);
    }
    crate::env_vars::set_var("HERMES_REPO_REVIEW_TOOL_PROFILE_MODE", next.as_str());
    emit_command_output(
        app,
        format!("repo_review_tool_profile mode set to `{}`", next),
    );
    Ok(CommandResult::Handled)
}

async fn handle_ops_eval_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    let Some(repo_root) = discover_repo_root_for_about() else {
        emit_command_output(
            app,
            "Eval controls are unavailable outside source checkout.",
        );
        return Ok(CommandResult::Handled);
    };
    let report_dir = repo_root.join(".sync-reports");
    match sub.as_str() {
        "status" => {
            let latest = latest_json_report(&report_dir, "session-eval-harness-")
                .or_else(|| latest_json_report(&report_dir, "eval-trend-gate-"));
            if let Some(path) = latest {
                let summary = summarize_gate_report(&path, "eval")
                    .unwrap_or_else(|| format!("latest eval report: {}", path.display()));
                emit_command_output(
                    app,
                    format!(
                        "{summary}\nRun `/ops eval run` to generate a fresh session-backed report."
                    ),
                );
            } else {
                emit_command_output(
                    app,
                    "No eval reports found yet. Run `/ops eval run` to generate one.",
                );
            }
            Ok(CommandResult::Handled)
        }
        "run" => {
            let out = run_ops_shell_command(
                "python3 scripts/run-session-eval-harness.py --repo-root . --json",
            )
            .await?;
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        "latest" => {
            let Some(path) = latest_json_report(&report_dir, "session-eval-harness-")
                .or_else(|| latest_json_report(&report_dir, "eval-trend-gate-"))
            else {
                emit_command_output(app, "No eval reports found.");
                return Ok(CommandResult::Handled);
            };
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| AgentError::Io(format!("read {}: {}", path.display(), e)))?;
            emit_command_output(
                app,
                format!(
                    "Latest eval report: {}\n{}",
                    path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string()),
                    raw
                ),
            );
            Ok(CommandResult::Handled)
        }
        _ => {
            emit_command_output(app, "Usage: /ops eval [status|run|latest]");
            Ok(CommandResult::Handled)
        }
    }
}

async fn handle_qos_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match sub.as_str() {
        "status" | "show" => {
            let learning_path = route_learning_state_path();
            let health_path = route_health_state_path();
            let autotune_path = route_autotune_state_path();
            let autotune_env = route_autotune_env_path();
            let learning_entries = read_json_file(&learning_path)
                .and_then(|v| {
                    v.get("entries")
                        .and_then(|e| e.as_array())
                        .map(|arr| arr.len())
                })
                .unwrap_or(0usize);
            let health_summary = summarize_route_health_state(&health_path);
            let mut out = String::new();
            let _ = writeln!(out, "Provider QoS router");
            let _ = writeln!(
                out,
                "  route_learning_entries={} ({})",
                learning_entries,
                learning_path.display()
            );
            let _ = writeln!(out, "  {} ({})", health_summary, health_path.display());
            if let Some(trace) = summarize_route_health_details(&health_path) {
                let _ = writeln!(out, "  {}", trace);
            }
            let _ = writeln!(
                out,
                "  route_autotune_state={} ({})",
                if autotune_path.exists() {
                    "present"
                } else {
                    "missing"
                },
                autotune_path.display()
            );
            let _ = writeln!(
                out,
                "  route_autotune_env={} ({})",
                if autotune_env.exists() {
                    "present"
                } else {
                    "missing"
                },
                autotune_env.display()
            );
            let _ = writeln!(
                out,
                "  actions: /qos health | /qos autotune plan | /qos autotune apply"
            );
            emit_command_output(app, out.trim_end());
            Ok(CommandResult::Handled)
        }
        "health" => {
            let out = run_current_hermes_cli_command(&["route-health", "--json"]).await?;
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        "autotune" => {
            let action = args.get(1).copied().unwrap_or("plan").to_ascii_lowercase();
            let out = match action.as_str() {
                "plan" => {
                    run_current_hermes_cli_command(&["route-autotune", "plan", "--json"]).await?
                }
                "apply" => {
                    run_current_hermes_cli_command(&[
                        "route-autotune",
                        "apply",
                        "--apply",
                        "--json",
                    ])
                    .await?
                }
                _ => {
                    emit_command_output(app, "Usage: /qos autotune [plan|apply]");
                    return Ok(CommandResult::Handled);
                }
            };
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        "help" => {
            emit_command_output(app, "Usage: /qos [status|health|autotune [plan|apply]]");
            Ok(CommandResult::Handled)
        }
        _ => {
            emit_command_output(app, "Usage: /qos [status|health|autotune [plan|apply]]");
            Ok(CommandResult::Handled)
        }
    }
}

fn handle_ops_skills_tier_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() || args[0].eq_ignore_ascii_case("status") {
        emit_command_output(
            app,
            format!(
                "skills_tier={} (bypass={})",
                skills::skills_execution_tier().as_str(),
                if skills::skills_tier_bypass_enabled() {
                    "ON"
                } else {
                    "OFF"
                }
            ),
        );
        return Ok(CommandResult::Handled);
    }

    let Some(next) = skills::SkillsExecutionTier::parse(args[0]) else {
        emit_command_output(
            app,
            "Usage: /ops skills-tier [status|trusted|balanced|open]",
        );
        return Ok(CommandResult::Handled);
    };
    crate::env_vars::set_var("HERMES_SKILLS_EXECUTION_TIER", next.as_str());
    emit_command_output(
        app,
        format!(
            "skills_tier set to '{}' for this runtime process.",
            next.as_str()
        ),
    );
    Ok(CommandResult::Handled)
}

async fn handle_ops_gate_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match sub.as_str() {
        "status" => {
            if let Some(repo_root) = discover_repo_root_for_about() {
                let report_dir = repo_root.join(".sync-reports");
                let eval = latest_json_report(&report_dir, "eval-trend-gate-")
                    .and_then(|p| summarize_gate_report(&p, "eval_trend"))
                    .unwrap_or_else(|| "eval_trend=unknown".to_string());
                let slo = latest_json_report(&report_dir, "slo-auto-rollback-")
                    .and_then(|p| summarize_gate_report(&p, "slo_rollback"))
                    .unwrap_or_else(|| "slo_rollback=unknown".to_string());
                let elite = latest_json_report(&report_dir, "elite-sync-gate-")
                    .and_then(|p| summarize_gate_report(&p, "elite_sync_gate"))
                    .unwrap_or_else(|| "elite_sync_gate=unknown".to_string());
                emit_command_output(app, format!("{}\n{}\n{}", eval, slo, elite));
            } else {
                emit_command_output(app, "Gate status unavailable outside source checkout.");
            }
            Ok(CommandResult::Handled)
        }
        "eval" => {
            let out = run_ops_shell_command(
                "python3 scripts/run-eval-trend-gate.py --allow-missing-baseline --json",
            )
            .await?;
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        "elite" => {
            let out =
                run_ops_shell_command("python3 scripts/run-elite-sync-gate.py --json").await?;
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        "slo" => {
            let check_cmd = std::env::var("HERMES_SLO_CHECK_CMD").ok();
            let rollback_cmd = std::env::var("HERMES_SLO_ROLLBACK_CMD").ok();
            let (Some(check), Some(rollback)) = (check_cmd, rollback_cmd) else {
                emit_command_output(
                    app,
                    "Set HERMES_SLO_CHECK_CMD and HERMES_SLO_ROLLBACK_CMD, then run `/ops gate slo`.",
                );
                return Ok(CommandResult::Handled);
            };
            let cmd = format!(
                "python3 scripts/run-slo-auto-rollback.py --check-cmd {} --rollback-cmd {} --json",
                shell_escape(&check),
                shell_escape(&rollback)
            );
            let out = run_ops_shell_command(&cmd).await?;
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        _ => {
            emit_command_output(app, "Usage: /ops gate [status|eval|elite|slo]");
            Ok(CommandResult::Handled)
        }
    }
}

async fn handle_ops_evolve_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    let Some(repo_root) = discover_repo_root_for_about() else {
        emit_command_output(
            app,
            "Self-evolution controls are unavailable outside source checkout.",
        );
        return Ok(CommandResult::Handled);
    };
    let report_dir = repo_root.join(".sync-reports");
    match sub.as_str() {
        "status" => {
            let summary = latest_json_report(&report_dir, "self-evolution-loop-")
                .and_then(|p| summarize_self_evolution_report(&p, "self_evolution"))
                .unwrap_or_else(|| "self_evolution=unknown (no reports yet)".to_string());
            emit_command_output(
                app,
                format!(
                    "{}\nRun `/ops evolve run` to execute the loop now.",
                    summary
                ),
            );
            Ok(CommandResult::Handled)
        }
        "run" => {
            let cmd = if let Some(obj) = app.session_objective.as_deref() {
                format!(
                    "python3 scripts/run-self-evolution-loop.py --json --objective {}",
                    shell_escape(obj)
                )
            } else {
                "python3 scripts/run-self-evolution-loop.py --json".to_string()
            };
            let out = run_ops_shell_command(&cmd).await?;
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        "recommend" | "recs" => {
            let Some(path) = latest_json_report(&report_dir, "self-evolution-loop-") else {
                emit_command_output(
                    app,
                    "No self-evolution reports found. Run `/ops evolve run` first.",
                );
                return Ok(CommandResult::Handled);
            };
            let recs = self_evolution_recommendations(&path);
            if recs.is_empty() {
                emit_command_output(
                    app,
                    format!(
                        "No recommendations found in {}.",
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string())
                    ),
                );
            } else {
                let file_label = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                emit_command_output(
                    app,
                    format!(
                        "Self-evolution recommendations ({file_label}):\n{}",
                        recs.join("\n")
                    ),
                );
            }
            Ok(CommandResult::Handled)
        }
        _ => {
            emit_command_output(app, "Usage: /ops evolve [status|run|recommend]");
            Ok(CommandResult::Handled)
        }
    }
}

async fn handle_ops_autopilot_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    let mode = std::env::var("HERMES_PERF_AUTOPILOT_MODE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "advisory".to_string());
    let profile = std::env::var("HERMES_PERF_AUTOPILOT_PROFILE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "off".to_string());

    let Some(repo_root) = discover_repo_root_for_about() else {
        emit_command_output(
            app,
            "Autopilot controls are unavailable outside source checkout.",
        );
        return Ok(CommandResult::Handled);
    };
    let report_dir = repo_root.join(".sync-reports");
    let latest = latest_json_report(&report_dir, "performance-autopilot-");

    match sub.as_str() {
        "status" => {
            let summary = latest
                .as_ref()
                .and_then(|p| summarize_performance_autopilot_report(p, "autopilot"))
                .unwrap_or_else(|| "autopilot=unknown (no reports yet)".to_string());
            emit_command_output(
                app,
                format!(
                    "{}\nmode={} profile={}\nUse `/ops autopilot run` then `/ops autopilot recommend`.",
                    summary, mode, profile
                ),
            );
            Ok(CommandResult::Handled)
        }
        "run" => {
            let out = run_ops_shell_command(
                "python3 scripts/run-performance-autopilot.py --repo-root . --json",
            )
            .await?;
            emit_command_output(app, out);
            Ok(CommandResult::Handled)
        }
        "recommend" | "recs" => {
            let Some(path) = latest else {
                emit_command_output(
                    app,
                    "No performance autopilot reports found. Run `/ops autopilot run` first.",
                );
                return Ok(CommandResult::Handled);
            };
            let recs = performance_autopilot_recommendations(&path);
            if recs.is_empty() {
                emit_command_output(
                    app,
                    format!(
                        "No recommendations found in {}.",
                        path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string())
                    ),
                );
            } else {
                let file_label = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                emit_command_output(
                    app,
                    format!(
                        "Autopilot recommendations ({file_label}):\n{}",
                        recs.join("\n")
                    ),
                );
            }
            Ok(CommandResult::Handled)
        }
        "apply" => {
            let env_path =
                report_dir.join(format!("performance-autopilot-env-{}.env", app.session_id));
            let cmd = format!(
                "python3 scripts/run-performance-autopilot.py --repo-root . --apply-env {} --json",
                shell_escape(&env_path.display().to_string())
            );
            let out = run_ops_shell_command(&cmd).await?;
            let kvs = parse_env_file_kv(&env_path);
            let mut applied = Vec::new();
            for (k, v) in kvs {
                if AUTOPILOT_ALLOWED_ENV_KEYS
                    .iter()
                    .any(|allowed| *allowed == k)
                {
                    crate::env_vars::set_var(&k, &v);
                    applied.push((k, v));
                }
            }
            write_autopilot_runtime_event(&report_dir, &app.session_id, &mode, &profile, &applied);
            let applied_keys = if applied.is_empty() {
                "(none)".to_string()
            } else {
                applied
                    .iter()
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            emit_command_output(
                app,
                format!(
                    "{out}\n\nApplied safe runtime knobs: {applied_keys}\nmode={mode} profile={profile}\nlog: {}",
                    report_dir
                        .join("performance-autopilot-runtime.jsonl")
                        .display()
                ),
            );
            Ok(CommandResult::Handled)
        }
        "profile" => {
            let next = args.get(1).map(|v| v.to_ascii_lowercase());
            match next.as_deref() {
                None | Some("status") => {
                    emit_command_output(app, format!("autopilot profile={profile} (mode={mode})"))
                }
                Some("list") => emit_command_output(
                    app,
                    "Autopilot profiles:\n- balanced: default stability/perf mix\n- throughput: lower latency and tighter loop cadence\n- quality: stronger verification and replay focus\n- reliability: prioritize retries/recovery and degraded-source tolerance\n- safety: strictest gate posture with conservative policy knobs",
                ),
                Some("balanced" | "throughput" | "quality" | "reliability" | "safety") => {
                    let value = next.unwrap_or_else(|| "off".to_string());
                    crate::env_vars::set_var("HERMES_PERF_AUTOPILOT_PROFILE", &value);
                    emit_command_output(app, format!("autopilot profile set to '{}'", value));
                }
                Some(other) => {
                    emit_command_output(
                        app,
                        format!(
                            "Unknown profile '{}'. Use `/ops autopilot profile list`.",
                            other
                        ),
                    );
                }
            }
            Ok(CommandResult::Handled)
        }
        "mode" => {
            let next = args.get(1).map(|v| v.to_ascii_lowercase());
            match next.as_deref() {
                None | Some("status") => emit_command_output(app, format!("autopilot mode={mode}")),
                Some("list") => emit_command_output(
                    app,
                    "Autopilot modes:\n- off: disabled\n- advisory: report + recommendations only\n- enforce: intended to pair with `/ops autopilot apply` during incidents",
                ),
                Some("off" | "advisory" | "enforce") => {
                    let value = next.unwrap_or_else(|| "advisory".to_string());
                    crate::env_vars::set_var("HERMES_PERF_AUTOPILOT_MODE", &value);
                    emit_command_output(app, format!("autopilot mode set to '{}'", value));
                }
                Some(other) => {
                    emit_command_output(
                        app,
                        format!("Unknown mode '{}'. Use `/ops autopilot mode list`.", other),
                    );
                }
            }
            Ok(CommandResult::Handled)
        }
        "clear" => {
            crate::env_vars::remove_var("HERMES_PERF_AUTOPILOT_MODE");
            crate::env_vars::remove_var("HERMES_PERF_AUTOPILOT_PROFILE");
            crate::env_vars::remove_var("HERMES_PERF_AUTOPILOT_STATUS");
            emit_command_output(
                app,
                "Cleared autopilot runtime overrides (mode/profile/status).",
            );
            Ok(CommandResult::Handled)
        }
        _ => {
            emit_command_output(
                app,
                "Usage: /ops autopilot [status|run|recommend|apply|profile [status|list|balanced|throughput|quality|reliability|safety]|mode [status|list|off|advisory|enforce]|clear]",
            );
            Ok(CommandResult::Handled)
        }
    }
}

async fn handle_ops_cockpit_command(
    app: &mut App,
    _args: &[&str],
) -> Result<CommandResult, AgentError> {
    let counters = app.tool_registry.policy_counters();
    let budget = RepoReviewBudgetRuntime::from_env();
    let board = render_mission_board(
        &app.current_model,
        app.session_objective.as_deref(),
        background::background_job_counts(),
    )
    .await?;
    let route_health = summarize_route_health_state(&route_health_state_path());
    let eval_summary = if let Some(repo_root) = discover_repo_root_for_about() {
        let report_dir = repo_root.join(".sync-reports");
        latest_json_report(&report_dir, "session-eval-harness-")
            .or_else(|| latest_json_report(&report_dir, "eval-trend-gate-"))
            .and_then(|p| summarize_gate_report(&p, "eval"))
            .unwrap_or_else(|| "eval=unknown".to_string())
    } else {
        "eval=unavailable".to_string()
    };
    let snapshot_count =
        session::enumerate_saved_sessions(&hermes_config::hermes_home().join("sessions")).len();
    let mut out = String::new();
    out.push_str("Ops Cockpit\n");
    out.push_str("===========\n");
    let _ = writeln!(out, "session: {}", app.session_id);
    let _ = writeln!(out, "model: {}", app.current_model);
    let _ = writeln!(
        out,
        "policy: profile={} mode={} preset={} sandbox={} skills_tier={}",
        policy::current_policy_profile_name(),
        std::env::var("HERMES_TOOL_POLICY_MODE").unwrap_or_else(|_| "enforce".into()),
        std::env::var("HERMES_TOOL_POLICY_PRESET").unwrap_or_else(|_| "relaxed".into()),
        std::env::var("HERMES_EXECUTION_SANDBOX_PROFILE").unwrap_or_else(|_| "balanced".into()),
        std::env::var("HERMES_SKILLS_EXECUTION_TIER").unwrap_or_else(|_| "balanced".into())
    );
    let _ = writeln!(
        out,
        "planner_capability_router={} compaction_governance={} replay_trace={}",
        plan_capability_mode().as_str(),
        compress::compaction_governance_mode().as_str(),
        if replay_enabled_runtime() {
            "on"
        } else {
            "off"
        }
    );
    let _ = writeln!(
        out,
        "repo_review_budget: profile={} repeat={} low_signal={} keep_repeat={} keep_low_signal={} min_signal={:.2}",
        budget.profile.as_str(),
        budget.repeat_threshold,
        budget.low_signal_threshold,
        budget.keep_repeat,
        budget.keep_low_signal,
        budget.min_signal_score
    );
    let _ = writeln!(
        out,
        "policy_counters: allow={} deny={} audit_only={} simulate={} would_block={}",
        counters.allow, counters.deny, counters.audit_only, counters.simulate, counters.would_block
    );
    let _ = writeln!(
        out,
        "qos: {} | learning_entries={} | snapshots={}",
        route_health,
        read_json_file(&route_learning_state_path())
            .and_then(|v| v
                .get("entries")
                .and_then(|e| e.as_array())
                .map(|arr| arr.len()))
            .unwrap_or(0usize),
        snapshot_count
    );
    let _ = writeln!(out, "eval: {}", eval_summary);
    out.push('\n');
    out.push_str(&board);
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn shell_escape(input: &str) -> String {
    let escaped = input.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

async fn handle_ops_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() || args[0].eq_ignore_ascii_case("status") {
        let yolo = !app.config.approval.require_approval;
        let policy_mode = std::env::var("HERMES_TOOL_POLICY_MODE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "enforce".to_string());
        let policy_preset = std::env::var("HERMES_TOOL_POLICY_PRESET")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "off".to_string());
        let counters = app.tool_registry.policy_counters();
        let dashboard_status = {
            let raw = app
                .tool_registry
                .dispatch_async("dashboard_control", serde_json::json!({"action":"status"}))
                .await;
            let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(
                |_| serde_json::json!({"enabled":false,"url":"unknown","error":"unparseable"}),
            );
            dashboard_status_line_from_payload(&parsed)
        };
        let gate_status = if let Some(repo_root) = discover_repo_root_for_about() {
            let report_dir = repo_root.join(".sync-reports");
            let eval = latest_json_report(&report_dir, "eval-trend-gate-")
                .and_then(|p| summarize_gate_report(&p, "eval"))
                .unwrap_or_else(|| "eval=unknown".to_string());
            let slo = latest_json_report(&report_dir, "slo-auto-rollback-")
                .and_then(|p| summarize_gate_report(&p, "slo"))
                .unwrap_or_else(|| "slo=unknown".to_string());
            let evolve = latest_json_report(&report_dir, "self-evolution-loop-")
                .and_then(|p| summarize_self_evolution_report(&p, "evolve"))
                .unwrap_or_else(|| "evolve=unknown".to_string());
            let autopilot = latest_json_report(&report_dir, "performance-autopilot-")
                .and_then(|p| summarize_performance_autopilot_report(&p, "autopilot"))
                .unwrap_or_else(|| "autopilot=unknown".to_string());
            format!("{eval}; {slo}; {evolve}; {autopilot}")
        } else {
            "unavailable (non-source checkout)".to_string()
        };
        let autopilot_mode = std::env::var("HERMES_PERF_AUTOPILOT_MODE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "advisory".to_string());
        let autopilot_profile = std::env::var("HERMES_PERF_AUTOPILOT_PROFILE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "off".to_string());
        let repo_review_budget = RepoReviewBudgetRuntime::from_env();
        let tool_profile_mode = std::env::var("HERMES_REPO_REVIEW_TOOL_PROFILE_MODE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "off".to_string());

        let out = format!(
            "Operator Control Plane\n\
             \n\
             Runtime:\n\
               session:      {}\n\
               model:        {}\n\
               personality:  {}\n\
             \n\
             Controls:\n\
               yolo:         {}\n\
               mouse:        {}\n\
               statusbar:    ON\n\
               reasoning:    `/ops reasoning status` + `/ops reasoning set ...`\n\
               raw:          toggle via `/ops raw`\n\
               verbose:      toggle via `/ops verbose`\n\
             \n\
             Policy/Gates:\n\
               tool_policy:  mode={} preset={}\n\
               autopilot:    mode={} profile={}\n\
               tool_profile: {}\n\
               repo_budget:  profile={} repeat={} low_signal={} keep_repeat={} keep_low_signal={} min_signal={:.2}\n\
               task_depth:   {}\n\
               policy_counts allow={} deny={} audit_only={} simulate={} would_block={}\n\
               skills_tier:  {} (bypass={})\n\
               {}\n\
               gate_status:  {}\n\
             \n\
             Quick actions:\n\
               /ops model [provider|provider:model]\n\
               /ops mode [status|list|strict|standard|dev]\n\
               /ops personality [list|name]\n\
               /ops mouse [on|off|toggle]\n\
               /ops yolo\n\
               /ops reasoning [status|on|off|toggle|set <level>]\n\
               /ops raw [on|off|toggle|once|trace ...]\n\
               /ops verbose\n\
               /ops dashboard [status|on|off|url] [host] [port]\n\
               /ops skills-tier [status|trusted|balanced|open]\n\
               /ops tool-profile [status|list|off|balanced|focus]\n\
               /ops budget [status|list|balanced|aggressive|relaxed|off|clear]\n\
               /ops evolve [status|run|recommend]\n\
               /ops eval [status|run|latest]\n\
               /ops autopilot [status|run|recommend|apply|profile|mode|clear]\n\
               /ops gate [status|eval|elite|slo]\n\
               /ops cockpit\n\
               /mission [status|init]\n\
               /ops help",
            app.session_id,
            app.current_model,
            app.current_personality.as_deref().unwrap_or("(none)"),
            if yolo { "ON" } else { "OFF" },
            if app.mouse_enabled() { "ON" } else { "OFF" },
            policy_mode,
            policy_preset,
            autopilot_mode,
            autopilot_profile,
            tool_profile_mode,
            repo_review_budget.profile.as_str(),
            repo_review_budget.repeat_threshold,
            repo_review_budget.low_signal_threshold,
            repo_review_budget.keep_repeat,
            repo_review_budget.keep_low_signal,
            repo_review_budget.min_signal_score,
            task_depth_runtime_summary(),
            counters.allow,
            counters.deny,
            counters.audit_only,
            counters.simulate,
            counters.would_block,
            skills::skills_execution_tier().as_str(),
            if skills::skills_tier_bypass_enabled() {
                "ON"
            } else {
                "OFF"
            },
            dashboard_status,
            gate_status,
        );
        emit_command_output(app, out);
        return Ok(CommandResult::Handled);
    }

    match args[0].to_ascii_lowercase().as_str() {
        "help" => {
            emit_command_output(
                app,
                "Operator control plane commands:\n\
                 - /ops status\n\
                 - /ops model [provider|provider:model]\n\
                 - /ops mode [status|list|strict|standard|dev]\n\
                 - /ops personality [list|name]\n\
                 - /ops mouse [on|off|toggle]\n\
                 - /ops yolo\n\
                 - /ops reasoning [status|on|off|toggle|set <level>]\n\
                 - /ops raw [on|off|toggle|once|trace ...]\n\
                 - /ops verbose\n\
                 - /ops statusbar\n\
                 - /ops dashboard [status|on|off|url] [host] [port]\n\
                 - /ops skills-tier [status|trusted|balanced|open]\n\
                 - /ops tool-profile [status|list|off|balanced|focus]\n\
                 - /ops budget [status|list|balanced|aggressive|relaxed|off|clear]\n\
                 - /ops evolve [status|run|recommend]\n\
                 - /ops eval [status|run|latest]\n\
                 - /ops autopilot [status|run|recommend|apply|profile|mode|clear]\n\
                 - /ops gate [status|eval|elite|slo]\n\
                 - /ops cockpit\n\
                 - /mission [status|init]",
            );
            Ok(CommandResult::Handled)
        }
        "model" => model::handle_model_command(app, &args[1..]).await,
        "mode" => policy::handle_policy_command(app, &args[1..]),
        "personality" => handle_personality_command(app, &args[1..]),
        "mouse" => handle_mouse_command(app, &args[1..]),
        "yolo" => handle_yolo_command(app),
        "reasoning" => handle_reasoning_command(app, &args[1..]),
        "raw" => handle_raw_command(app, &args[1..]),
        "verbose" => handle_verbose_command(app),
        "statusbar" => handle_statusbar_command(app),
        "dashboard" => handle_dashboard_command(app, &args[1..]).await,
        "skills-tier" => handle_ops_skills_tier_command(app, &args[1..]),
        "tool-profile" | "toolprofile" | "tool_profile" => {
            handle_ops_tool_profile_command(app, &args[1..])
        }
        "budget" => handle_ops_budget_command(app, &args[1..]),
        "evolve" => handle_ops_evolve_command(app, &args[1..]).await,
        "eval" => handle_ops_eval_command(app, &args[1..]).await,
        "autopilot" => handle_ops_autopilot_command(app, &args[1..]).await,
        "gate" => handle_ops_gate_command(app, &args[1..]).await,
        "cockpit" => handle_ops_cockpit_command(app, &args[1..]).await,
        other => {
            emit_command_output(
                app,
                format!(
                    "Unknown /ops target '{}'. Try `/ops help` for available controls.",
                    other
                ),
            );
            Ok(CommandResult::Handled)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TriggerTriageDecision {
    Drop,
    Notify,
    Escalate,
    AgentRun,
}

impl TriggerTriageDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Drop => "drop",
            Self::Notify => "notify",
            Self::Escalate => "escalate",
            Self::AgentRun => "agent-run",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TriggerTriageAssessment {
    source: String,
    payload: String,
    severity: i32,
    decision: TriggerTriageDecision,
    requires_approval: bool,
    reasons: Vec<String>,
}

fn trigger_triage_mode() -> String {
    std::env::var("HERMES_TRIGGER_TRIAGE_MODE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "off".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TriggerTriageLearningEntry {
    at: String,
    source: String,
    outcome: String,
    decision: String,
    severity: i32,
    bias_delta: i32,
    note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TriggerTriageLearningState {
    #[serde(default)]
    entries: Vec<TriggerTriageLearningEntry>,
}

fn trigger_triage_learning_state_path() -> PathBuf {
    hermes_config::hermes_home()
        .join("triage")
        .join("learning.json")
}

fn load_trigger_triage_learning_state() -> TriggerTriageLearningState {
    let path = trigger_triage_learning_state_path();
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<TriggerTriageLearningState>(&raw).unwrap_or_default()
}

fn save_trigger_triage_learning_state(
    state: &TriggerTriageLearningState,
) -> Result<(), AgentError> {
    let path = trigger_triage_learning_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", parent.display(), e)))?;
    }
    let payload = serde_json::to_string_pretty(state)
        .map_err(|e| AgentError::Io(format!("Failed to encode triage learning state: {}", e)))?;
    std::fs::write(&path, payload)
        .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", path.display(), e)))?;
    Ok(())
}

fn triage_feedback_delta(outcome: &str) -> Option<i32> {
    match outcome.trim().to_ascii_lowercase().as_str() {
        "critical" | "escalate" | "confirmed" | "true-positive" | "tp" => Some(2),
        "useful" | "good" | "notify" | "watch" => Some(1),
        "neutral" | "mixed" => Some(0),
        "false-positive" | "fp" | "noise" | "noisy" => Some(-2),
        "drop" | "ignore" | "spam" => Some(-1),
        _ => None,
    }
}

fn triage_learning_bias(source: &str, payload: &str) -> (i32, Vec<String>) {
    let source_l = source.trim().to_ascii_lowercase();
    let payload_l = payload.trim().to_ascii_lowercase();
    let state = load_trigger_triage_learning_state();
    let mut total = 0i32;
    let mut reasons = Vec::new();
    for entry in state.entries.iter().rev().take(120) {
        if entry.source.eq_ignore_ascii_case(&source_l) {
            total += entry.bias_delta;
            if reasons.len() < 3 {
                reasons.push(format!(
                    "source feedback {} ({})",
                    entry.outcome, entry.bias_delta
                ));
            }
            continue;
        }
        if !entry.note.trim().is_empty()
            && payload_l.contains(entry.note.trim().to_ascii_lowercase().as_str())
        {
            total += entry.bias_delta.signum();
            if reasons.len() < 3 {
                reasons.push(format!("matched prior note '{}'", entry.note));
            }
        }
    }
    (total.clamp(-3, 3), reasons)
}

fn evaluate_trigger_triage(source: &str, payload: &str) -> TriggerTriageAssessment {
    let source_l = source.trim().to_ascii_lowercase();
    let payload_l = payload.trim().to_ascii_lowercase();
    let mode = trigger_triage_mode();
    let mut severity = 0i32;
    let mut reasons = Vec::new();

    for (needle, score, reason) in [
        ("panic", 4, "runtime panic or crash"),
        ("outage", 4, "service outage signal"),
        ("secret", 5, "secret exposure indicator"),
        ("key leak", 5, "key leak indicator"),
        ("drawdown", 4, "drawdown or loss event"),
        ("halt", 3, "trading halt or critical gate"),
        ("blocked", 2, "policy or sandbox block"),
        ("timeout", 1, "timeout/retry pressure"),
        ("latency", 1, "latency degradation"),
        ("error", 2, "error signal"),
    ] {
        if payload_l.contains(needle) || source_l.contains(needle) {
            severity += score;
            reasons.push(reason.to_string());
        }
    }

    if source_l.contains("webhook") {
        severity += 1;
        reasons.push("external webhook trigger".to_string());
    }
    if source_l.contains("cron") {
        severity += 1;
        reasons.push("scheduled trigger".to_string());
    }

    let (learning_bias, learning_reasons) = triage_learning_bias(source, payload);
    if learning_bias != 0 {
        severity += learning_bias;
        reasons.push(format!("learning bias applied ({:+})", learning_bias));
        reasons.extend(learning_reasons);
    }

    if mode == "strict" {
        severity += 1;
    } else if mode == "relaxed" {
        severity = severity.saturating_sub(1);
    }

    let (decision, requires_approval) = if severity >= 7 {
        (TriggerTriageDecision::Escalate, true)
    } else if severity >= 4 {
        (TriggerTriageDecision::AgentRun, false)
    } else if severity >= 2 {
        (TriggerTriageDecision::Notify, false)
    } else if payload_l.len() < 6 {
        (TriggerTriageDecision::Drop, false)
    } else {
        (TriggerTriageDecision::Notify, false)
    };

    TriggerTriageAssessment {
        source: source.trim().to_string(),
        payload: payload.trim().to_string(),
        severity,
        decision,
        requires_approval,
        reasons,
    }
}

fn render_trigger_triage_assessment(assessment: &TriggerTriageAssessment) -> String {
    let mut out = String::new();
    out.push_str("Trigger triage assessment\n");
    out.push_str("------------------------\n");
    let _ = writeln!(out, "source: {}", assessment.source);
    let _ = writeln!(out, "payload: {}", truncate_chars(&assessment.payload, 220));
    let _ = writeln!(out, "severity: {}", assessment.severity);
    let _ = writeln!(out, "decision: {}", assessment.decision.as_str());
    let _ = writeln!(out, "requires_approval: {}", assessment.requires_approval);
    if assessment.reasons.is_empty() {
        out.push_str("reasons: none\n");
    } else {
        out.push_str("reasons:\n");
        for reason in &assessment.reasons {
            let _ = writeln!(out, "- {}", reason);
        }
    }
    out
}

fn append_triage_learning_feedback(
    source: &str,
    payload: &str,
    outcome: &str,
    assessment: &TriggerTriageAssessment,
) -> Result<TriggerTriageLearningEntry, AgentError> {
    let delta = triage_feedback_delta(outcome).ok_or_else(|| {
        AgentError::Config(
            "Unknown triage feedback outcome. Use critical|confirmed|useful|neutral|false-positive|drop."
                .to_string(),
        )
    })?;
    let note = payload
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let entry = TriggerTriageLearningEntry {
        at: chrono::Utc::now().to_rfc3339(),
        source: source.trim().to_ascii_lowercase(),
        outcome: outcome.trim().to_ascii_lowercase(),
        decision: assessment.decision.as_str().to_string(),
        severity: assessment.severity,
        bias_delta: delta,
        note,
    };
    let mut state = load_trigger_triage_learning_state();
    state.entries.push(entry.clone());
    if state.entries.len() > 400 {
        let remove = state.entries.len().saturating_sub(400);
        state.entries.drain(0..remove);
    }
    save_trigger_triage_learning_state(&state)?;
    Ok(entry)
}

fn render_trigger_triage_learning_status() -> String {
    let state = load_trigger_triage_learning_state();
    let mut by_source: HashMap<String, i32> = HashMap::new();
    for entry in &state.entries {
        *by_source.entry(entry.source.clone()).or_insert(0) += entry.bias_delta;
    }
    let mut ranked = by_source.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out = String::new();
    out.push_str("Trigger triage learning\n");
    out.push_str("----------------------\n");
    let _ = writeln!(out, "entries: {}", state.entries.len());
    if ranked.is_empty() {
        out.push_str("source_bias: none\n");
    } else {
        out.push_str("source_bias:\n");
        for (source, bias) in ranked.into_iter().take(6) {
            let _ = writeln!(out, "- {} => {:+}", source, bias);
        }
    }
    if let Some(last) = state.entries.last() {
        let _ = writeln!(
            out,
            "last_feedback: {} source={} outcome={} delta={:+}",
            last.at, last.source, last.outcome, last.bias_delta
        );
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubconsciousTask {
    id: String,
    source: String,
    prompt: String,
    score: f64,
    risk: String,
    requires_approval: bool,
    status: String,
    #[serde(default)]
    job_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SubconsciousQueueState {
    #[serde(default)]
    tasks: Vec<SubconsciousTask>,
}

fn subconscious_state_path() -> PathBuf {
    hermes_config::hermes_home()
        .join("subconscious")
        .join("queue.json")
}

fn load_subconscious_state() -> SubconsciousQueueState {
    let path = subconscious_state_path();
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str::<SubconsciousQueueState>(&raw).unwrap_or_default()
}

fn save_subconscious_state(state: &SubconsciousQueueState) -> Result<(), AgentError> {
    let path = subconscious_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", parent.display(), e)))?;
    }
    let payload = serde_json::to_string_pretty(state)
        .map_err(|e| AgentError::Io(format!("Failed to encode subconscious state: {}", e)))?;
    std::fs::write(&path, payload)
        .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", path.display(), e)))?;
    Ok(())
}

fn score_subconscious_task(prompt: &str) -> f64 {
    let text = prompt.to_ascii_lowercase();
    let mut score = 1.0f64;
    if text.contains("profit")
        || text.contains("wallet")
        || text.contains("sol")
        || text.contains("latency")
        || text.contains("regression")
    {
        score += 1.2;
    }
    if text.contains("fix") || text.contains("verify") || text.contains("test") {
        score += 0.8;
    }
    if let Ok(terms) = utility_terms_from_contract() {
        let mut overlap = 0.0f64;
        for (term, weight) in terms {
            if text.contains(&term.to_ascii_lowercase()) {
                overlap += weight.max(0.0);
            }
        }
        score += overlap.min(2.5);
    }
    score
}

fn risk_for_prompt(prompt: &str) -> (&'static str, bool) {
    let text = prompt.to_ascii_lowercase();
    if text.contains("rm -rf")
        || text.contains("delete ")
        || text.contains("rotate key")
        || text.contains("prod")
        || text.contains("mainnet")
    {
        return ("high", true);
    }
    if text.contains("live trading") || text.contains("wallet") || text.contains("deploy") {
        return ("medium", true);
    }
    ("low", false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubconsciousProfile {
    Strict,
    Balanced,
    Dev,
}

impl SubconsciousProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Balanced => "balanced",
            Self::Dev => "dev",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(Self::Strict),
            "balanced" | "standard" => Some(Self::Balanced),
            "dev" => Some(Self::Dev),
            _ => None,
        }
    }
}

fn subconscious_profile_env() -> SubconsciousProfile {
    std::env::var("HERMES_SUBCONSCIOUS_PROFILE")
        .ok()
        .and_then(|v| SubconsciousProfile::parse(&v))
        .unwrap_or(SubconsciousProfile::Balanced)
}

fn subconscious_guard_allows(
    profile: SubconsciousProfile,
    task: &SubconsciousTask,
) -> (bool, String) {
    let risk = task.risk.to_ascii_lowercase();
    match profile {
        SubconsciousProfile::Dev => (true, "dev profile allows execution".to_string()),
        SubconsciousProfile::Balanced => {
            if risk == "high" {
                (
                    false,
                    "balanced profile blocks high-risk subconscious runs".to_string(),
                )
            } else {
                (true, "balanced profile allows low/medium risk".to_string())
            }
        }
        SubconsciousProfile::Strict => {
            if task.requires_approval || risk != "low" {
                (
                    false,
                    "strict profile allows only low-risk non-approval tasks".to_string(),
                )
            } else {
                (true, "strict profile allows low-risk task".to_string())
            }
        }
    }
}

fn handle_subconscious_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match action.as_str() {
        "status" | "list" => {
            let state = load_subconscious_state();
            let profile = subconscious_profile_env();
            let mut out = String::new();
            out.push_str("Subconscious queue\n");
            out.push_str("-----------------\n");
            let _ = writeln!(out, "profile: {}", profile.as_str());
            if state.tasks.is_empty() {
                out.push_str("No queued subconscious tasks.\n");
            } else {
                for task in state.tasks.iter().rev().take(24) {
                    let _ = writeln!(
                        out,
                        "- {} [{}] score={:.2} risk={} approval={} source={} :: {}",
                        task.id,
                        task.status,
                        task.score,
                        task.risk,
                        task.requires_approval,
                        task.source,
                        truncate_chars(&task.prompt, 100)
                    );
                }
            }
            out.push_str(
                "\nUsage: /subconscious add <prompt> | approve <id> | reject <id> | run [n] [--dry-run] [profile=<strict|balanced|dev>] | profile [status|list|strict|balanced|dev|clear] | clear",
            );
            emit_command_output(app, out.trim_end());
        }
        "add" => {
            let prompt = args.get(1..).unwrap_or(&[]).join(" ").trim().to_string();
            if prompt.is_empty() {
                emit_command_output(app, "Usage: /subconscious add <prompt>");
                return Ok(CommandResult::Handled);
            }
            let (risk, requires_approval) = risk_for_prompt(&prompt);
            let score = score_subconscious_task(&prompt);
            let mut state = load_subconscious_state();
            let id = format!(
                "sc-{}",
                uuid::Uuid::new_v4()
                    .simple()
                    .to_string()
                    .chars()
                    .take(8)
                    .collect::<String>()
            );
            let task = SubconsciousTask {
                id: id.clone(),
                source: "manual".to_string(),
                prompt,
                score,
                risk: risk.to_string(),
                requires_approval,
                status: if requires_approval {
                    "pending-approval".to_string()
                } else {
                    "pending".to_string()
                },
                job_id: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            state.tasks.push(task.clone());
            save_subconscious_state(&state)?;
            emit_command_output(
                app,
                format!(
                    "Queued subconscious task {}\nstatus={} score={:.2} risk={}\n{}",
                    task.id,
                    task.status,
                    task.score,
                    task.risk,
                    if task.requires_approval {
                        "Requires approval: /subconscious approve <id>"
                    } else {
                        "Ready to run: /subconscious run"
                    }
                ),
            );
        }
        "approve" | "reject" => {
            let Some(task_id) = args.get(1).copied() else {
                emit_command_output(app, format!("Usage: /subconscious {} <id>", action));
                return Ok(CommandResult::Handled);
            };
            let mut state = load_subconscious_state();
            let mut found = false;
            for task in &mut state.tasks {
                if task.id.eq_ignore_ascii_case(task_id) {
                    found = true;
                    task.status = if action == "approve" {
                        "pending".to_string()
                    } else {
                        "rejected".to_string()
                    };
                    task.updated_at = chrono::Utc::now().to_rfc3339();
                    break;
                }
            }
            if !found {
                emit_command_output(app, format!("Task not found: {}", task_id));
                return Ok(CommandResult::Handled);
            }
            save_subconscious_state(&state)?;
            emit_command_output(app, format!("Subconscious task {} {}", task_id, action));
        }
        "run" => {
            let mut limit = 1usize;
            let mut dry_run = false;
            let mut profile_override: Option<SubconsciousProfile> = None;
            for token in args.get(1..).unwrap_or(&[]) {
                let token_l = token.trim().to_ascii_lowercase();
                if token_l == "--dry-run" || token_l == "dry-run" || token_l == "preview" {
                    dry_run = true;
                    continue;
                }
                if let Ok(parsed) = token_l.parse::<usize>() {
                    limit = parsed.clamp(1, 8);
                    continue;
                }
                if let Some(raw) = token_l.strip_prefix("profile=") {
                    profile_override = SubconsciousProfile::parse(raw);
                    continue;
                }
                if profile_override.is_none() {
                    profile_override = SubconsciousProfile::parse(&token_l);
                }
            }
            let profile = profile_override.unwrap_or_else(subconscious_profile_env);
            let mut state = load_subconscious_state();
            let mut reviewed = 0usize;
            let mut dispatched = 0usize;
            let mut blocked = 0usize;
            let mut notes = Vec::new();
            for task in &mut state.tasks {
                if reviewed >= limit {
                    break;
                }
                if task.status != "pending" {
                    continue;
                }
                reviewed += 1;
                let (allowed, guard_note) = subconscious_guard_allows(profile, task);
                if !allowed {
                    blocked += 1;
                    notes.push(format!("{} blocked ({})", task.id, guard_note));
                    continue;
                }
                if dry_run {
                    notes.push(format!("{} would dispatch ({})", task.id, guard_note));
                    continue;
                }
                let job = background::queue_background_job(&task.prompt)?;
                task.status = "dispatched".to_string();
                task.job_id = Some(job.id.clone());
                task.updated_at = chrono::Utc::now().to_rfc3339();
                dispatched += 1;
                notes.push(format!("{} dispatched id={}", task.id, job.id));
            }
            if !dry_run {
                save_subconscious_state(&state)?;
            }
            emit_command_output(
                app,
                format!(
                    "{} subconscious run profile={}\nreviewed={} dispatched={} blocked={}\n{}\nUse `/background status` and `/subconscious status` for tracking.",
                    if dry_run { "Dry-run" } else { "Executed" },
                    profile.as_str(),
                    reviewed,
                    dispatched,
                    blocked,
                    if notes.is_empty() {
                        "No pending tasks matched selection.".to_string()
                    } else {
                        notes.join("\n")
                    }
                ),
            );
        }
        "profile" => {
            let token = args
                .get(1)
                .copied()
                .unwrap_or("status")
                .to_ascii_lowercase();
            match token.as_str() {
                "status" | "show" => emit_command_output(
                    app,
                    format!(
                        "Subconscious profile: {}\nUse `/subconscious profile list` or `/subconscious profile strict|balanced|dev`.",
                        subconscious_profile_env().as_str()
                    ),
                ),
                "list" => emit_command_output(
                    app,
                    "Subconscious profiles:\n- strict: only low-risk non-approval tasks auto-dispatch\n- balanced: low/medium dispatch, high-risk blocked\n- dev: permit all pending tasks\nSet with `/subconscious profile <name>`.",
                ),
                "clear" => {
                    crate::env_vars::remove_var("HERMES_SUBCONSCIOUS_PROFILE");
                    emit_command_output(
                        app,
                        "Cleared subconscious profile override (default=balanced).",
                    );
                }
                other => {
                    let Some(next) = SubconsciousProfile::parse(other) else {
                        emit_command_output(
                            app,
                            "Usage: /subconscious profile [status|list|strict|balanced|dev|clear]",
                        );
                        return Ok(CommandResult::Handled);
                    };
                    crate::env_vars::set_var("HERMES_SUBCONSCIOUS_PROFILE", next.as_str());
                    emit_command_output(
                        app,
                        format!("Subconscious profile set to {}.", next.as_str()),
                    );
                }
            }
        }
        "clear" => {
            let path = subconscious_state_path();
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| {
                    AgentError::Io(format!("Failed to remove {}: {}", path.display(), e))
                })?;
            }
            emit_command_output(app, "Cleared subconscious queue.");
        }
        _ => emit_command_output(
            app,
            "Usage: /subconscious [status|add <prompt>|approve <id>|reject <id>|run [n] [--dry-run] [profile=<strict|balanced|dev>]|profile [status|list|strict|balanced|dev|clear]|clear]",
        ),
    }
    Ok(CommandResult::Handled)
}

fn handle_trigger_triage_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match action.as_str() {
        "status" => {
            emit_command_output(
                app,
                format!(
                    "Trigger triage mode: {}\n{}\nUsage: /triage eval <source> <payload> | /triage queue <source> <payload> | /triage feedback <source> <outcome> <payload>",
                    trigger_triage_mode(),
                    render_trigger_triage_learning_status().trim_end()
                ),
            );
        }
        "list" | "rules" => {
            emit_command_output(
                app,
                "Trigger triage heuristics\n\
                 - high severity: panic/outage/secret leak/drawdown/halt -> escalate\n\
                 - medium severity: repeated errors/blocked/timeout -> agent-run\n\
                 - low severity: notify\n\
                 - empty/noise payload -> drop\n\
                 Mode override: HERMES_TRIGGER_TRIAGE_MODE={strict|balanced|relaxed}\n\
                 Feedback loop: `/triage feedback <source> <outcome> <payload>` updates persistent bias.",
            );
        }
        "feedback" => {
            let Some(source) = args.get(1).copied() else {
                emit_command_output(app, "Usage: /triage feedback <source> <outcome> <payload>");
                return Ok(CommandResult::Handled);
            };
            let Some(outcome) = args.get(2).copied() else {
                emit_command_output(app, "Usage: /triage feedback <source> <outcome> <payload>");
                return Ok(CommandResult::Handled);
            };
            let payload = args.get(3..).unwrap_or(&[]).join(" ").trim().to_string();
            if payload.is_empty() {
                emit_command_output(app, "Usage: /triage feedback <source> <outcome> <payload>");
                return Ok(CommandResult::Handled);
            }
            let assessment = evaluate_trigger_triage(source, &payload);
            let entry = append_triage_learning_feedback(source, &payload, outcome, &assessment)?;
            let (bias_now, _) = triage_learning_bias(source, &payload);
            emit_command_output(
                app,
                format!(
                    "Recorded triage feedback.\nsource={} outcome={} delta={:+} decision={} severity={}\nsource_bias_now={:+}",
                    entry.source,
                    entry.outcome,
                    entry.bias_delta,
                    entry.decision,
                    entry.severity,
                    bias_now
                ),
            );
        }
        "eval" | "queue" => {
            let Some(source) = args.get(1).copied() else {
                emit_command_output(
                    app,
                    "Usage: /triage eval <source> <payload>\nUsage: /triage queue <source> <payload>",
                );
                return Ok(CommandResult::Handled);
            };
            let payload = args.get(2..).unwrap_or(&[]).join(" ");
            if payload.trim().is_empty() {
                emit_command_output(app, "Payload cannot be empty.");
                return Ok(CommandResult::Handled);
            }
            let assessment = evaluate_trigger_triage(source, &payload);
            let mut out = render_trigger_triage_assessment(&assessment);
            if action == "queue" {
                match assessment.decision {
                    TriggerTriageDecision::Drop => {
                        out.push_str("\n\nqueue_action: dropped");
                    }
                    TriggerTriageDecision::Notify => {
                        out.push_str("\n\nqueue_action: notify-only (no agent run queued)");
                    }
                    TriggerTriageDecision::Escalate => {
                        let mut state = load_subconscious_state();
                        let id = format!(
                            "sc-{}",
                            uuid::Uuid::new_v4()
                                .simple()
                                .to_string()
                                .chars()
                                .take(8)
                                .collect::<String>()
                        );
                        state.tasks.push(SubconsciousTask {
                            id: id.clone(),
                            source: source.to_string(),
                            prompt: payload.trim().to_string(),
                            score: score_subconscious_task(&payload),
                            risk: "high".to_string(),
                            requires_approval: true,
                            status: "pending-approval".to_string(),
                            job_id: None,
                            created_at: chrono::Utc::now().to_rfc3339(),
                            updated_at: chrono::Utc::now().to_rfc3339(),
                        });
                        save_subconscious_state(&state)?;
                        let _ = write!(
                            out,
                            "\n\nqueue_action: escalated to subconscious queue as {} (requires approval)",
                            id
                        );
                    }
                    TriggerTriageDecision::AgentRun => {
                        let job = background::queue_background_job(payload.trim())?;
                        let _ = write!(
                            out,
                            "\n\nqueue_action: background job queued id={} status_file={}",
                            job.id,
                            job.status_path.display()
                        );
                    }
                }
            }
            emit_command_output(app, out);
        }
        _ => emit_command_output(
            app,
            "Usage: /triage [status|list|eval <source> <payload>|queue <source> <payload>|feedback <source> <outcome> <payload>]",
        ),
    }
    Ok(CommandResult::Handled)
}

fn handle_verbose_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let current = tracing::enabled!(tracing::Level::DEBUG);
    if current {
        emit_command_output(
            app,
            "Verbose mode: OFF (switching to info level)\n(Runtime log level changes require restart — use `hermes -v` for verbose)",
        );
    } else {
        emit_command_output(
            app,
            "Verbose mode: ON (switching to debug level)\n(Runtime log level changes require restart — use `hermes -v` for verbose)",
        );
    }
    Ok(CommandResult::Handled)
}

fn handle_yolo_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let currently_required = app.config.approval.require_approval;
    let new_val = !currently_required;

    app.config = Arc::new({
        let mut cfg = (*app.config).clone();
        cfg.approval.require_approval = new_val;
        cfg
    });

    if !new_val {
        emit_command_output(
            app,
            "YOLO mode: ON — tool executions will not require approval.\nBe careful! The agent can now execute tools without confirmation.",
        );
    } else {
        emit_command_output(
            app,
            "YOLO mode: OFF — tool executions will require approval.",
        );
    }
    Ok(CommandResult::Handled)
}

fn reasoning_display_flag() -> &'static std::sync::atomic::AtomicBool {
    static SHOW_REASONING: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    &SHOW_REASONING
}

fn set_reasoning_display(enabled: bool) {
    reasoning_display_flag().store(enabled, std::sync::atomic::Ordering::Relaxed);
}

fn toggle_reasoning_display() -> bool {
    let prev = reasoning_display_flag().fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
    !prev
}

fn reasoning_display_enabled() -> bool {
    reasoning_display_flag().load(std::sync::atomic::Ordering::Relaxed)
}

fn parse_reasoning_effort(raw: &str) -> Result<Option<&'static str>, AgentError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "minimal" | "min" => Ok(Some("minimal")),
        "low" => Ok(Some("low")),
        "medium" | "med" => Ok(Some("medium")),
        "high" => Ok(Some("high")),
        "xhigh" | "max" => Ok(Some("xhigh")),
        "auto" | "default" | "clear" | "reset" | "none" => Ok(None),
        other => Err(AgentError::Config(format!(
            "Unknown reasoning effort '{}'. Use one of: minimal, low, medium, high, xhigh, auto.",
            other
        ))),
    }
}

fn set_provider_reasoning_effort(cfg: &mut GatewayConfig, provider: &str, effort: Option<&str>) {
    let provider_key = model::resolve_provider_key(cfg, provider);
    let provider_cfg = cfg
        .llm_providers
        .entry(provider_key.clone())
        .or_insert_with(LlmProviderConfig::default);

    let mut body_map = provider_cfg
        .extra_body
        .take()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    match effort {
        Some(level) => {
            // Keep request payloads OpenAI-compatible for Nous/OpenRouter/OpenAI routes:
            // use `reasoning.effort` (`low|medium|high`) instead of legacy top-level
            // `reasoning_effort` which can trigger schema validation errors.
            body_map.remove("reasoning_effort");
            let mut reasoning_obj = body_map
                .get("reasoning")
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            let mapped_reasoning = model::openai_reasoning_effort_for_level(level);
            reasoning_obj.insert(
                "effort".to_string(),
                serde_json::Value::String(mapped_reasoning.to_string()),
            );
            body_map.insert(
                "reasoning".to_string(),
                serde_json::Value::Object(reasoning_obj),
            );

            if provider_key.contains("gemini") || provider_key == "google" {
                let level_mapped = model::gemini_thinking_level_for_effort(level);
                let mut google_obj = body_map
                    .get("google")
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                let mut thinking_cfg = google_obj
                    .get("thinking_config")
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                thinking_cfg.insert(
                    "thinking_level".to_string(),
                    serde_json::Value::String(level_mapped.to_string()),
                );
                google_obj.insert(
                    "thinking_config".to_string(),
                    serde_json::Value::Object(thinking_cfg.clone()),
                );
                body_map.insert("google".to_string(), serde_json::Value::Object(google_obj));
                body_map.insert(
                    "thinking_config".to_string(),
                    serde_json::Value::Object(thinking_cfg),
                );
            }
        }
        None => {
            body_map.remove("reasoning_effort");
            if let Some(reasoning_obj) = body_map
                .get_mut("reasoning")
                .and_then(|value| value.as_object_mut())
            {
                reasoning_obj.remove("effort");
                if reasoning_obj.is_empty() {
                    body_map.remove("reasoning");
                }
            }
            body_map.remove("thinking_config");
            if let Some(google_obj) = body_map
                .get_mut("google")
                .and_then(|value| value.as_object_mut())
            {
                google_obj.remove("thinking_config");
                if google_obj.is_empty() {
                    body_map.remove("google");
                }
            }
        }
    }

    provider_cfg.extra_body = if body_map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(body_map))
    };
}

fn provider_reasoning_effort(cfg: &GatewayConfig, provider: &str) -> Option<String> {
    let provider_key = model::resolve_provider_key(cfg, provider);
    cfg.llm_providers
        .get(&provider_key)
        .and_then(|entry| entry.extra_body.as_ref())
        .and_then(|body| {
            body.get("reasoning")
                .and_then(|value| value.get("effort"))
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    body.get("reasoning_effort")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string)
                })
        })
}

fn handle_reasoning_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        let enabled = toggle_reasoning_display();
        if enabled {
            emit_command_output(
                app,
                "Reasoning display: ON — model reasoning will be shown.",
            );
        } else {
            emit_command_output(
                app,
                "Reasoning display: OFF — model reasoning will be hidden.",
            );
        }
        return Ok(CommandResult::Handled);
    }

    match args[0].trim().to_ascii_lowercase().as_str() {
        "status" => {
            let (provider, _) = split_provider_model(&app.current_model);
            let effort = provider_reasoning_effort(&app.config, provider)
                .unwrap_or_else(|| "auto".to_string());
            emit_command_output(
                app,
                format!(
                    "Reasoning status\n- display: {}\n- effort: {}\n- provider: {}",
                    if reasoning_display_enabled() {
                        "ON"
                    } else {
                        "OFF"
                    },
                    effort,
                    provider
                ),
            );
        }
        "toggle" => {
            let enabled = toggle_reasoning_display();
            emit_command_output(
                app,
                format!(
                    "Reasoning display: {} — model reasoning will be {}.",
                    if enabled { "ON" } else { "OFF" },
                    if enabled { "shown" } else { "hidden" }
                ),
            );
        }
        "on" | "show" => {
            set_reasoning_display(true);
            emit_command_output(
                app,
                "Reasoning display: ON — model reasoning will be shown.",
            );
        }
        "off" | "hide" => {
            set_reasoning_display(false);
            emit_command_output(
                app,
                "Reasoning display: OFF — model reasoning will be hidden.",
            );
        }
        "set" | "level" | "effort" => {
            if args.len() < 2 {
                emit_command_output(
                    app,
                    "Usage: /reasoning set <minimal|low|medium|high|xhigh|auto>",
                );
                return Ok(CommandResult::Handled);
            }
            let effort = parse_reasoning_effort(args[1])?;
            let provider = split_provider_model(&app.current_model).0.to_string();
            let current_model = app.current_model.clone();
            app.config = Arc::new({
                let mut cfg = (*app.config).clone();
                set_provider_reasoning_effort(&mut cfg, &provider, effort);
                cfg
            });
            app.switch_model(&current_model);
            let effort_label = effort.unwrap_or("auto");
            emit_command_output(
                app,
                format!(
                    "Reasoning effort set to `{}` for provider `{}` (model `{}`).",
                    effort_label, provider, current_model
                ),
            );
        }
        "help" => {
            emit_command_output(
                app,
                "Reasoning controls:\n\
                 - /reasoning                 Toggle reasoning display\n\
                 - /reasoning status          Show display + effort state\n\
                 - /reasoning on|off          Explicitly show/hide reasoning\n\
                 - /reasoning set <level>     Set provider reasoning effort\n\
                 Levels: minimal, low, medium, high, xhigh, auto",
            );
        }
        shorthand => {
            let effort = parse_reasoning_effort(shorthand)?;
            let provider = split_provider_model(&app.current_model).0.to_string();
            let current_model = app.current_model.clone();
            app.config = Arc::new({
                let mut cfg = (*app.config).clone();
                set_provider_reasoning_effort(&mut cfg, &provider, effort);
                cfg
            });
            app.switch_model(&current_model);
            emit_command_output(
                app,
                format!(
                    "Reasoning effort set to `{}` for provider `{}` (model `{}`).",
                    effort.unwrap_or("auto"),
                    provider,
                    current_model
                ),
            );
        }
    }
    Ok(CommandResult::Handled)
}

fn replay_enabled_runtime() -> bool {
    std::env::var("HERMES_REPLAY_ENABLED")
        .ok()
        .is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

fn replay_log_path_for_session(session_id: &str) -> PathBuf {
    let sid = if session_id.trim().is_empty() {
        "session".to_string()
    } else {
        session_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    hermes_config::hermes_home()
        .join("logs")
        .join("replay")
        .join(format!("{sid}.jsonl"))
}

fn render_replay_trace_tail(path: &Path, limit: usize) -> Result<String, AgentError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        AgentError::Io(format!(
            "Failed to read replay log {}: {}",
            path.display(),
            e
        ))
    })?;
    let lines: Vec<&str> = raw
        .lines()
        .rev()
        .take(limit.max(1))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if lines.is_empty() {
        return Ok("Replay log is empty.".to_string());
    }
    let mut out = String::new();
    for line in lines {
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                let _ = writeln!(out, "{}", line);
                continue;
            }
        };
        let seq = value
            .get("seq")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".to_string());
        let event = value
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let trace_id = value
            .get("trace_id")
            .and_then(|v| v.as_str())
            .unwrap_or("missing");
        let prev_hash = value
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let event_hash = value
            .get("event_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let turn = value
            .get("payload")
            .and_then(|payload| payload.get("turn"))
            .and_then(|turn| turn.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string());
        let _ = writeln!(
            out,
            "#{seq:<4} turn={turn:<3} event={event:<24} trace={trace_id} prev={prev_hash} hash={event_hash}"
        );
    }
    Ok(out.trim_end().to_string())
}

fn replay_entries(path: &Path, limit: usize) -> Result<Vec<serde_json::Value>, AgentError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        AgentError::Io(format!(
            "Failed to read replay log {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(raw
        .lines()
        .rev()
        .take(limit.max(1))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect())
}

fn render_replay_trace_focus(
    path: &Path,
    trace_id: &str,
    limit: usize,
) -> Result<String, AgentError> {
    let trace_filter = trace_id.trim();
    if trace_filter.is_empty() {
        return Ok("Usage: /raw trace focus <trace-id> [N]".to_string());
    }
    let rows = replay_entries(path, limit)?;
    let filtered: Vec<serde_json::Value> = rows
        .into_iter()
        .filter(|row| {
            row.get("trace_id")
                .and_then(|v| v.as_str())
                .is_some_and(|v| v == trace_filter || v.contains(trace_filter))
        })
        .collect();
    if filtered.is_empty() {
        return Ok(format!(
            "No replay events found for trace '{}' in {}.",
            trace_filter,
            path.display()
        ));
    }
    let mut out = String::new();
    let _ = writeln!(out, "Replay trace focus: {}", trace_filter);
    let _ = writeln!(out, "events: {}", filtered.len());
    let _ = writeln!(out, "path: {}", path.display());
    let _ = writeln!(out);
    for row in filtered {
        let seq = row.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
        let event = row
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let turn = row
            .get("payload")
            .and_then(|payload| payload.get("turn"))
            .and_then(|turn| turn.as_u64())
            .unwrap_or(0);
        let preview = row
            .get("payload")
            .map(|v| truncate_chars(&v.to_string(), 120))
            .unwrap_or_else(|| "{}".to_string());
        let _ = writeln!(out, "#{seq:<4} turn={turn:<3} event={event:<24} {preview}");
    }
    Ok(out.trim_end().to_string())
}

fn render_replay_trace_graph(path: &Path, limit: usize) -> Result<String, AgentError> {
    let rows = replay_entries(path, limit)?;
    if rows.is_empty() {
        return Ok("Replay graph: no entries in current window.".to_string());
    }
    let mut out = String::new();
    let _ = writeln!(out, "Replay lineage graph");
    let _ = writeln!(out, "--------------------");
    let _ = writeln!(out, "window={} path={}", rows.len(), path.display());
    for row in rows {
        let seq = row
            .get("seq")
            .and_then(|value| value.as_u64())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".to_string());
        let event = row
            .get("event")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let trace_id = row
            .get("trace_id")
            .and_then(|value| value.as_str())
            .unwrap_or("missing");
        let prev = row
            .get("prev_hash")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let hash = row
            .get("event_hash")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        let _ = writeln!(
            out,
            "#{:<4} {:<20} trace={} {} -> {}",
            seq, event, trace_id, prev, hash
        );
    }
    Ok(out.trim_end().to_string())
}

fn replay_trace_integrity(path: &Path) -> Result<(usize, usize, usize), AgentError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        AgentError::Io(format!(
            "Failed to read replay log {}: {}",
            path.display(),
            e
        ))
    })?;
    let mut entries = 0usize;
    let mut parse_errors = 0usize;
    let mut chain_breaks = 0usize;
    let mut last_event_hash: Option<String> = None;
    for line in raw.lines() {
        let parsed: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                parse_errors = parse_errors.saturating_add(1);
                continue;
            }
        };
        entries = entries.saturating_add(1);
        let prev_hash = parsed
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let event_hash = parsed
            .get("event_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let (Some(last), Some(prev)) = (last_event_hash.as_ref(), prev_hash.as_ref()) {
            if last != prev {
                chain_breaks = chain_breaks.saturating_add(1);
            }
        }
        if let Some(curr) = event_hash {
            last_event_hash = Some(curr);
        }
    }
    Ok((entries, parse_errors, chain_breaks))
}

fn export_replay_trace_json(
    replay_path: &Path,
    limit: usize,
    output_path: &Path,
) -> Result<usize, AgentError> {
    let raw = std::fs::read_to_string(replay_path).map_err(|e| {
        AgentError::Io(format!(
            "Failed to read replay log {}: {}",
            replay_path.display(),
            e
        ))
    })?;
    let rows: Vec<serde_json::Value> = raw
        .lines()
        .rev()
        .take(limit.max(1))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let payload = serde_json::json!({
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "source_replay": replay_path.display().to_string(),
        "rows": rows,
    });
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AgentError::Io(format!(
                "Failed to create replay export directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }
    std::fs::write(
        output_path,
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string()),
    )
    .map_err(|e| {
        AgentError::Io(format!(
            "Failed to write replay export {}: {}",
            output_path.display(),
            e
        ))
    })?;
    Ok(payload["rows"].as_array().map(|arr| arr.len()).unwrap_or(0))
}

fn handle_raw_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args
        .first()
        .is_some_and(|sub| sub.eq_ignore_ascii_case("trace"))
    {
        let replay_path = replay_log_path_for_session(&app.session_id);
        let sub = args.get(1).map(|s| s.trim().to_ascii_lowercase());
        match sub.as_deref() {
            None | Some("status") => {
                emit_command_output(
                    app,
                    format!(
                        "Replay trace: {}{}\nSession: {}\nPath: {}\nUsage: /raw trace [on|off|toggle|status|tail [N]|focus <trace-id> [N]|graph [N]|verify|export [N] [PATH]|path]",
                        if replay_enabled_runtime() {
                            "ON"
                        } else {
                            "OFF"
                        },
                        if replay_path.exists() {
                            ""
                        } else {
                            " (no log yet)"
                        },
                        app.session_id,
                        replay_path.display()
                    ),
                );
            }
            Some("path") => {
                emit_command_output(app, format!("Replay path: {}", replay_path.display()));
            }
            Some("tail") => {
                let limit = args
                    .get(2)
                    .and_then(|raw| raw.trim().parse::<usize>().ok())
                    .unwrap_or(20)
                    .clamp(1, 200);
                if !replay_path.exists() {
                    emit_command_output(
                        app,
                        format!(
                            "Replay log not found for current session yet: {}",
                            replay_path.display()
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                let rendered = render_replay_trace_tail(&replay_path, limit)?;
                emit_command_output(app, rendered);
            }
            Some("focus") => {
                let Some(trace_id) = args.get(2).copied() else {
                    emit_command_output(app, "Usage: /raw trace focus <trace-id> [N]");
                    return Ok(CommandResult::Handled);
                };
                let limit = args
                    .get(3)
                    .and_then(|raw| raw.trim().parse::<usize>().ok())
                    .unwrap_or(150)
                    .clamp(1, 1000);
                if !replay_path.exists() {
                    emit_command_output(
                        app,
                        format!(
                            "Replay log not found for current session yet: {}",
                            replay_path.display()
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                let rendered = render_replay_trace_focus(&replay_path, trace_id, limit)?;
                emit_command_output(app, rendered);
            }
            Some("graph") => {
                let limit = args
                    .get(2)
                    .and_then(|raw| raw.trim().parse::<usize>().ok())
                    .unwrap_or(80)
                    .clamp(1, 500);
                if !replay_path.exists() {
                    emit_command_output(
                        app,
                        format!(
                            "Replay log not found for current session yet: {}",
                            replay_path.display()
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                let rendered = render_replay_trace_graph(&replay_path, limit)?;
                emit_command_output(app, rendered);
            }
            Some("verify") => {
                if !replay_path.exists() {
                    emit_command_output(
                        app,
                        format!(
                            "Replay log not found for current session yet: {}",
                            replay_path.display()
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                let (entries, parse_errors, chain_breaks) = replay_trace_integrity(&replay_path)?;
                let ok = parse_errors == 0 && chain_breaks == 0;
                emit_command_output(
                    app,
                    format!(
                        "Replay integrity: {}\nentries: {}\nparse_errors: {}\nchain_breaks: {}\npath: {}",
                        if ok { "PASS" } else { "FAIL" },
                        entries,
                        parse_errors,
                        chain_breaks,
                        replay_path.display()
                    ),
                );
            }
            Some("export") => {
                let limit = args
                    .get(2)
                    .and_then(|raw| raw.trim().parse::<usize>().ok())
                    .unwrap_or(100)
                    .clamp(1, 1000);
                let output_path = args.get(3).map(PathBuf::from).unwrap_or_else(|| {
                    hermes_config::hermes_home()
                        .join("logs")
                        .join("replay")
                        .join("exports")
                        .join(format!("{}-tail.json", app.session_id))
                });
                if !replay_path.exists() {
                    emit_command_output(
                        app,
                        format!(
                            "Replay log not found for current session yet: {}",
                            replay_path.display()
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
                let written = export_replay_trace_json(&replay_path, limit, &output_path)?;
                emit_command_output(
                    app,
                    format!(
                        "Replay export written.\nrows: {}\nsource: {}\noutput: {}",
                        written,
                        replay_path.display(),
                        output_path.display()
                    ),
                );
            }
            Some("on") | Some("off") | Some("toggle") => {
                let next = match sub.as_deref().unwrap_or("status") {
                    "on" => true,
                    "off" => false,
                    "toggle" => !replay_enabled_runtime(),
                    _ => replay_enabled_runtime(),
                };
                crate::env_vars::set_var("HERMES_REPLAY_ENABLED", if next { "1" } else { "0" });
                emit_command_output(
                    app,
                    format!(
                        "Replay trace mode: {}.\nThis applies to new turns in the current process.",
                        if next { "ON" } else { "OFF" }
                    ),
                );
            }
            Some("help") | Some("--help") | Some("-h") => emit_command_output(
                app,
                "Replay trace controls:\n  /raw trace status              Show enabled state + current log path\n  /raw trace on|off              Enable or disable deterministic replay trace logs\n  /raw trace toggle              Toggle replay trace logs\n  /raw trace tail [N]            Show latest trace events with lineage hashes\n  /raw trace focus <id> [N]      Filter replay rows by trace_id\n  /raw trace graph [N]           Show lineage edges for recent rows\n  /raw trace verify              Validate replay hash-chain integrity\n  /raw trace export [N] [PATH]   Export tail events to JSON\n  /raw trace path                Show trace log file for current session",
            ),
            _ => emit_command_output(
                app,
                "Usage: /raw trace [on|off|toggle|status|tail [N]|focus <trace-id> [N]|graph [N]|verify|export [N] [PATH]|path]",
            ),
        }
        return Ok(CommandResult::Handled);
    }

    let state = app.tool_registry.raw_mode_state();
    let log_dir = app.tool_registry.rtk_log_dir();
    if args.is_empty() || args[0].eq_ignore_ascii_case("status") {
        emit_command_output(
            app,
            format!(
                "RTK raw mode: {}{}\nDual logs: {}\nReplay trace: {}\nUsage: /raw [on|off|toggle|once|status|trace]",
                if state.enabled { "ON" } else { "OFF" },
                if state.once_pending {
                    " (one-shot pending)"
                } else {
                    ""
                },
                log_dir.display(),
                if replay_enabled_runtime() {
                    "ON"
                } else {
                    "OFF"
                }
            ),
        );
        return Ok(CommandResult::Handled);
    }

    match args[0].trim().to_ascii_lowercase().as_str() {
        "help" => emit_command_output(
            app,
            "RTK raw controls:\n  /raw status        Show current mode + log path\n  /raw on            Disable output filtering for all tool calls\n  /raw off           Re-enable RTK output filtering\n  /raw toggle        Toggle global raw mode\n  /raw once          Raw pass-through for next tool call only\n  /raw trace ...     Deterministic replay trace controls",
        ),
        "once" => {
            app.tool_registry.set_raw_mode_once();
            emit_command_output(
                app,
                "RTK raw mode armed for next tool call only. It auto-resets after one dispatch.",
            );
        }
        "on" | "off" | "toggle" | "true" | "false" | "yes" | "no" | "1" | "0" => {
            let next = match args[0].trim().to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" | "1" => true,
                "off" | "false" | "no" | "0" => false,
                "toggle" => !state.enabled,
                _ => state.enabled,
            };
            app.tool_registry.set_raw_mode(next);
            crate::env_vars::set_var("HERMES_RTK_RAW", if next { "1" } else { "0" });
            emit_command_output(
                app,
                format!(
                    "RTK raw mode: {} (dual logs: {})",
                    if next { "ON" } else { "OFF" },
                    log_dir.display()
                ),
            );
        }
        _ => emit_command_output(app, "Usage: /raw [on|off|toggle|once|status|trace]"),
    }
    Ok(CommandResult::Handled)
}

fn handle_history_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let sp = session::session_db(app);
    let db_messages = if sp.ensure_db().is_ok() && !app.session_id.is_empty() {
        sp.load_session(&app.session_id).ok()
    } else {
        None
    };

    let transcript: Vec<_> = db_messages
        .as_ref()
        .filter(|m| !m.is_empty())
        .cloned()
        .unwrap_or_else(|| app.transcript_messages());

    if transcript.is_empty() {
        emit_command_output(app, "No conversation history yet.");
        return Ok(CommandResult::Handled);
    }

    let source_note = if db_messages.is_some() {
        " (from state.db)"
    } else {
        ""
    };
    let mut out = format!("Recent conversation history{source_note}:\n");
    for (idx, msg) in transcript.iter().enumerate().rev().take(12).rev() {
        let role = match msg.role {
            hermes_core::MessageRole::User => "USER",
            hermes_core::MessageRole::Assistant => "HERMES",
            hermes_core::MessageRole::System => "SYSTEM",
            hermes_core::MessageRole::Tool => "TOOL",
        };
        let preview =
            hermes_agent::session_persistence::decode_content_preview(msg.content.as_deref());
        let preview = preview.lines().next().unwrap_or("").trim();
        let clipped = if preview.chars().count() > 96 {
            let mut s: String = preview.chars().take(95).collect();
            s.push('…');
            s
        } else {
            preview.to_string()
        };
        let _ = writeln!(out, "{:>3}. {:<7} {}", idx + 1, role, clipped);
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

pub(crate) fn truncate_chars(input: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    if input.chars().count() <= max_len {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max_len.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn handle_recap_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let requested = args
        .first()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .unwrap_or(24)
        .clamp(1, 200);
    let transcript = app.transcript_messages();
    if transcript.is_empty() {
        emit_command_output(app, "No activity yet. Start with a prompt first.");
        return Ok(CommandResult::Handled);
    }

    let start = transcript.len().saturating_sub(requested);
    let window = &transcript[start..];
    let mut user_msgs = 0usize;
    let mut assistant_msgs = 0usize;
    let mut tool_msgs = 0usize;
    let mut system_msgs = 0usize;
    let mut tool_call_count = 0usize;
    let mut char_count = 0usize;

    for msg in window {
        match msg.role {
            hermes_core::MessageRole::User => user_msgs += 1,
            hermes_core::MessageRole::Assistant => assistant_msgs += 1,
            hermes_core::MessageRole::Tool => tool_msgs += 1,
            hermes_core::MessageRole::System => system_msgs += 1,
        }
        tool_call_count += msg.tool_calls.as_ref().map(|c| c.len()).unwrap_or(0);
        char_count += msg.content.as_deref().map(str::len).unwrap_or(0);
    }

    let latest_user = window
        .iter()
        .rev()
        .find(|m| matches!(m.role, hermes_core::MessageRole::User))
        .and_then(|m| m.content.as_deref())
        .map(|c| truncate_chars(c.trim(), 120))
        .unwrap_or_else(|| "(none)".to_string());
    let latest_assistant = window
        .iter()
        .rev()
        .find(|m| matches!(m.role, hermes_core::MessageRole::Assistant))
        .and_then(|m| m.content.as_deref())
        .map(|c| truncate_chars(c.trim(), 120))
        .unwrap_or_else(|| "(none)".to_string());

    let approx_tokens = (char_count / 4).max(1);
    emit_command_output(
        app,
        format!(
            "Session recap (last {} messages)\n\
             model: {}\n\
             roles: user={} assistant={} tool={} system={}\n\
             tool_calls: {}\n\
             approx_tokens: {}\n\
             latest_user: {}\n\
             latest_hermes: {}",
            window.len(),
            app.current_model,
            user_msgs,
            assistant_msgs,
            tool_msgs,
            system_msgs,
            tool_call_count,
            approx_tokens,
            latest_user,
            latest_assistant
        ),
    );
    Ok(CommandResult::Handled)
}

async fn handle_context_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match action.as_str() {
        "status" => {
            let transcript = app.transcript_messages();
            let total_chars: usize = transcript
                .iter()
                .map(|m| m.content.as_deref().map(str::len).unwrap_or(0))
                .sum();
            let approx_tokens = (total_chars / 4).max(1);
            let context_files = if app.config.agent.skip_context_files {
                "disabled"
            } else {
                "enabled"
            };
            emit_command_output(
                app,
                format!(
                    "Context status\n\
                     model: {}\n\
                     transcript_messages: {}\n\
                     approx_tokens: {}\n\
                     context_files: {}\n\
                     hint: run `/context breakdown` for per-message footprint or `/context compress` for immediate compaction",
                    app.current_model,
                    transcript.len(),
                    approx_tokens,
                    context_files
                ),
            );
        }
        "breakdown" => {
            let transcript = app.transcript_messages();
            if transcript.is_empty() {
                emit_command_output(app, "No transcript yet.");
                return Ok(CommandResult::Handled);
            }
            let mut out = String::from("Context breakdown (recent)\n");
            for (idx, msg) in transcript.iter().enumerate().rev().take(20).rev() {
                let role = match msg.role {
                    hermes_core::MessageRole::User => "USER",
                    hermes_core::MessageRole::Assistant => "HERMES",
                    hermes_core::MessageRole::Tool => "TOOL",
                    hermes_core::MessageRole::System => "SYSTEM",
                };
                let chars = msg.content.as_deref().map(str::len).unwrap_or(0);
                let est_tokens = (chars / 4).max(1);
                let preview = msg
                    .content
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim();
                let _ = writeln!(
                    out,
                    "{:>3}. {:<7} chars={:<5} tok≈{:<5} {}",
                    idx + 1,
                    role,
                    chars,
                    est_tokens,
                    truncate_chars(preview, 70)
                );
            }
            emit_command_output(app, out.trim_end());
        }
        "compress" | "compact" => {
            return compress::handle_compress_command(app, &[]).await;
        }
        _ => {
            emit_command_output(
                app,
                "Usage: /context [status|breakdown|compress]\nAlias: /summary -> /recap",
            );
        }
    }
    Ok(CommandResult::Handled)
}

async fn handle_provider_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let providers = curated_provider_slugs();
    if providers.is_empty() {
        emit_command_output(app, "No providers registered.");
        return Ok(CommandResult::Handled);
    }
    let entries = crate::model_switch::provider_catalog_entries(&providers, 4).await;
    if entries.is_empty() {
        emit_command_output(
            app,
            format!(
                "Configured providers: {}\nCurrent model: {}",
                providers.join(", "),
                app.current_model
            ),
        );
        return Ok(CommandResult::Handled);
    }
    let mut out = format!("Current model: {}\n\nProviders:\n", app.current_model);
    for entry in entries {
        let preview = entry.models.join(", ");
        let suffix = if entry.total_models > entry.models.len() {
            format!(" (+{} more)", entry.total_models - entry.models.len())
        } else {
            String::new()
        };
        let _ = writeln!(out, "  - {:<14} {}{}", entry.provider, preview, suffix);
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_runbook_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let action = args.first().copied().unwrap_or("list").to_ascii_lowercase();
    if action == "list" || action == "status" {
        emit_command_output(
            app,
            "Runbooks\n- auth-refresh: provider auth/session rejected\n- model-not-found: catalog drift / unknown model\n- contextlattice-connect: local memory integration bootstrap\n- tool-policy-deny: blocked by policy or sandbox profile\n- stream-finalization: stream done but transcript not finalized\n\nUse `/runbook show <name>`.",
        );
        return Ok(CommandResult::Handled);
    }
    if action == "show" {
        let Some(name) = args.get(1).map(|v| v.to_ascii_lowercase()) else {
            emit_command_output(app, "Usage: /runbook show <name>");
            return Ok(CommandResult::Handled);
        };
        let body = match name.as_str() {
            "auth-refresh" => {
                "Runbook: auth-refresh\n1) `/auth status`\n2) `/auth refresh`\n3) retry prompt\n4) if still failing, run `/model` and confirm provider/model pair is valid for your account."
            }
            "model-not-found" => {
                "Runbook: model-not-found\n1) `/model` and select a valid catalog model\n2) retry request\n3) if provider alias was stale, run `/auth verify` and re-check."
            }
            "contextlattice-connect" => {
                "Runbook: contextlattice-connect\n1) ensure contextlattice tools are registered via `/tools`\n2) ask agent to run `contextlattice_search` first (not shell command `contextlattice`)\n3) checkpoint verified integration via `contextlattice_write`."
            }
            "tool-policy-deny" => {
                "Runbook: tool-policy-deny\n1) inspect denial reason in tool card `[remediation]` section\n2) remove secret-like args from inline command payload\n3) retry with safer params or approved tool route (`/tools`)."
            }
            "stream-finalization" => {
                "Runbook: stream-finalization\n1) wait for final transcript writeback (status shows `Finalizing response…`)\n2) avoid submitting a new prompt until finalization completes\n3) if UI appears stale, use Ctrl+G to refresh and jump latest."
            }
            _ => {
                emit_command_output(
                    app,
                    format!(
                        "Unknown runbook `{}`. Use `/runbook list` for available entries.",
                        name
                    ),
                );
                return Ok(CommandResult::Handled);
            }
        };
        emit_command_output(app, body);
        return Ok(CommandResult::Handled);
    }
    emit_command_output(app, "Usage: /runbook [list|show <name>]");
    Ok(CommandResult::Handled)
}

fn provider_health_snapshot(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "nous" | "google-gemini-cli" | "gemini-cli" | "gemini-oauth" | "qwen-oauth" => {
            "oauth-capable"
        }
        "openai" | "anthropic" | "openrouter" => "api-key/session",
        _ => "unknown",
    }
}

fn detect_repo_root_from_cwd() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    for candidate in cwd.ancestors() {
        if candidate.join(".git").exists() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

fn handle_profile_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let home = hermes_config::hermes_home();
    let selected = app.config.profile.current.as_deref().unwrap_or("default");
    let mut out = String::new();
    let _ = writeln!(out, "Active profile: {}", selected);
    let _ = writeln!(out, "Hermes home: {}", home.display());
    let _ = writeln!(out, "Session id: {}", app.session_id);
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_runtime_ui_mode_command(
    app: &mut App,
    cmd: &str,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let msg = match cmd {
        "/skin" => {
            let first = args.first().copied().unwrap_or("status");
            if first.eq_ignore_ascii_case("list") {
                let active = std::env::var("HERMES_THEME").unwrap_or_else(|_| "ultra-neon".to_string());
                let active_canonical = canonical_skin_name(&active).unwrap_or("ultra-neon");
                let mut out = String::new();
                let _ = writeln!(out, "Built-in skins (active: {}):", active_canonical);
                for (name, detail) in BUILTIN_SKINS {
                    let marker = if *name == active_canonical { "✓" } else { " " };
                    let _ = writeln!(out, "  {} {:<30} {}", marker, name, detail);
                }
                let _ = writeln!(
                    out,
                    "\nUse `/skin <name>` or `/skin set <name>` to switch immediately."
                );
                out.trim_end().to_string()
            } else if first.eq_ignore_ascii_case("status") || first.eq_ignore_ascii_case("show") {
                let active = std::env::var("HERMES_THEME").unwrap_or_else(|_| "ultra-neon".to_string());
                let active_canonical = canonical_skin_name(&active).unwrap_or("ultra-neon");
                format!(
                    "Current skin: {}\nUse `/skin list` to browse options.\nUse `/skin <name>` to switch now.",
                    active_canonical
                )
            } else {
                let requested = if first.eq_ignore_ascii_case("set") {
                    args.get(1).copied().unwrap_or("")
                } else {
                    first
                };
                if requested.trim().is_empty() {
                    "Usage: `/skin list` or `/skin <name>`".to_string()
                } else if let Some(canonical) = canonical_skin_name(requested) {
                    crate::env_vars::set_var("HERMES_THEME", canonical);
                    app.request_theme_change(canonical);
                    format!(
                        "Skin switched to `{}`.\nApplied in this TUI session and exported as HERMES_THEME for child processes.",
                        canonical
                    )
                } else {
                    format!(
                        "Unknown skin `{}`. Use `/skin list` for built-ins.",
                        requested
                    )
                }
            }
        }
        "/fast" => format!(
            "Fast mode compatibility command received (`{}`).\nCurrent model: {}\nTip: switch to a lower-latency model via `/model`.",
            args.first().copied().unwrap_or("status"),
            app.current_model
        ),
        "/voice" => "Voice mode uses provider/platform capabilities; no separate TUI voice engine is active in this session.".to_string(),
        _ => "Unsupported runtime UI mode command.".to_string(),
    };
    emit_command_output(app, msg);
    Ok(CommandResult::Handled)
}

fn render_pet_status(settings: &PetSettings) -> String {
    format!(
        "Pet status:\n  - enabled: {}\n  - species: {}\n  - mood: {}\n  - dock: {}\n  - speed_ms: {}\n\nUse `/pet on`, `/pet off`, `/pet toggle`, `/pet set <species>`, `/pet mood <mood>`, `/pet dock <left|right>`, `/pet speed <ms>`, `/pet list`.",
        if settings.enabled { "ON" } else { "OFF" },
        settings.species,
        settings.mood,
        settings.dock.as_str(),
        settings.tick_ms
    )
}

fn parse_pet_species(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    PetSettings::species_catalog()
        .iter()
        .find(|candidate| **candidate == normalized)
        .map(|candidate| (*candidate).to_string())
}

fn parse_pet_mood(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    PetSettings::mood_catalog()
        .iter()
        .find(|candidate| **candidate == normalized)
        .map(|candidate| (*candidate).to_string())
}

fn parse_pet_dock(value: &str) -> Option<PetDock> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "left" => Some(PetDock::Left),
        "right" => Some(PetDock::Right),
        _ => None,
    }
}

fn handle_pet_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let action = args.first().copied().unwrap_or("status");
    let mut settings = app.pet_settings().clone();

    match action.to_ascii_lowercase().as_str() {
        "status" => {
            emit_command_output(app, render_pet_status(&settings));
        }
        "list" => {
            emit_command_output(
                app,
                format!(
                    "Available pets:\n  - species: {}\n  - moods: {}\n  - dock: left, right",
                    PetSettings::species_catalog().join(", "),
                    PetSettings::mood_catalog().join(", ")
                ),
            );
        }
        "on" | "off" | "toggle" | "wake" | "sleep" | "tuck" => {
            let action_lc = action.to_ascii_lowercase();
            let normalized_toggle = match action_lc.as_str() {
                "wake" => Some("on"),
                "sleep" | "tuck" => Some("off"),
                other => Some(other),
            };
            match parse_toggle_arg(normalized_toggle, settings.enabled) {
                Ok(enabled) => {
                    settings.enabled = enabled;
                    app.set_pet_settings(settings.clone())?;
                    emit_command_output(
                        app,
                        format!(
                            "Pet {}.\n{}",
                            if settings.enabled {
                                "enabled"
                            } else {
                                "hidden"
                            },
                            render_pet_status(&settings)
                        ),
                    );
                }
                Err(_) => emit_command_output(
                    app,
                    "Usage: /pet [status|on|off|toggle|wake|tuck|list|set <species>|mood <mood>|dock <left|right>|speed <ms>]",
                ),
            }
        }
        "set" | "species" => {
            let Some(raw) = args.get(1).copied() else {
                emit_command_output(
                    app,
                    format!(
                        "Usage: /pet set <species>\nAvailable species: {}",
                        PetSettings::species_catalog().join(", ")
                    ),
                );
                return Ok(CommandResult::Handled);
            };
            if let Some(species) = parse_pet_species(raw) {
                settings.species = species;
                app.set_pet_settings(settings.clone())?;
                emit_command_output(app, render_pet_status(&settings));
            } else {
                emit_command_output(
                    app,
                    format!(
                        "Unknown species '{}'. Available: {}",
                        raw,
                        PetSettings::species_catalog().join(", ")
                    ),
                );
            }
        }
        "mood" => {
            let Some(raw) = args.get(1).copied() else {
                emit_command_output(
                    app,
                    format!(
                        "Usage: /pet mood <mood>\nAvailable moods: {}",
                        PetSettings::mood_catalog().join(", ")
                    ),
                );
                return Ok(CommandResult::Handled);
            };
            if let Some(mood) = parse_pet_mood(raw) {
                settings.mood = mood;
                app.set_pet_settings(settings.clone())?;
                emit_command_output(app, render_pet_status(&settings));
            } else {
                emit_command_output(
                    app,
                    format!(
                        "Unknown mood '{}'. Available: {}",
                        raw,
                        PetSettings::mood_catalog().join(", ")
                    ),
                );
            }
        }
        "dock" => {
            let Some(raw) = args.get(1).copied() else {
                emit_command_output(app, "Usage: /pet dock <left|right>");
                return Ok(CommandResult::Handled);
            };
            if let Some(dock) = parse_pet_dock(raw) {
                settings.dock = dock;
                app.set_pet_settings(settings.clone())?;
                emit_command_output(app, render_pet_status(&settings));
            } else {
                emit_command_output(app, "Usage: /pet dock <left|right>");
            }
        }
        "speed" => {
            let Some(raw) = args.get(1).copied() else {
                emit_command_output(app, "Usage: /pet speed <ms>");
                return Ok(CommandResult::Handled);
            };
            match raw.trim().parse::<u64>() {
                Ok(ms) => {
                    settings.tick_ms = ms;
                    app.set_pet_settings(settings.clone())?;
                    emit_command_output(app, render_pet_status(&settings));
                }
                Err(_) => emit_command_output(app, "Usage: /pet speed <ms>"),
            }
        }
        _ => emit_command_output(
            app,
            "Usage: /pet [status|on|off|toggle|wake|tuck|list|set <species>|mood <mood>|dock <left|right>|speed <ms>]",
        ),
    }

    Ok(CommandResult::Handled)
}

fn handle_toolsets_command(app: &mut App) -> Result<CommandResult, AgentError> {
    if app.config.platform_toolsets.is_empty() {
        emit_command_output(app, "No explicit platform toolsets configured.");
        return Ok(CommandResult::Handled);
    }
    let mut rows: Vec<_> = app.config.platform_toolsets.iter().collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::from("Configured toolsets by platform:\n");
    for (platform, toolsets) in rows {
        let _ = writeln!(out, "  - {:<10} {}", platform, toolsets.join(", "));
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_plugins_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let rows = discover_plugin_surface(true);
    if rows.is_empty() {
        let plugins_dir = hermes_config::hermes_home().join("plugins");
        emit_command_output(
            app,
            format!(
                "No plugin bundles discovered.\nUser plugin dir: {}\nInstall with `hermes plugins install <owner/repo>`.",
                plugins_dir.display()
            ),
        );
    } else {
        emit_command_output(
            app,
            format!(
                "Plugin surface ({} entries):\n{}",
                rows.len(),
                render_plugin_surface_table(&rows)
            ),
        );
    }
    Ok(CommandResult::Handled)
}

fn handle_mcp_command(app: &mut App) -> Result<CommandResult, AgentError> {
    if app.config.mcp_servers.is_empty() {
        emit_command_output(app, "No MCP servers configured in `config.yaml`.");
        return Ok(CommandResult::Handled);
    }
    let mut out = String::from("Configured MCP servers:\n");
    for server in &app.config.mcp_servers {
        let endpoint = server
            .url
            .as_deref()
            .filter(|u| !u.is_empty())
            .unwrap_or("<stdio>");
        let _ = writeln!(
            out,
            "  - {:<18} {}  [parallel_tool_calls:{}]",
            server.name,
            endpoint,
            if server.supports_parallel_tool_calls {
                "on"
            } else {
                "off"
            }
        );
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_reload_command(app: &mut App, cmd: &str) -> Result<CommandResult, AgentError> {
    if cmd == "/reload-mcp" {
        emit_command_output(
            app,
            "MCP reload requested. Restart session/gateway for full connector renegotiation.",
        );
    } else {
        hermes_config::loader::load_dotenv();
        match hermes_config::load_config(app.state_root.to_str()) {
            Ok(cfg) => {
                app.config = Arc::new(cfg);
                emit_command_output(
                    app,
                    "Reload complete: env + config rehydrated for this session.",
                );
            }
            Err(err) => {
                emit_command_output(
                    app,
                    format!(
                        "Reload partially applied (.env refreshed), but config parse failed: {}",
                        err
                    ),
                );
            }
        }
    }
    Ok(CommandResult::Handled)
}

fn handle_cron_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let cron_data = hermes_config::cron_dir();
    let jobs_file = cron_data.join("jobs.json");
    let count = std::fs::read_to_string(&jobs_file)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("jobs")
                .and_then(|j| j.as_array())
                .map(|arr| arr.len())
                .or_else(|| v.as_array().map(|arr| arr.len()))
        })
        .unwrap_or_else(|| {
            std::fs::read_dir(&cron_data)
                .ok()
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| {
                            e.path().extension().and_then(|x| x.to_str()) == Some("json")
                                && e.file_name().to_string_lossy() != "jobs.json"
                        })
                        .count()
                })
                .unwrap_or(0)
        });
    emit_command_output(
        app,
        format!(
            "Cron scheduler data dir: {}\nPersisted jobs: {}\nUse `hermes cron list` for full job table.",
            cron_data.display(),
            count
        ),
    );
    Ok(CommandResult::Handled)
}

fn background_status_rows() -> Vec<String> {
    let jobs_dir = hermes_config::hermes_home().join("background_jobs");
    let mut rows = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(&jobs_dir) else {
        return rows;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("unknown");
        let status = v
            .get("status")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown");
        let task = v
            .get("task")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .replace('\n', " ");
        rows.push(format!("{id}  [{status}]  {task}"));
    }
    rows.sort();
    rows
}

fn env_truthy(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn handle_agents_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let sub = args.first().map(|s| s.trim().to_ascii_lowercase());

    if matches!(sub.as_deref(), Some("pause")) {
        crate::env_vars::set_var("HERMES_DELEGATION_PAUSED", "1");
        emit_command_output(
            app,
            "Delegation spawning paused for this runtime.\nSet with `/agents resume`.\nStatus: `/agents status`.",
        );
        return Ok(CommandResult::Handled);
    }
    if matches!(sub.as_deref(), Some("resume" | "unpause")) {
        crate::env_vars::set_var("HERMES_DELEGATION_PAUSED", "0");
        emit_command_output(
            app,
            "Delegation spawning resumed for this runtime.\nStatus: `/agents status`.",
        );
        return Ok(CommandResult::Handled);
    }
    if matches!(sub.as_deref(), Some("doctor")) {
        emit_command_output(
            app,
            "Agents doctor\n- queue manifest audit: `python3 scripts/audit_background_queue.py`\n- optional repair: `python3 scripts/audit_background_queue.py --repair`\n- delegation state: `/agents status`\n- spawn tree UI: `/agents` (TUI overlay)",
        );
        return Ok(CommandResult::Handled);
    }

    if matches!(sub.as_deref(), Some(other) if other != "status" && other != "list") {
        emit_command_output(app, "Usage: /agents [status|pause|resume|doctor]");
        return Ok(CommandResult::Handled);
    }

    let paused = std::env::var("HERMES_DELEGATION_PAUSED")
        .ok()
        .map(|raw| env_truthy(&raw))
        .unwrap_or(false);
    let rows = background_status_rows();
    if rows.is_empty() {
        emit_command_output(
            app,
            format!(
                "Delegation spawning: {}\nBackground jobs: 0\n\nNo background jobs found.\nAudit/repair queue manifests with `python3 scripts/audit_background_queue.py [--repair]`.",
                if paused { "paused" } else { "active" }
            ),
        );
    } else {
        let joined = rows.into_iter().take(20).collect::<Vec<_>>().join("\n");
        emit_command_output(
            app,
            format!(
                "Delegation spawning: {}\nBackground jobs (top 20):\n{}\n\nQueue audit: `python3 scripts/audit_background_queue.py`\nPause/resume: `/agents pause` or `/agents resume`",
                if paused { "paused" } else { "active" },
                joined,
            ),
        );
    }
    Ok(CommandResult::Handled)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanCapabilityMode {
    Off,
    Advisory,
    Enforce,
}

impl PlanCapabilityMode {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "disable" | "disabled" | "0" => Some(Self::Off),
            "advisory" | "warn" | "on" | "1" => Some(Self::Advisory),
            "enforce" | "strict" => Some(Self::Enforce),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Advisory => "advisory",
            Self::Enforce => "enforce",
        }
    }
}

fn plan_capability_mode() -> PlanCapabilityMode {
    std::env::var("HERMES_PLAN_CAPABILITY_ROUTER")
        .ok()
        .as_deref()
        .and_then(PlanCapabilityMode::parse)
        .unwrap_or(PlanCapabilityMode::Off)
}

fn infer_plan_requirements(task: &str) -> ModelCapabilityRequirements {
    let lower = task.to_ascii_lowercase();
    let mut req = ModelCapabilityRequirements::default();

    if [
        "repo",
        "code",
        "patch",
        "implement",
        "fix",
        "test",
        "lint",
        "build",
        "deploy",
        "file",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        req.require_tools = true;
    }
    if [
        "audit",
        "parity",
        "objective",
        "investigate",
        "diagnose",
        "analysis",
        "architecture",
        "production",
        "security",
        "trading",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        req.require_reasoning = true;
    }
    if [
        "full repo",
        "entire repo",
        "all files",
        "large codebase",
        "multi-repo",
        "end to end",
        "end-to-end",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        req.require_long_context = true;
    }
    if ["image", "screenshot", "diagram", "figma"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        req.require_vision = true;
    }

    req
}

fn plan_capability_preflight(app: &App, task: &str) -> (Option<String>, bool) {
    let mode = plan_capability_mode();
    if matches!(mode, PlanCapabilityMode::Off) {
        return (None, true);
    }

    let req = infer_plan_requirements(task);
    if req.is_empty() {
        return (None, true);
    }

    let (provider, model_id) = split_provider_model(&app.current_model);
    let client = default_client();
    let caps = resolve_model_capabilities(provider, model_id, client);
    let unmet = unmet_model_requirements(caps, req);
    if unmet.is_empty() {
        return (
            Some(format!(
                "planner capability preflight: PASS ({}) for `{}`",
                req.summary(),
                app.current_model
            )),
            true,
        );
    }

    let explain_hint = format!(
        "/model explain {} --cap tools,reasoning --min-context 128000",
        app.current_model
    );
    let message = format!(
        "planner capability preflight: {} ({}) for `{}`.\nmissing: {}\nhint: run `{}` or switch with `/model` before queuing this task.",
        if matches!(mode, PlanCapabilityMode::Enforce) {
            "BLOCKED"
        } else {
            "WARN"
        },
        req.summary(),
        app.current_model,
        unmet.join(", "),
        explain_hint
    );

    let allowed = !matches!(mode, PlanCapabilityMode::Enforce);
    (Some(message), allowed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskDepthProfile {
    Shallow,
    Balanced,
    Deep,
    Max,
}

impl TaskDepthProfile {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "shallow" | "fast" => Some(Self::Shallow),
            "balanced" | "default" => Some(Self::Balanced),
            "deep" | "thorough" => Some(Self::Deep),
            "max" | "exhaustive" => Some(Self::Max),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Shallow => "shallow",
            Self::Balanced => "balanced",
            Self::Deep => "deep",
            Self::Max => "max",
        }
    }
}

fn set_env_var_u64(key: &str, value: u64) {
    crate::env_vars::set_var(key, value.to_string());
}

fn set_env_var_f64(key: &str, value: f64) {
    crate::env_vars::set_var(key, format!("{value:.2}"));
}

fn apply_task_depth_profile(profile: TaskDepthProfile) {
    crate::env_vars::set_var("HERMES_TASK_DEPTH_PROFILE", profile.as_str());
    match profile {
        TaskDepthProfile::Shallow => {
            set_env_var_u64("HERMES_MAX_ITERATIONS", 18);
            set_env_var_u64("HERMES_TOOL_CALL_MAX_CONCURRENCY", 10);
            set_env_var_u64("HERMES_MAX_DELEGATE_DEPTH", 1);
            set_env_var_u64("HERMES_PERF_GOV_WINDOW", 6);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_WARN_MS", 2800.0);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_CRITICAL_MS", 5200.0);
            crate::env_vars::set_var("HERMES_REPO_REVIEW_BUDGET_PROFILE", "aggressive");
        }
        TaskDepthProfile::Balanced => {
            set_env_var_u64("HERMES_MAX_ITERATIONS", 250);
            set_env_var_u64("HERMES_TOOL_CALL_MAX_CONCURRENCY", 12);
            set_env_var_u64("HERMES_MAX_DELEGATE_DEPTH", 4);
            set_env_var_u64("HERMES_PERF_GOV_WINDOW", 8);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_WARN_MS", 3500.0);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_CRITICAL_MS", 6500.0);
            crate::env_vars::set_var("HERMES_REPO_REVIEW_BUDGET_PROFILE", "off");
        }
        TaskDepthProfile::Deep => {
            set_env_var_u64("HERMES_MAX_ITERATIONS", 120);
            set_env_var_u64("HERMES_TOOL_CALL_MAX_CONCURRENCY", 6);
            set_env_var_u64("HERMES_MAX_DELEGATE_DEPTH", 3);
            set_env_var_u64("HERMES_PERF_GOV_WINDOW", 10);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_WARN_MS", 4800.0);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_CRITICAL_MS", 9000.0);
            crate::env_vars::set_var("HERMES_REPO_REVIEW_BUDGET_PROFILE", "relaxed");
        }
        TaskDepthProfile::Max => {
            set_env_var_u64("HERMES_MAX_ITERATIONS", 250);
            set_env_var_u64("HERMES_TOOL_CALL_MAX_CONCURRENCY", 5);
            set_env_var_u64("HERMES_MAX_DELEGATE_DEPTH", 4);
            set_env_var_u64("HERMES_PERF_GOV_WINDOW", 12);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_WARN_MS", 6500.0);
            set_env_var_f64("HERMES_PERF_GOV_LATENCY_CRITICAL_MS", 12000.0);
            crate::env_vars::set_var("HERMES_REPO_REVIEW_BUDGET_PROFILE", "off");
        }
    }
}

fn current_task_depth_profile() -> TaskDepthProfile {
    std::env::var("HERMES_TASK_DEPTH_PROFILE")
        .ok()
        .as_deref()
        .and_then(TaskDepthProfile::parse)
        .unwrap_or(TaskDepthProfile::Balanced)
}

fn task_depth_runtime_summary() -> String {
    let profile = current_task_depth_profile();
    let max_iters = std::env::var("HERMES_MAX_ITERATIONS").unwrap_or_else(|_| "250".to_string());
    let tool_concurrency =
        std::env::var("HERMES_TOOL_CALL_MAX_CONCURRENCY").unwrap_or_else(|_| "12".to_string());
    let delegate_depth =
        std::env::var("HERMES_MAX_DELEGATE_DEPTH").unwrap_or_else(|_| "4".to_string());
    let repo_budget =
        std::env::var("HERMES_REPO_REVIEW_BUDGET_PROFILE").unwrap_or_else(|_| "off".to_string());
    format!(
        "task_depth profile={} max_iterations={} tool_concurrency={} max_delegate_depth={} repo_budget_profile={}",
        profile.as_str(),
        max_iters,
        tool_concurrency,
        delegate_depth,
        repo_budget
    )
}

fn handle_plan_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty()
        || args
            .first()
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "help" | "usage"))
    {
        emit_command_output(
            app,
            "Planner controls:\n  /plan <task>          Queue a planning/research task in background\n  /plan status          Show queue health + active steering\n  /plan list            Show queue health + active steering\n  /plan clear           Clear queued/running status records\n  /plan caps [mode]     Optional capability router (`off|advisory|enforce`)\n  /plan depth [profile] Task-depth governor (`status|list|shallow|balanced|deep|max|clear`)",
        );
        return Ok(CommandResult::Handled);
    }

    let sub = args[0].to_ascii_lowercase();
    if sub == "caps" || sub == "capability" || sub == "capabilities" {
        let next = args
            .get(1)
            .copied()
            .unwrap_or("status")
            .to_ascii_lowercase();
        match next.as_str() {
            "status" | "show" => {
                emit_command_output(
                    app,
                    format!(
                        "planner capability router mode={}\nUse `/plan caps [off|advisory|enforce]`.",
                        plan_capability_mode().as_str()
                    ),
                );
            }
            "off" | "advisory" | "enforce" => {
                if let Some(mode) = PlanCapabilityMode::parse(&next) {
                    crate::env_vars::set_var("HERMES_PLAN_CAPABILITY_ROUTER", mode.as_str());
                    emit_command_output(
                        app,
                        format!("planner capability router set to `{}`.", mode.as_str()),
                    );
                }
            }
            _ => emit_command_output(app, "Usage: /plan caps [status|off|advisory|enforce]"),
        }
        return Ok(CommandResult::Handled);
    }
    if sub == "depth" {
        let next = args
            .get(1)
            .copied()
            .unwrap_or("status")
            .to_ascii_lowercase();
        match next.as_str() {
            "status" | "show" => emit_command_output(app, task_depth_runtime_summary()),
            "list" => emit_command_output(
                app,
                "Task depth profiles:\n- shallow: quickest turn cadence; strict exploration trim\n- balanced: default profile for most sessions\n- deep: larger turn budget + lower concurrency for heavier analysis\n- max: exhaustive mode for very complex objective work\nUse `/plan depth <profile>` to apply.",
            ),
            "clear" => {
                crate::env_vars::remove_var("HERMES_TASK_DEPTH_PROFILE");
                for key in [
                    "HERMES_MAX_ITERATIONS",
                    "HERMES_TOOL_CALL_MAX_CONCURRENCY",
                    "HERMES_MAX_DELEGATE_DEPTH",
                    "HERMES_PERF_GOV_WINDOW",
                    "HERMES_PERF_GOV_LATENCY_WARN_MS",
                    "HERMES_PERF_GOV_LATENCY_CRITICAL_MS",
                    "HERMES_REPO_REVIEW_BUDGET_PROFILE",
                ] {
                    crate::env_vars::remove_var(key);
                }
                apply_task_depth_profile(TaskDepthProfile::Balanced);
                emit_command_output(
                    app,
                    format!(
                        "Task depth reset to defaults.\n{}",
                        task_depth_runtime_summary()
                    ),
                );
            }
            _ => {
                let Some(profile) = TaskDepthProfile::parse(&next) else {
                    emit_command_output(
                        app,
                        "Usage: /plan depth [status|list|shallow|balanced|deep|max|clear]",
                    );
                    return Ok(CommandResult::Handled);
                };
                apply_task_depth_profile(profile);
                emit_command_output(
                    app,
                    format!(
                        "Task depth profile set to `{}`.\n{}",
                        profile.as_str(),
                        task_depth_runtime_summary()
                    ),
                );
            }
        }
        return Ok(CommandResult::Handled);
    }
    if sub == "status" || sub == "list" {
        let (queued, running, completed, failed) = background::background_job_counts();
        let mut out = String::new();
        let _ = writeln!(out, "Planner queue status");
        let _ = writeln!(
            out,
            "  queued={} running={} completed={} failed={}",
            queued, running, completed, failed
        );
        if let Some(steer) = objective::current_session_steer(app) {
            let _ = writeln!(out, "  steering={}", truncate_chars(&steer, 160));
        } else {
            let _ = writeln!(out, "  steering=(none)");
        }
        if let Some(objective) = app.session_objective.as_deref() {
            let _ = writeln!(out, "  objective={}", truncate_chars(objective, 160));
        } else {
            let _ = writeln!(out, "  objective=(none)");
        }
        let _ = writeln!(
            out,
            "  capability_router={}",
            plan_capability_mode().as_str()
        );
        let _ = writeln!(out, "  {}", task_depth_runtime_summary());
        emit_command_output(app, out.trim_end());
        return Ok(CommandResult::Handled);
    }
    if sub == "clear" {
        return background::handle_clear_queue_command(app);
    }
    let task = args.join(" ");
    if !task.trim().is_empty() {
        let (note, allowed) = plan_capability_preflight(app, &task);
        if let Some(msg) = note {
            emit_command_output(app, msg);
        }
        if !allowed {
            return Ok(CommandResult::Handled);
        }
    }
    background::handle_background_command(app, args)
}

fn handle_lsp_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "status".to_string());
    match sub.as_str() {
        "status" | "show" => {
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "<unavailable>".to_string());
            let mut out = String::new();
            let _ = writeln!(out, "LSP/code-index status");
            let _ = writeln!(out, "  cwd: {}", cwd);
            let _ = writeln!(
                out,
                "  code_index_enabled: {}",
                yes_no(app.config.agent.code_index_enabled)
            );
            let _ = writeln!(
                out,
                "  code_index_max_files: {}",
                app.config.agent.code_index_max_files
            );
            let _ = writeln!(
                out,
                "  code_index_max_symbols: {}",
                app.config.agent.code_index_max_symbols
            );
            let _ = writeln!(
                out,
                "  lsp_context_enabled: {}",
                yes_no(app.config.agent.lsp_context_enabled)
            );
            let _ = writeln!(
                out,
                "  lsp_context_max_chars: {}",
                app.config.agent.lsp_context_max_chars
            );
            let _ = writeln!(
                out,
                "  tip: run `/plan map the repo architecture` to force a high-signal repo-map pass."
            );
            emit_command_output(app, out.trim_end());
        }
        "refresh" => {
            emit_command_output(
                app,
                "Code index refresh is automatic while the agent executes tool calls. Queue a focused analysis with `/plan <task>` if you want a deliberate repo-map rebuild now.",
            );
        }
        "help" => {
            emit_command_output(
                app,
                "Usage: /lsp [status|refresh]\n  status   show code-index + LSP context configuration\n  refresh  explain how to trigger a fresh index pass",
            );
        }
        _ => emit_command_output(app, "Usage: /lsp [status|refresh]"),
    }
    Ok(CommandResult::Handled)
}

fn collect_graph_candidate_files(
    root: &Path,
    max_files: usize,
    out: &mut Vec<PathBuf>,
) -> Result<(), AgentError> {
    if out.len() >= max_files {
        return Ok(());
    }
    let rd = std::fs::read_dir(root)
        .map_err(|e| AgentError::Io(format!("read_dir {}: {}", root.display(), e)))?;
    for entry in rd {
        if out.len() >= max_files {
            break;
        }
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            if matches!(
                name,
                ".git"
                    | "target"
                    | "node_modules"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | ".mypy_cache"
                    | ".pytest_cache"
            ) {
                continue;
            }
            collect_graph_candidate_files(&path, max_files, out)?;
            continue;
        }
        let ext = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "rs" | "py" | "ts" | "tsx" | "js" | "jsx") {
            out.push(path);
        }
    }
    Ok(())
}

fn extract_semantic_refs_for_file(ext: &str, content: &str) -> Vec<String> {
    let mut refs = Vec::new();
    match ext {
        "rs" => {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("use ") {
                    let target = rest.split(';').next().unwrap_or_default().trim();
                    if !target.is_empty() {
                        refs.push(target.to_string());
                    }
                }
                if let Some(rest) = trimmed.strip_prefix("mod ") {
                    let target = rest.split(';').next().unwrap_or_default().trim();
                    if !target.is_empty() {
                        refs.push(target.to_string());
                    }
                }
            }
        }
        "py" => {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    for item in rest.split(',') {
                        let target = item.split_whitespace().next().unwrap_or_default().trim();
                        if !target.is_empty() {
                            refs.push(target.to_string());
                        }
                    }
                } else if let Some(rest) = trimmed.strip_prefix("from ") {
                    let target = rest.split_whitespace().next().unwrap_or_default().trim();
                    if !target.is_empty() {
                        refs.push(target.to_string());
                    }
                }
            }
        }
        "ts" | "tsx" | "js" | "jsx" => {
            let re = Regex::new(r#"(?m)from\s+["']([^"']+)["']"#).expect("valid import regex");
            for caps in re.captures_iter(content) {
                if let Some(m) = caps.get(1) {
                    refs.push(m.as_str().trim().to_string());
                }
            }
            let re_req = Regex::new(r#"(?m)require\(\s*["']([^"']+)["']\s*\)"#)
                .expect("valid require regex");
            for caps in re_req.captures_iter(content) {
                if let Some(m) = caps.get(1) {
                    refs.push(m.as_str().trim().to_string());
                }
            }
        }
        _ => {}
    }
    refs
}

fn sanitize_graph_node(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn contextlattice_base_url_for_graph() -> String {
    std::env::var("CONTEXTLATTICE_ORCHESTRATOR_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var("MEMMCP_ORCHESTRATOR_URL").ok())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8075".to_string())
}

fn contextlattice_api_key_for_graph() -> Option<String> {
    std::env::var("CONTEXTLATTICE_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var("MEMMCP_API_KEY").ok())
        .filter(|v| !v.trim().is_empty())
}

fn extract_json_path<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut cur = value;
    for key in path {
        cur = cur.get(*key)?;
    }
    Some(cur)
}

fn extract_embedding_diag_line(payload: &serde_json::Value) -> String {
    let backend = [
        &["backend"][..],
        &["embedding_backend"][..],
        &["embeddings", "backend"][..],
        &["retrieval", "embedding_backend"][..],
    ]
    .into_iter()
    .find_map(|path| extract_json_path(payload, path))
    .and_then(|v| v.as_str())
    .unwrap_or("unknown");
    let dimension = [
        &["dimension"][..],
        &["embeddings", "dimension"][..],
        &["retrieval", "embedding_dimension"][..],
    ]
    .into_iter()
    .find_map(|path| extract_json_path(payload, path))
    .and_then(|v| v.as_u64())
    .map(|v| v.to_string())
    .unwrap_or_else(|| "n/a".to_string());
    let model = [
        &["model"][..],
        &["embeddings", "model"][..],
        &["retrieval", "embedding_model"][..],
    ]
    .into_iter()
    .find_map(|path| extract_json_path(payload, path))
    .and_then(|v| v.as_str())
    .unwrap_or("unknown");
    format!(
        "embedding_diagnostics: backend={} model={} dimension={}",
        backend, model, dimension
    )
}

async fn contextlattice_embedding_diagnostics_lines() -> Vec<String> {
    let base_url = contextlattice_base_url_for_graph();
    let mut lines = Vec::new();
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            lines.push(format!("client_error: {}", err));
            return lines;
        }
    };

    let mut health_req = client.get(format!("{}/health", base_url.trim_end_matches('/')));
    if let Some(key) = contextlattice_api_key_for_graph() {
        health_req = health_req.header("x-api-key", key);
    }
    match health_req.send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            lines.push(format!("health_status: {}", code));
        }
        Err(err) => {
            lines.push(format!("health_status: unreachable ({})", err));
        }
    }

    let mut emb_req = client.get(format!(
        "{}/telemetry/embeddings",
        base_url.trim_end_matches('/')
    ));
    if let Some(key) = contextlattice_api_key_for_graph() {
        emb_req = emb_req.header("x-api-key", key);
    }
    match emb_req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                match resp.json::<serde_json::Value>().await {
                    Ok(payload) => lines.push(extract_embedding_diag_line(&payload)),
                    Err(err) => {
                        lines.push(format!("embedding_diagnostics: invalid_json ({})", err))
                    }
                }
            } else {
                lines.push(format!(
                    "embedding_diagnostics: unavailable (telemetry/embeddings status={})",
                    status.as_u16()
                ));
                lines.push("embedding_diagnostics: fallback=recall_telemetry".to_string());
            }
        }
        Err(err) => {
            lines.push(format!(
                "embedding_diagnostics: unavailable (unreachable: {})",
                err
            ));
            lines.push("embedding_diagnostics: fallback=recall_telemetry".to_string());
        }
    }

    let mut recall_req = client.get(format!(
        "{}/telemetry/recall",
        base_url.trim_end_matches('/')
    ));
    if let Some(key) = contextlattice_api_key_for_graph() {
        recall_req = recall_req.header("x-api-key", key);
    }
    match recall_req.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(payload) => {
                let qps = payload
                    .get("query_per_sec")
                    .or_else(|| payload.get("qps"))
                    .and_then(|v| v.as_f64())
                    .map(|v| format!("{:.3}", v))
                    .unwrap_or_else(|| "n/a".to_string());
                let hit_rate = payload
                    .get("hit_rate")
                    .or_else(|| payload.get("grounded_hit_rate"))
                    .and_then(|v| v.as_f64())
                    .map(|v| format!("{:.3}", v))
                    .unwrap_or_else(|| "n/a".to_string());
                lines.push(format!(
                    "recall_telemetry: qps={} hit_rate={}",
                    qps, hit_rate
                ));
            }
            Err(err) => lines.push(format!("recall_telemetry: invalid_json ({})", err)),
        },
        Ok(resp) => lines.push(format!(
            "recall_telemetry: endpoint_status={}",
            resp.status()
        )),
        Err(err) => lines.push(format!("recall_telemetry: unreachable ({})", err)),
    }

    lines
}

async fn handle_graph_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "status".to_string());
    match sub.as_str() {
        "status" | "show" => {
            let contextlattice_mcp = app.config.mcp_servers.iter().any(|entry| {
                let name = entry.name.to_ascii_lowercase();
                let url = entry
                    .url
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                name.contains("contextlattice") || url.contains("contextlattice")
            });
            let policy = load_contextlattice_policy().ok();
            let mut out = String::new();
            let _ = writeln!(out, "Graph-memory status");
            let _ = writeln!(out, "  contextlattice_mcp: {}", yes_no(contextlattice_mcp));
            let diag = contextlattice_embedding_diagnostics_lines().await;
            for row in &diag {
                let _ = writeln!(out, "  {}", row);
            }
            if let Some(policy) = policy {
                let _ = writeln!(
                    out,
                    "  retrieval_mode_hint: {}",
                    policy.preferred_retrieval_mode
                );
                let _ = writeln!(out, "  preflight_required: {}", policy.preflight_required);
                let _ = writeln!(
                    out,
                    "  include_grounding_required: {}",
                    policy.include_grounding_required
                );
                let _ = writeln!(
                    out,
                    "  degradation_aware_planning: {}",
                    policy.degradation_aware_planning
                );
            } else {
                let _ = writeln!(out, "  contextlattice_policy: unavailable");
            }
            emit_command_output(app, out.trim_end());
        }
        "embeddings" | "embedding" | "diag" => {
            let mut out = String::new();
            let _ = writeln!(out, "ContextLattice embedding diagnostics");
            let _ = writeln!(out, "base_url: {}", contextlattice_base_url_for_graph());
            let lines = contextlattice_embedding_diagnostics_lines().await;
            if lines.is_empty() {
                out.push_str("no diagnostic lines returned.");
            } else {
                for line in lines {
                    let _ = writeln!(out, "- {}", line);
                }
            }
            out.push_str("\nIf endpoint support is partial, Hermes falls back to `/telemetry/recall` snapshots.");
            emit_command_output(app, out.trim_end());
        }
        "repo" | "semantic" => {
            let mut max_files = 220usize;
            let mut repo_arg: Option<&str> = None;
            let mut idx = 1usize;
            while idx < args.len() {
                if args[idx] == "--max-files" {
                    if let Some(raw) = args.get(idx + 1).copied() {
                        if let Ok(parsed) = raw.parse::<usize>() {
                            max_files = parsed.clamp(20, 1500);
                        }
                        idx += 2;
                        continue;
                    }
                }
                repo_arg = Some(args[idx]);
                idx += 1;
            }
            let repo_root = if let Some(raw) = repo_arg {
                PathBuf::from(raw)
            } else {
                std::env::current_dir()
                    .map_err(|e| AgentError::Io(format!("current_dir: {}", e)))?
            };
            if !repo_root.exists() {
                emit_command_output(
                    app,
                    format!("Repo path does not exist: {}", repo_root.display()),
                );
                return Ok(CommandResult::Handled);
            }

            let mut files = Vec::new();
            collect_graph_candidate_files(&repo_root, max_files, &mut files)?;
            if files.is_empty() {
                emit_command_output(
                    app,
                    format!(
                        "No candidate source files found under {} (max_files={}).",
                        repo_root.display(),
                        max_files
                    ),
                );
                return Ok(CommandResult::Handled);
            }

            let mut edges: HashMap<(String, String), usize> = HashMap::new();
            let mut node_degree: HashMap<String, usize> = HashMap::new();
            for path in &files {
                let rel = path
                    .strip_prefix(&repo_root)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                let ext = path
                    .extension()
                    .and_then(|v| v.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let content = std::fs::read_to_string(path).unwrap_or_default();
                for rf in extract_semantic_refs_for_file(&ext, &content) {
                    let key = (rel.clone(), rf.clone());
                    *edges.entry(key).or_insert(0usize) += 1;
                    *node_degree.entry(rel.clone()).or_insert(0usize) += 1;
                    *node_degree.entry(rf).or_insert(0usize) += 1;
                }
            }

            let mut degree_ranked: Vec<(String, usize)> = node_degree.into_iter().collect();
            degree_ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let mut edge_ranked: Vec<((String, String), usize)> = edges.into_iter().collect();
            edge_ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

            let mut out = String::new();
            let _ = writeln!(out, "Semantic repo graph");
            let _ = writeln!(out, "  repo_root={}", repo_root.display());
            let _ = writeln!(out, "  files_scanned={} (cap={})", files.len(), max_files);
            let _ = writeln!(out, "  semantic_edges={}", edge_ranked.len());
            let _ = writeln!(out);
            let _ = writeln!(out, "Top hubs (degree):");
            for (idx, (node, degree)) in degree_ranked.iter().take(12).enumerate() {
                let _ = writeln!(out, "  {}. {} ({})", idx + 1, node, degree);
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "Top semantic edges:");
            for (idx, ((src, dst), weight)) in edge_ranked.iter().take(16).enumerate() {
                let _ = writeln!(out, "  {}. {} -> {} ({})", idx + 1, src, dst, weight);
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "Mermaid preview:");
            let _ = writeln!(out, "```mermaid");
            let _ = writeln!(out, "graph LR");
            for ((src, dst), _) in edge_ranked.iter().take(32) {
                let src_n = sanitize_graph_node(src);
                let dst_n = sanitize_graph_node(dst);
                let _ = writeln!(out, "  {}[\"{}\"] --> {}[\"{}\"]", src_n, src, dst_n, dst);
            }
            let _ = writeln!(out, "```");
            emit_command_output(app, out.trim_end());
        }
        "help" => emit_command_output(
            app,
            "Usage: /graph [status|embeddings|repo [path] [--max-files N]]",
        ),
        _ => emit_command_output(
            app,
            "Usage: /graph [status|embeddings|repo [path] [--max-files N]]",
        ),
    }
    Ok(CommandResult::Handled)
}

fn handle_image_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        let status = app
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
        emit_command_output(app, status);
        return Ok(CommandResult::Handled);
    }

    if args[0].eq_ignore_ascii_case("clear") {
        app.clear_pending_image_hint();
        emit_command_output(app, "Cleared pending image hint.");
        return Ok(CommandResult::Handled);
    }

    let path = args.join(" ").trim().to_string();
    if path.is_empty() {
        emit_command_output(app, "Usage: /image <path> | /image clear");
        return Ok(CommandResult::Handled);
    }
    let exists = Path::new(&path).exists();
    app.set_pending_image_hint(path.clone());
    if exists {
        emit_command_output(
            app,
            format!(
                "Image hint queued: `{}`.\nIt will be injected into the next prompt automatically.",
                path
            ),
        );
    } else {
        emit_command_output(
            app,
            format!(
                "Image hint queued: `{}` (path not found right now).\nIt will still be injected into the next prompt.",
                path
            ),
        );
    }
    Ok(CommandResult::Handled)
}

fn handle_claims_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .trim()
        .to_ascii_lowercase();
    match sub.as_str() {
        "status" => {
            let policy = load_claim_verifier_policy()?;
            emit_command_output(
                app,
                format!(
                    "Claim verifier policy\nenabled={}\nrequired={}\nmax_retries={}\nupdated_at={}\n\nWhen enabled, repo-review finalization enforces verified evidence tags before completion claims.",
                    policy.enabled, policy.required, policy.max_retries, policy.updated_at
                ),
            );
        }
        "on" | "enable" | "true" | "1" => {
            let policy = set_claim_verifier_enabled(true)?;
            crate::env_vars::set_var("HERMES_CLAIM_VERIFIER_ENABLED", "1");
            emit_command_output(
                app,
                format!(
                    "Claim verifier enabled.\nrequired={}\nmax_retries={}",
                    policy.required, policy.max_retries
                ),
            );
        }
        "off" | "disable" | "false" | "0" => {
            let policy = set_claim_verifier_enabled(false)?;
            crate::env_vars::set_var("HERMES_CLAIM_VERIFIER_ENABLED", "0");
            emit_command_output(
                app,
                format!(
                    "Claim verifier disabled.\nrequired={}\nmax_retries={}",
                    policy.required, policy.max_retries
                ),
            );
        }
        _ => emit_command_output(app, "Usage: /claims [status|on|off]"),
    }
    Ok(CommandResult::Handled)
}

fn clear_quorum_system_hints(app: &mut App) {
    app.messages.retain(|m| {
        if m.role != hermes_core::MessageRole::System {
            return true;
        }
        !m.content
            .as_deref()
            .unwrap_or_default()
            .starts_with("[QUORUM_MODE] ")
    });
}

fn install_quorum_system_hint(app: &mut App, voters: usize, models: &[String]) {
    clear_quorum_system_hints(app);
    let model_hint = if models.is_empty() {
        "current-model-only".to_string()
    } else {
        models.join(", ")
    };
    app.messages.push(hermes_core::Message::system(format!(
        "[QUORUM_MODE] Quorum reasoning is enabled. For complex decisions, evaluate at least {} independent hypotheses and present: (1) strongest case, (2) strongest counter-case, (3) final synthesis with explicit confidence. Preferred voter models: {}.",
        voters, model_hint
    )));
}

async fn handle_quorum_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .trim()
        .to_ascii_lowercase();
    match sub.as_str() {
        "status" => {
            let policy = load_quorum_policy()?;
            emit_command_output(
                app,
                format!(
                    "Quorum policy\nenabled={}\nmode={}\nvoters={}\nmodels={}\narmed_once={}\nupdated_at={}\n\nQuorum is optional and off by default to control token cost.",
                    policy.enabled,
                    policy.mode,
                    policy.voters,
                    if policy.models.is_empty() {
                        "(none)".to_string()
                    } else {
                        policy.models.join(", ")
                    },
                    app.quorum_armed_once,
                    policy.updated_at
                ),
            );
        }
        "on" | "enable" | "true" | "1" => {
            let policy = set_quorum_policy(true, None, None)?;
            crate::env_vars::set_var("HERMES_QUORUM_ENABLED", "1");
            install_quorum_system_hint(app, policy.voters, &policy.models);
            app.quorum_armed_once = false;
            emit_command_output(
                app,
                format!(
                    "Quorum mode enabled (optional deep reasoning).\nvoters={}\nmodels={}",
                    policy.voters,
                    if policy.models.is_empty() {
                        "(current model)".to_string()
                    } else {
                        policy.models.join(", ")
                    }
                ),
            );
        }
        "off" | "disable" | "false" | "0" => {
            let policy = set_quorum_policy(false, None, None)?;
            crate::env_vars::set_var("HERMES_QUORUM_ENABLED", "0");
            clear_quorum_system_hints(app);
            app.quorum_armed_once = false;
            emit_command_output(
                app,
                format!(
                    "Quorum mode disabled.\nvoters={}\nmodels={}",
                    policy.voters,
                    if policy.models.is_empty() {
                        "(none)".to_string()
                    } else {
                        policy.models.join(", ")
                    }
                ),
            );
        }
        "voters" => {
            let Some(raw) = args.get(1) else {
                emit_command_output(app, "Usage: /quorum voters <2..8>");
                return Ok(CommandResult::Handled);
            };
            let voters = raw.parse::<usize>().ok().unwrap_or(3).clamp(2, 8);
            let current = load_quorum_policy()?;
            let policy = set_quorum_policy(current.enabled, Some(voters), None)?;
            if policy.enabled {
                install_quorum_system_hint(app, policy.voters, &policy.models);
            }
            emit_command_output(app, format!("Quorum voters updated to {}.", policy.voters));
        }
        "models" => {
            if args.len() < 2 {
                emit_command_output(
                    app,
                    "Usage: /quorum models <provider:model[,provider:model,...]>",
                );
                return Ok(CommandResult::Handled);
            }
            let joined = args[1..].join(" ");
            let parsed: Vec<String> = joined
                .split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect();
            let (default_provider, _) = split_provider_model(&app.current_model);
            let default_provider = default_provider.trim().to_ascii_lowercase();
            let mut models: Vec<String> = Vec::new();
            let mut notes: Vec<String> = Vec::new();
            for raw in parsed {
                let normalized = if raw.contains(':') {
                    normalize_provider_model(raw.as_str())?
                } else {
                    normalize_provider_model(format!("{}:{}", default_provider, raw).as_str())?
                };
                let (provider, model_id) = split_provider_model(&normalized);
                let provider = provider.trim().to_ascii_lowercase();
                let model_id = model_id.trim();
                if provider.is_empty() || model_id.is_empty() {
                    continue;
                }
                let mut final_model = normalized.clone();
                let catalog = provider_model_ids(&provider).await;
                if !catalog.is_empty() {
                    if let Some(candidate) = resolve_catalog_model_candidate(model_id, &catalog) {
                        final_model = format!("{}:{}", provider, candidate.trim());
                        if !final_model.eq_ignore_ascii_case(&normalized) {
                            notes.push(format!("{} -> {}", normalized, final_model));
                        }
                    } else if let Some(fallback) = catalog.first() {
                        let close = rank_catalog_model_candidates(model_id, &catalog, 3);
                        final_model = format!("{}:{}", provider, fallback.trim());
                        notes.push(format!(
                            "{} -> {} (close: {})",
                            normalized,
                            final_model,
                            if close.is_empty() {
                                "(none)".to_string()
                            } else {
                                close.join(", ")
                            }
                        ));
                    }
                }
                if !models
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&final_model))
                {
                    models.push(final_model);
                }
            }
            let current = load_quorum_policy()?;
            let policy = set_quorum_policy(current.enabled, None, Some(models))?;
            if policy.enabled {
                install_quorum_system_hint(app, policy.voters, &policy.models);
            }
            emit_command_output(
                app,
                if notes.is_empty() {
                    format!(
                        "Quorum models updated: {}",
                        if policy.models.is_empty() {
                            "(none)".to_string()
                        } else {
                            policy.models.join(", ")
                        }
                    )
                } else {
                    format!(
                        "Quorum models updated: {}\nCatalog remaps: {}",
                        if policy.models.is_empty() {
                            "(none)".to_string()
                        } else {
                            policy.models.join(", ")
                        },
                        notes.join(" | ")
                    )
                },
            );
        }
        "run" => {
            let policy = load_quorum_policy()?;
            if !policy.enabled {
                emit_command_output(
                    app,
                    "Quorum mode is OFF. Run `/quorum on` first (kept optional to control token cost).",
                );
                return Ok(CommandResult::Handled);
            }
            install_quorum_system_hint(app, policy.voters, &policy.models);
            app.quorum_armed_once = true;
            emit_command_output(
                app,
                "Quorum deep-reasoning armed for subsequent turns.\nNext user prompt will run multi-voter fan-out across configured models and return synthesis (plus persisted quorum artifact).",
            );
        }
        _ => emit_command_output(
            app,
            "Usage: /quorum [status|on|off|voters <2..8>|models <a,b,c>|run]",
        ),
    }
    Ok(CommandResult::Handled)
}

fn parse_swarm_mode(input: Option<&str>) -> SwarmExecutionMode {
    match input
        .unwrap_or("concurrent")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "sequential" | "sequence" => SwarmExecutionMode::Sequential,
        "graph" | "dag" => SwarmExecutionMode::Graph,
        _ => SwarmExecutionMode::Concurrent,
    }
}

fn read_swarm_pass_cap() -> usize {
    let raw = std::env::var("HERMES_QUORUM_VOTER_PASSES").unwrap_or_else(|_| "6".to_string());
    let normalized = raw.trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "0" | "off" | "unlimited" | "infinite") {
        return 64;
    }
    normalized.parse::<usize>().ok().unwrap_or(6).clamp(1, 64)
}

fn latest_quorum_artifact_path(app: &App) -> Option<PathBuf> {
    let dir = app.state_root.join("quorum");
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut best_session: Option<(SystemTime, PathBuf)> = None;
    let mut best_any: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if let Some((best_time, _)) = &best_any {
            if modified > *best_time {
                best_any = Some((modified, path.clone()));
            }
        } else {
            best_any = Some((modified, path.clone()));
        }

        let file_name = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        if !file_name.starts_with(&format!("{}-", app.session_id)) {
            continue;
        }
        if let Some((best_time, _)) = &best_session {
            if modified > *best_time {
                best_session = Some((modified, path.clone()));
            }
        } else {
            best_session = Some((modified, path.clone()));
        }
    }
    best_session.or(best_any).map(|(_, path)| path)
}

async fn handle_swarm_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let sub = args
        .first()
        .copied()
        .unwrap_or("status")
        .trim()
        .to_ascii_lowercase();

    match sub.as_str() {
        "status" => {
            let policy = load_quorum_policy()?;
            let runtime = swarm_runtime_status();
            let artifact_path = latest_quorum_artifact_path(app)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(none yet)".to_string());
            let mut out = String::new();
            let _ = writeln!(out, "Swarm runtime");
            let _ = writeln!(out, "engine={}", runtime.engine);
            let _ = writeln!(out, "feature_enabled={}", runtime.feature_enabled);
            let _ = writeln!(
                out,
                "supported_modes={}",
                runtime
                    .supported_modes
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let _ = writeln!(
                out,
                "quorum_policy=enabled:{} voters:{} models:{} armed_once:{}",
                policy.enabled,
                policy.voters,
                if policy.models.is_empty() {
                    "(current model)".to_string()
                } else {
                    policy.models.join(", ")
                },
                app.quorum_armed_once
            );
            let _ = writeln!(out, "latest_artifact={}", artifact_path);
            if !runtime.notes.is_empty() {
                let _ = writeln!(out, "notes:");
                for note in runtime.notes {
                    let _ = writeln!(out, "- {}", note);
                }
            }
            emit_command_output(app, out.trim_end());
        }
        "plan" => {
            let policy = load_quorum_policy()?;
            let mode = parse_swarm_mode(args.get(1).copied());
            let pass_cap = read_swarm_pass_cap();
            let models = if policy.models.is_empty() {
                vec![app.current_model.clone()]
            } else {
                policy.models.clone()
            };
            let plan = build_swarm_execution_plan(
                mode,
                policy.voters,
                models,
                app.session_objective.clone(),
                pass_cap,
            );
            let pretty = serde_json::to_string_pretty(&plan)
                .map_err(|e| AgentError::Config(format!("failed to render swarm plan: {e}")))?;
            emit_command_output(
                app,
                format!(
                    "Swarm execution plan\n{}\n\nUsage: /swarm run [passes] [mode]\nmode: concurrent|sequential|graph",
                    pretty
                ),
            );
        }
        "run" => {
            let pass_override = args
                .get(1)
                .and_then(|raw| raw.trim().parse::<usize>().ok())
                .map(|v| v.clamp(1, 64));
            let mode = if pass_override.is_some() {
                parse_swarm_mode(args.get(2).copied())
            } else {
                parse_swarm_mode(args.get(1).copied())
            };
            if let Some(passes) = pass_override {
                crate::env_vars::set_var("HERMES_QUORUM_VOTER_PASSES", passes.to_string());
            }
            let policy = load_quorum_policy()?;
            if !policy.enabled {
                emit_command_output(
                    app,
                    "Swarm run blocked: quorum policy is OFF.\nRun `/swarm on` (or `/quorum on`) first to keep cost explicit.",
                );
                return Ok(CommandResult::Handled);
            }
            install_quorum_system_hint(app, policy.voters, &policy.models);
            app.quorum_armed_once = true;
            emit_command_output(
                app,
                format!(
                    "Swarm run armed.\nmode={}\npass_cap={}\nnext user prompt will execute multi-voter fan-out + synthesis and persist an artifact.",
                    mode.as_str(),
                    read_swarm_pass_cap(),
                ),
            );
        }
        "cancel" => {
            app.quorum_armed_once = false;
            clear_quorum_system_hints(app);
            emit_command_output(
                app,
                "Swarm run canceled. Pending one-shot fan-out was disarmed.",
            );
        }
        "artifact" => {
            let Some(path) = latest_quorum_artifact_path(app) else {
                emit_command_output(app, "No swarm/quorum artifact exists yet for this runtime.");
                return Ok(CommandResult::Handled);
            };
            let summary = std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .map(|v| {
                    let session_id = v
                        .get("session_id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("unknown");
                    let saved_at = v
                        .get("saved_at")
                        .and_then(|x| x.as_str())
                        .unwrap_or("unknown");
                    let voters = v
                        .get("voters")
                        .and_then(|x| x.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    format!("session_id={session_id}\nsaved_at={saved_at}\nvoters={voters}")
                })
                .unwrap_or_else(|| "(unable to parse artifact summary)".to_string());
            emit_command_output(
                app,
                format!(
                    "Latest swarm artifact\npath={}\n{}",
                    path.display(),
                    summary
                ),
            );
        }
        "on" | "off" | "enable" | "disable" | "true" | "false" | "1" | "0" | "voters"
        | "models" => return handle_quorum_command(app, args).await,
        _ => emit_command_output(
            app,
            "Usage: /swarm [status|plan [mode]|run [passes] [mode]|cancel|artifact|on|off|voters <2..8>|models <a,b,c>]",
        ),
    }
    Ok(CommandResult::Handled)
}

fn specpatch_block_reason(command: &str) -> Option<&'static str> {
    let lower = command.to_ascii_lowercase();
    if lower.contains("rm -rf /")
        || lower.contains("dd if=")
        || lower.contains("mkfs")
        || lower.contains("shutdown")
    {
        return Some("destructive command pattern");
    }
    if lower.contains("git reset --hard") || lower.contains("git clean -fdx") {
        return Some("history/destructive git command pattern");
    }
    None
}

fn slash_command_payload_from_history(app: &App, cmd: &str, args: &[&str]) -> String {
    let fallback = args.join(" ");
    let Some(last) = app.input_history.last() else {
        return fallback;
    };
    if let Some(raw) = last.strip_prefix(cmd) {
        return raw.trim().to_string();
    }
    fallback
}

async fn run_shell_capture(command: &str) -> Result<(i32, String, String), AgentError> {
    let output = tokio::process::Command::new("bash")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AgentError::Io(format!("shell command failed: {}", e)))?;
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok((code, stdout, stderr))
}

async fn handle_specpatch_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let payload = slash_command_payload_from_history(app, "/specpatch", args);
    if payload.is_empty() {
        emit_command_output(
            app,
            "Usage: /specpatch <verify_cmd> | <candidate_cmd_1> | <candidate_cmd_2> ...",
        );
        return Ok(CommandResult::Handled);
    }
    let segments: Vec<String> = payload
        .split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() < 2 {
        emit_command_output(
            app,
            "Need at least a verify command and one candidate.\nExample: /specpatch \"cargo test -p hermes-cli\" | \"git apply fix.patch\"",
        );
        return Ok(CommandResult::Handled);
    }
    let verify_cmd = segments[0].clone();
    let candidates = &segments[1..];

    if let Some(reason) = specpatch_block_reason(&verify_cmd) {
        emit_command_output(app, format!("specpatch blocked verify_cmd: {}", reason));
        return Ok(CommandResult::Handled);
    }

    let mut out = String::new();
    out.push_str("SpecPatch executor\n");
    out.push_str("------------------\n");
    let _ = writeln!(out, "verify_cmd: {}", verify_cmd);

    let mut winner: Option<String> = None;
    for (idx, candidate) in candidates.iter().enumerate() {
        if let Some(reason) = specpatch_block_reason(candidate) {
            let _ = writeln!(
                out,
                "[{}] blocked candidate: {} ({})",
                idx + 1,
                candidate,
                reason
            );
            continue;
        }
        let _ = writeln!(out, "[{}] candidate: {}", idx + 1, candidate);
        let (code, stdout, stderr) = run_shell_capture(candidate).await?;
        let _ = writeln!(out, "    apply_exit={}", code);
        if !stdout.is_empty() {
            let _ = writeln!(
                out,
                "    apply_stdout={}",
                stdout.lines().next().unwrap_or("")
            );
        }
        if !stderr.is_empty() {
            let _ = writeln!(
                out,
                "    apply_stderr={}",
                stderr.lines().next().unwrap_or("")
            );
        }
        let (v_code, v_stdout, v_stderr) = run_shell_capture(&verify_cmd).await?;
        let _ = writeln!(out, "    verify_exit={}", v_code);
        if !v_stdout.is_empty() {
            let _ = writeln!(
                out,
                "    verify_stdout={}",
                v_stdout.lines().next().unwrap_or("")
            );
        }
        if !v_stderr.is_empty() {
            let _ = writeln!(
                out,
                "    verify_stderr={}",
                v_stderr.lines().next().unwrap_or("")
            );
        }
        if v_code == 0 {
            winner = Some(candidate.clone());
            break;
        }
    }

    if let Some(chosen) = winner {
        let _ = writeln!(out, "\nwinner={}", chosen);
    } else {
        out.push_str("\nNo candidate passed verify command.\n");
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn objective_runtime_ledger_path() -> PathBuf {
    hermes_config::hermes_home()
        .join("alpha")
        .join("objective_runtime_ledger.jsonl")
}

fn normalize_repo_relative_path(repo_root: &Path, raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .trim_matches(',');
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        let rel = path.strip_prefix(repo_root).ok()?;
        return Some(rel.display().to_string());
    }
    Some(path.display().to_string())
}

fn extract_marker_paths(text: &str) -> Vec<String> {
    let Ok(re) = Regex::new(r"(?:path|file)=([^\s\],;]+)") else {
        return Vec::new();
    };
    re.captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

async fn count_git_tracked_files(repo_root: &Path) -> Result<usize, AgentError> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| AgentError::Io(format!("git ls-files failed: {}", e)))?;
    if !output.status.success() {
        return Ok(0);
    }
    let count = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    Ok(count)
}

async fn handle_heatmap_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let repo_root = if let Some(path) = args.first() {
        PathBuf::from(path)
    } else if let Some(root) = discover_repo_root_for_about() {
        root
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    if !repo_root.exists() {
        emit_command_output(
            app,
            format!("Repo path does not exist: {}", repo_root.display()),
        );
        return Ok(CommandResult::Handled);
    }

    let mut counts: HashMap<String, u64> = HashMap::new();
    let ledger_path = objective_runtime_ledger_path();
    if ledger_path.exists() {
        let raw = std::fs::read_to_string(&ledger_path).unwrap_or_default();
        for line in raw.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if let Some(files) = value.get("evidence_files").and_then(|v| v.as_array()) {
                for raw_path in files.iter().filter_map(|v| v.as_str()) {
                    if let Some(path) = normalize_repo_relative_path(&repo_root, raw_path) {
                        *counts.entry(path).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    for msg in &app.messages {
        if let Some(content) = msg.content.as_deref() {
            for raw_path in extract_marker_paths(content) {
                if let Some(path) = normalize_repo_relative_path(&repo_root, &raw_path) {
                    *counts.entry(path).or_insert(0) += 1;
                }
            }
        }
    }

    let tracked = count_git_tracked_files(&repo_root).await?;
    let mut rows: Vec<(String, u64, bool)> = counts
        .into_iter()
        .map(|(path, hits)| {
            let exists = repo_root.join(&path).exists();
            (path, hits, exists)
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let verified_existing = rows.iter().filter(|(_, _, exists)| *exists).count();
    let coverage_pct = if tracked == 0 {
        0.0
    } else {
        (verified_existing as f64 / tracked as f64) * 100.0
    };

    let mut out = String::new();
    out.push_str("Context heatmap\n");
    out.push_str("---------------\n");
    let _ = writeln!(out, "repo_root={}", repo_root.display());
    let _ = writeln!(out, "tracked_files={}", tracked);
    let _ = writeln!(out, "observed_paths={}", rows.len());
    let _ = writeln!(
        out,
        "verified_existing_paths={} ({:.2}% coverage of tracked files)",
        verified_existing, coverage_pct
    );
    for (path, hits, exists) in rows.iter().take(30) {
        let _ = writeln!(out, "- hits={:<4} exists={} path={}", hits, exists, path);
    }
    if rows.is_empty() {
        out.push_str("- no evidence paths recorded yet\n");
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn read_replay_export_rows(path: &Path) -> Result<Vec<serde_json::Value>, AgentError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| AgentError::Io(format!("Failed to read {}: {}", path.display(), e)))?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        AgentError::Config(format!(
            "Failed to parse replay export {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(parsed
        .get("rows")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

async fn handle_studio_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        emit_command_output(
            app,
            "Usage: /studio replay [status|verify [path]|diff <export_a.json> <export_b.json>]",
        );
        return Ok(CommandResult::Handled);
    }
    let section = args[0].trim().to_ascii_lowercase();
    if section != "replay" {
        emit_command_output(
            app,
            "Usage: /studio replay [status|verify [path]|diff <export_a.json> <export_b.json>]",
        );
        return Ok(CommandResult::Handled);
    }
    let action = args
        .get(1)
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "status".to_string());
    match action.as_str() {
        "status" => {
            let replay_path = replay_log_path_for_session(&app.session_id);
            let export_dir = hermes_config::hermes_home()
                .join("logs")
                .join("replay")
                .join("exports");
            emit_command_output(
                app,
                format!(
                    "Replay studio status\nsession={}\nreplay_log={}\nreplay_exists={}\nexport_dir={}",
                    app.session_id,
                    replay_path.display(),
                    replay_path.exists(),
                    export_dir.display()
                ),
            );
        }
        "verify" => {
            let replay_path = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| replay_log_path_for_session(&app.session_id));
            if !replay_path.exists() {
                emit_command_output(
                    app,
                    format!("Replay file not found: {}", replay_path.display()),
                );
                return Ok(CommandResult::Handled);
            }
            if replay_path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                let rows = read_replay_export_rows(&replay_path)?;
                emit_command_output(
                    app,
                    format!(
                        "Replay export verification\npath={}\nrows={}\nstatus={}",
                        replay_path.display(),
                        rows.len(),
                        if rows.is_empty() { "empty" } else { "ok" }
                    ),
                );
            } else {
                let (entries, parse_errors, chain_breaks) = replay_trace_integrity(&replay_path)?;
                emit_command_output(
                    app,
                    format!(
                        "Replay log verification\npath={}\nentries={}\nparse_errors={}\nchain_breaks={}\nstatus={}",
                        replay_path.display(),
                        entries,
                        parse_errors,
                        chain_breaks,
                        if parse_errors == 0 && chain_breaks == 0 {
                            "pass"
                        } else {
                            "fail"
                        }
                    ),
                );
            }
        }
        "diff" => {
            if args.len() < 4 {
                emit_command_output(
                    app,
                    "Usage: /studio replay diff <export_a.json> <export_b.json>",
                );
                return Ok(CommandResult::Handled);
            }
            let a = PathBuf::from(args[2]);
            let b = PathBuf::from(args[3]);
            let a_rows = read_replay_export_rows(&a)?;
            let b_rows = read_replay_export_rows(&b)?;
            let a_hashes: HashSet<String> = a_rows
                .iter()
                .filter_map(|row| {
                    row.get("event_hash")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            let b_hashes: HashSet<String> = b_rows
                .iter()
                .filter_map(|row| {
                    row.get("event_hash")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            let only_a = a_hashes.difference(&b_hashes).count();
            let only_b = b_hashes.difference(&a_hashes).count();
            let overlap = a_hashes.intersection(&b_hashes).count();
            emit_command_output(
                app,
                format!(
                    "Replay diff\nA={} rows={} hashes={}\nB={} rows={} hashes={}\noverlap_hashes={}\nonly_in_a={}\nonly_in_b={}",
                    a.display(),
                    a_rows.len(),
                    a_hashes.len(),
                    b.display(),
                    b_rows.len(),
                    b_hashes.len(),
                    overlap,
                    only_a,
                    only_b
                ),
            );
        }
        _ => emit_command_output(
            app,
            "Usage: /studio replay [status|verify [path]|diff <export_a.json> <export_b.json>]",
        ),
    }
    Ok(CommandResult::Handled)
}

fn handle_interactive_question_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        emit_command_output(
            app,
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
            app,
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
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_insights_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let msg_count = app.messages.len();
    let user_count = app
        .messages
        .iter()
        .filter(|m| m.role == hermes_core::MessageRole::User)
        .count();
    let assistant_count = app
        .messages
        .iter()
        .filter(|m| m.role == hermes_core::MessageRole::Assistant)
        .count();
    emit_command_output(
        app,
        format!(
            "Session insights:\n  - Total messages: {}\n  - User messages: {}\n  - Hermes messages: {}\n  - Session: {}",
            msg_count, user_count, assistant_count, app.session_id
        ),
    );
    Ok(CommandResult::Handled)
}

fn handle_platforms_command(app: &mut App) -> Result<CommandResult, AgentError> {
    if app.config.platforms.is_empty() {
        emit_command_output(
            app,
            "No explicit gateway platform adapters configured (running in local CLI mode).",
        );
        return Ok(CommandResult::Handled);
    }
    let mut entries: Vec<_> = app.config.platforms.keys().cloned().collect();
    entries.sort();
    let mut out = String::from("Configured gateway platforms:\n");
    for p in entries {
        let _ = writeln!(out, "  - {}", p);
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn integrations_snapshot_path(session_id: &str) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    hermes_config::hermes_home().join("logs").join(format!(
        "integrations-snapshot-{}-{}.json",
        session_id, stamp
    ))
}

fn render_integrations_repair_steps(
    provider: &str,
    auth_ok: bool,
    oauth_gate: Option<(bool, String)>,
    memory_probe: &str,
) -> String {
    let mut out = String::new();
    out.push_str("Integrations repair plan\n");
    out.push_str("------------------------\n");
    let _ = writeln!(out, "provider: {}", provider);
    if !auth_ok {
        out.push_str("- auth: FAIL -> run `/auth status` then `/auth verify` (or `hermes-ultra auth add`).\n");
    } else {
        out.push_str("- auth: PASS\n");
    }
    if let Some((ok, detail)) = oauth_gate {
        if ok {
            let _ = writeln!(out, "- oauth runtime gate: PASS ({})", detail);
        } else {
            let _ = writeln!(
                out,
                "- oauth runtime gate: FAIL ({}) -> rebuild/install latest CLI binary.",
                detail
            );
        }
    }
    if memory_probe.to_ascii_lowercase().starts_with("warn") {
        let _ = writeln!(
            out,
            "- contextlattice probe: {} -> verify local orchestrator and env vars (CONTEXTLATTICE_ORCHESTRATOR_URL/MEMMCP_ORCHESTRATOR_URL).",
            memory_probe
        );
    } else {
        let _ = writeln!(out, "- contextlattice probe: {}", memory_probe);
    }
    out.push_str(
        "- tools: run `/tools` and `/integrations status` to verify adapter registry health.\n",
    );
    out.push_str(
        "- walkthrough: run `/walkthrough next` to continue operator recovery sequence.\n",
    );
    out
}

async fn handle_integrations_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    let provider = app.current_runtime_provider();
    let provider_cap = crate::providers::provider_capability_for(&provider);
    let oauth_capable = provider_cap
        .as_ref()
        .map(|cap| cap.oauth_supported)
        .unwrap_or(false);
    let managed_tools = provider_cap
        .as_ref()
        .map(|cap| cap.managed_tools_supported)
        .unwrap_or(false);
    let credential_present = crate::app::provider_api_key_from_env(&provider).is_some();
    let oauth_state_present = crate::auth::read_provider_auth_state(&provider)
        .ok()
        .flatten()
        .is_some();
    let auth_ok = credential_present || (oauth_capable && oauth_state_present);
    let oauth_gate = policy::oauth_runtime_gate_for_provider(&provider);
    let oauth_manifest_source = policy::oauth_min_version_for_provider(&provider)
        .map(|(_, source)| source)
        .unwrap_or_else(|| "n/a".to_string());

    let memory_url = std::env::var("CONTEXTLATTICE_ORCHESTRATOR_URL")
        .ok()
        .or_else(|| std::env::var("MEMMCP_ORCHESTRATOR_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8075".to_string());
    let memory_probe = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => {
            let health_url = format!("{}/health", memory_url.trim_end_matches('/'));
            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => format!("PASS ({})", health_url),
                Ok(resp) => format!("WARN ({} status={})", health_url, resp.status()),
                Err(err) => format!(
                    "WARN ({} error={})",
                    health_url,
                    truncate_chars(&err.to_string(), 96)
                ),
            }
        }
        Err(err) => format!(
            "WARN (client build failed: {})",
            truncate_chars(&err.to_string(), 96)
        ),
    };

    let tools_count = app.tool_registry.list_tools().len();
    let plugins_count = discover_plugin_surface(true).len();
    let mcp_count = app.config.mcp_servers.len();
    let platforms_count = app.config.platforms.len();

    if action == "repair" {
        emit_command_output(
            app,
            render_integrations_repair_steps(&provider, auth_ok, oauth_gate.clone(), &memory_probe),
        );
        return Ok(CommandResult::Handled);
    }

    if action == "snapshot" {
        let path = integrations_snapshot_path(&app.session_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentError::Io(format!("Failed to create {}: {}", parent.display(), e))
            })?;
        }
        let payload = serde_json::json!({
            "captured_at": chrono::Utc::now().to_rfc3339(),
            "session_id": app.session_id,
            "provider": provider,
            "model": app.current_model,
            "auth": {
                "oauth_capable": oauth_capable,
                "managed_tools_supported": managed_tools,
                "credential_present": credential_present,
                "oauth_state_present": oauth_state_present,
                "status": if auth_ok { "PASS" } else { "FAIL" },
                "oauth_runtime_gate": oauth_gate.as_ref().map(|(ok, detail)| serde_json::json!({"ok": ok, "detail": detail})),
            },
            "panels": {
                "providers_count": curated_provider_slugs().len(),
                "platform_adapters": platforms_count,
                "mcp_servers": mcp_count,
                "plugins": plugins_count,
                "toolsets": app.config.platform_toolsets.len(),
                "registered_tools": tools_count,
                "contextlattice_url": memory_url,
                "memory_probe": memory_probe,
            }
        });
        let json = serde_json::to_string_pretty(&payload)
            .map_err(|e| AgentError::Io(format!("Failed to encode snapshot payload: {}", e)))?;
        std::fs::write(&path, json)
            .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", path.display(), e)))?;
        emit_command_output(
            app,
            format!(
                "Integration snapshot exported:\n{}\nUse `/integrations repair` for remediation guidance.",
                path.display()
            ),
        );
        return Ok(CommandResult::Handled);
    }

    let mut out = String::new();
    out.push_str("Integration Control Plane\n");
    out.push_str("=========================\n");

    if action == "status" || action == "all" || action == "auth" {
        out.push_str("Auth panel\n----------\n");
        let _ = writeln!(out, "provider: {}", provider);
        let _ = writeln!(out, "model: {}", app.current_model);
        let _ = writeln!(out, "oauth_capable: {}", oauth_capable);
        let _ = writeln!(out, "managed_tools_supported: {}", managed_tools);
        let _ = writeln!(out, "credential_present: {}", credential_present);
        let _ = writeln!(out, "oauth_state_present: {}", oauth_state_present);
        let _ = writeln!(out, "status: {}", if auth_ok { "PASS" } else { "FAIL" });
        let _ = writeln!(out, "oauth_manifest: {}", oauth_manifest_source);
        if let Some((gate_ok, gate_detail)) = oauth_gate.clone() {
            let _ = writeln!(
                out,
                "oauth_runtime_gate: {} ({})",
                if gate_ok { "PASS" } else { "FAIL" },
                gate_detail
            );
            if !gate_ok {
                out.push_str("remediation: upgrade runtime and retry auth.\n");
            }
        }
        out.push('\n');
    }

    if action == "status" || action == "all" || action == "providers" {
        let providers = curated_provider_slugs();
        out.push_str("Providers panel\n---------------\n");
        let _ = writeln!(out, "configured_providers: {}", providers.join(", "));
        let _ = writeln!(out, "provider_count: {}", providers.len());
        out.push('\n');
    }

    if action == "status" || action == "all" || action == "gateway" {
        out.push_str("Gateway panel\n-------------\n");
        let _ = writeln!(out, "platform_adapters: {}", platforms_count);
        let _ = writeln!(out, "mcp_servers: {}", mcp_count);
        let _ = writeln!(out, "plugins: {}", plugins_count);
        let _ = writeln!(out, "toolsets: {}", app.config.platform_toolsets.len());
        out.push('\n');
    }

    if action == "status" || action == "all" || action == "memory" {
        out.push_str("Memory panel\n------------\n");
        let _ = writeln!(out, "contextlattice_url: {}", memory_url);
        let _ = writeln!(out, "probe: {}", memory_probe);
        let _ = writeln!(out, "registered_tools: {}", tools_count);
        out.push('\n');
    }

    if !matches!(
        action.as_str(),
        "status" | "all" | "auth" | "providers" | "gateway" | "memory" | "repair" | "snapshot"
    ) {
        emit_command_output(
            app,
            "Usage: /integrations [status|all|auth|providers|gateway|memory|repair|snapshot]",
        );
        return Ok(CommandResult::Handled);
    }

    out.push_str("Next actions:\n");
    out.push_str("- `/boot` for startup readiness\n");
    out.push_str("- `/auth verify` for runtime credential hydration\n");
    out.push_str("- `/walkthrough next` for guided operator setup\n");
    out.push_str(
        "- `/integrations repair` for remediation plan and `/integrations snapshot` for export\n",
    );
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_log_command(app: &mut App) -> Result<CommandResult, AgentError> {
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
        emit_command_output(app, format!("No log files found in {}", logs_dir.display()));
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
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_debug_dump_command(app: &mut App, _args: &[&str]) -> Result<CommandResult, AgentError> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let prefix = app.session_id.chars().take(8).collect::<String>();
    let stem = format!("debug-{}-{}", prefix, stamp);
    let snapshot_path = app.persist_session_snapshot(Some(&stem))?;
    let logs_dir = hermes_config::hermes_home().join("logs");
    let log_files = std::fs::read_dir(&logs_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.filter_map(|entry| entry.ok()))
        .filter(|entry| entry.path().is_file())
        .count();
    let out = format!(
        "Debug snapshot written.\n  session_id: {}\n  model: {}\n  messages: {}\n  snapshot: {}\n  logs_dir: {} ({} files)\nTip: run `hermes debug share --local` for a support bundle.",
        app.session_id,
        app.current_model,
        app.messages.len(),
        snapshot_path.display(),
        logs_dir.display(),
        log_files
    );
    emit_command_output(app, out);
    Ok(CommandResult::Handled)
}

fn handle_dump_format_command(app: &mut App) -> Result<CommandResult, AgentError> {
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
        app.state_root.display()
    );
    let _ = writeln!(out, "Use `/save [name]` to persist a snapshot now.");
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_experiment_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        let active = objective::current_session_steer(app)
            .filter(|value| value.to_ascii_lowercase().starts_with("experiment: "))
            .map(|value| value.trim_start_matches("Experiment: ").to_string())
            .unwrap_or_else(|| "(none)".to_string());
        emit_command_output(
            app,
            format!(
                "Experiment steering: {}\nUsage: /experiment <label or instruction> | /experiment clear",
                active
            ),
        );
        return Ok(CommandResult::Handled);
    }
    if args[0].eq_ignore_ascii_case("clear") {
        let active = objective::current_session_steer(app)
            .map(|value| value.to_ascii_lowercase().starts_with("experiment: "))
            .unwrap_or(false);
        if active {
            objective::set_session_steer(app, None);
            emit_command_output(app, "Cleared experiment steering context.");
        } else {
            emit_command_output(
                app,
                "No experiment steering context active. Use `/experiment <instruction>`.",
            );
        }
        return Ok(CommandResult::Handled);
    }
    let hint = args.join(" ").trim().to_string();
    if hint.is_empty() {
        emit_command_output(
            app,
            "Usage: /experiment <label or instruction> | /experiment clear",
        );
        return Ok(CommandResult::Handled);
    }
    let steer = format!("Experiment: {hint}");
    objective::set_session_steer(app, Some(steer.clone()));
    emit_command_output(
        app,
        format!(
            "Experiment steering applied.\n{}\nUse `/model` to switch variants, then `/retry` to re-run the last turn.",
            steer
        ),
    );
    Ok(CommandResult::Handled)
}

fn feedback_log_path() -> PathBuf {
    hermes_config::hermes_home()
        .join("logs")
        .join("feedback.ndjson")
}

fn handle_feedback_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.is_empty() {
        emit_command_output(
            app,
            "Usage: /feedback <note>\nStores a local feedback record at ~/.hermes-agent-ultra/logs/feedback.ndjson.",
        );
        return Ok(CommandResult::Handled);
    }
    let note = args.join(" ").trim().to_string();
    if note.is_empty() {
        emit_command_output(app, "Usage: /feedback <note>");
        return Ok(CommandResult::Handled);
    }
    let path = feedback_log_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", parent.display(), e)))?;
    }
    let record = serde_json::json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "session_id": app.session_id,
        "model": app.current_model,
        "note": note,
    });
    let mut writer = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| AgentError::Io(format!("Failed to open {}: {}", path.display(), e)))?;
    writer
        .write_all(format!("{}\n", record).as_bytes())
        .map_err(|e| AgentError::Io(format!("Failed to append {}: {}", path.display(), e)))?;
    emit_command_output(app, format!("Feedback captured in {}", path.display()));
    Ok(CommandResult::Handled)
}

fn handle_restart_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let preserve_model = args.first().is_some_and(|v| {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "keep-model" | "--keep-model"
        )
    });
    let previous_model = app.current_model.clone();
    app.new_session();
    if preserve_model && !previous_model.eq_ignore_ascii_case(&app.current_model) {
        app.switch_model(&previous_model);
    }
    emit_command_output(
        app,
        format!(
            "Session restarted.\n  new_session_id: {}\n  model: {}",
            app.session_id, app.current_model
        ),
    );
    Ok(CommandResult::Handled)
}

async fn handle_update_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let check_only = args
        .first()
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "check" | "--check"));
    let report = crate::update::check_for_updates().await?;
    let mut out = String::new();
    let _ = writeln!(out, "Update status");
    if check_only {
        let _ = writeln!(out, "  mode: check-only");
    }
    let _ = writeln!(out, "{}", report.trim());
    if !check_only {
        let _ = writeln!(out, "\nTo perform the update, exit and run: hermes update");
    }
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_redraw_command(app: &mut App) -> Result<CommandResult, AgentError> {
    app.push_ui_assistant("↻ Repaint pulse requested.");
    emit_command_output(
        app,
        "Repaint pulse sent.\nIf the screen still looks stale: press Ctrl+L (lane toggle) or resize the terminal once.",
    );
    Ok(CommandResult::Handled)
}

fn handle_paste_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let text = if let Some(mock) = std::env::var("HERMES_TEST_CLIPBOARD_TEXT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        mock
    } else {
        arboard::Clipboard::new()
            .and_then(|mut cb| cb.get_text())
            .map_err(|e| AgentError::Config(format!("Clipboard unavailable: {}", e)))?
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        emit_command_output(app, "Clipboard is empty.");
        return Ok(CommandResult::Handled);
    }
    let pastes_dir = hermes_config::hermes_home().join("pastes");
    std::fs::create_dir_all(&pastes_dir)
        .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", pastes_dir.display(), e)))?;
    let file_name = format!("paste-{}.txt", chrono::Utc::now().format("%Y%m%d-%H%M%S"));
    let path = pastes_dir.join(file_name);
    std::fs::write(&path, trimmed)
        .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", path.display(), e)))?;

    let preview = if args.first().is_some_and(|v| v.eq_ignore_ascii_case("show")) {
        trimmed.to_string()
    } else {
        truncate_chars(trimmed, 280)
    };

    let mut out = String::new();
    let _ = writeln!(out, "Clipboard captured:");
    let _ = writeln!(out, "  - chars: {}", trimmed.chars().count());
    let _ = writeln!(out, "  - saved: {}", path.display());
    let _ = writeln!(out, "  - preview: {}", preview);
    let _ = writeln!(
        out,
        "Use `/background review {}` to process it in isolation.",
        path.display()
    );
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

async fn handle_gquota_command(app: &mut App, _args: &[&str]) -> Result<CommandResult, AgentError> {
    let provider = app
        .current_model
        .split_once(':')
        .map(|(p, _)| p.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_string());
    let gemini_vars = [
        "HERMES_GEMINI_OAUTH_API_KEY",
        "GOOGLE_API_KEY",
        "GEMINI_API_KEY",
    ];
    let mut present = Vec::new();
    for key in gemini_vars {
        if std::env::var(key)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
        {
            present.push(key.to_string());
        }
    }
    let oauth_state = crate::auth::read_provider_auth_state("google-gemini-cli")
        .ok()
        .flatten();
    let expires_at = oauth_state
        .as_ref()
        .and_then(|v| v.get("expires_at_ms"))
        .and_then(|v| v.as_i64());
    let mut out = String::new();
    let _ = writeln!(out, "Gemini quota/auth diagnostics");
    let _ = writeln!(out, "  - active provider: {}", provider);
    let _ = writeln!(
        out,
        "  - gemini creds in env: {} ({})",
        if present.is_empty() { "no" } else { "yes" },
        if present.is_empty() {
            "none".to_string()
        } else {
            present.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "  - oauth state file: {}",
        if oauth_state.is_some() {
            "present"
        } else {
            "missing"
        }
    );
    if let Some(ms) = expires_at {
        let ts = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "invalid".to_string());
        let _ = writeln!(out, "  - token expiry: {}", ts);
    }
    let _ = writeln!(
        out,
        "  - live quota API: unavailable in local CLI; check provider dashboard for hard usage limits."
    );
    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}

fn handle_approve_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let store = PairingStore::open_default();
    if args.is_empty() || args[0].eq_ignore_ascii_case("list") {
        let pending: Vec<_> = store
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.status == PairingStatus::Pending)
            .collect();
        if pending.is_empty() {
            emit_command_output(
                app,
                "No pending devices to approve. Use `hermes pairing list` for full inventory.",
            );
            return Ok(CommandResult::Handled);
        }
        let mut out = String::from("Pending pairing devices:\n");
        for dev in pending {
            out.push_str(&format!(
                "  - {} ({})\n",
                dev.device_id,
                dev.name.unwrap_or_else(|| "unnamed".to_string())
            ));
        }
        out.push_str("Approve one with `/approve <device-id>` or all with `/approve all`.");
        emit_command_output(app, out.trim_end());
        return Ok(CommandResult::Handled);
    }

    if args[0].eq_ignore_ascii_case("all") {
        let mut approved = 0usize;
        for dev in store.list().unwrap_or_default() {
            if dev.status == PairingStatus::Pending && store.approve(&dev.device_id).is_ok() {
                approved += 1;
            }
        }
        emit_command_output(app, format!("Approved {} pending device(s).", approved));
        return Ok(CommandResult::Handled);
    }

    match store.approve(args[0]) {
        Ok(dev) => emit_command_output(
            app,
            format!(
                "Approved device '{}' (name={}).",
                dev.device_id,
                dev.name.unwrap_or_else(|| "unnamed".to_string())
            ),
        ),
        Err(err) => emit_command_output(
            app,
            format!(
                "Approve failed: {}. Use `/approve list` or `hermes pairing list`.",
                err
            ),
        ),
    }
    Ok(CommandResult::Handled)
}

fn handle_deny_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    let store = PairingStore::open_default();
    if args.is_empty() || args[0].eq_ignore_ascii_case("list") {
        let entries = store.list().unwrap_or_default();
        let mut out = String::from("Pairing devices (deny/revoke candidates):\n");
        if entries.is_empty() {
            out.push_str("  - none\n");
        } else {
            for dev in entries {
                out.push_str(&format!("  - {} [{}]\n", dev.device_id, dev.status));
            }
        }
        out.push_str("Revoke one with `/deny <device-id>` or purge pending with `/deny pending`.");
        emit_command_output(app, out.trim_end());
        return Ok(CommandResult::Handled);
    }

    if args[0].eq_ignore_ascii_case("pending") || args[0].eq_ignore_ascii_case("clear-pending") {
        match store.clear_pending() {
            Ok(count) => emit_command_output(app, format!("Removed {} pending device(s).", count)),
            Err(err) => {
                emit_command_output(app, format!("Failed clearing pending devices: {}", err))
            }
        }
        return Ok(CommandResult::Handled);
    }

    match store.revoke(args[0]) {
        Ok(dev) => emit_command_output(
            app,
            format!(
                "Revoked device '{}' (name={}).",
                dev.device_id,
                dev.name.unwrap_or_else(|| "unnamed".to_string())
            ),
        ),
        Err(err) => emit_command_output(
            app,
            format!(
                "Deny failed: {}. Use `/deny list` or `hermes pairing list`.",
                err
            ),
        ),
    }
    Ok(CommandResult::Handled)
}

fn handle_copy_command(app: &mut App) -> Result<CommandResult, AgentError> {
    let maybe_text = app.transcript_messages().into_iter().rev().find_map(|msg| {
        if msg.role != hermes_core::MessageRole::Assistant {
            return None;
        }
        let content = msg.content.unwrap_or_default();
        let trimmed = content.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let Some(text) = maybe_text else {
        emit_command_output(
            app,
            "Copy skipped: no assistant message content available yet.",
        );
        return Ok(CommandResult::Handled);
    };

    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone())) {
        Ok(()) => emit_command_output(
            app,
            format!(
                "Copied latest assistant message ({} chars).",
                text.chars().count()
            ),
        ),
        Err(err) => emit_command_output(
            app,
            format!(
                "Clipboard unavailable ({}). Copy directly from transcript as fallback.",
                err
            ),
        ),
    }
    Ok(CommandResult::Handled)
}

fn handle_statusbar_command(app: &mut App) -> Result<CommandResult, AgentError> {
    emit_command_output(
        app,
        "Status bar is always enabled in the current TUI renderer.",
    );
    Ok(CommandResult::Handled)
}

fn parse_toggle_arg(raw: Option<&str>, current: bool) -> Result<bool, &'static str> {
    let Some(raw) = raw else {
        return Ok(!current);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "toggle" => Ok(!current),
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err("Usage: /mouse [on|off|toggle]"),
    }
}

fn handle_mouse_command(app: &mut App, args: &[&str]) -> Result<CommandResult, AgentError> {
    if args.len() >= 2 && args[0].eq_ignore_ascii_case("set") {
        match parse_toggle_arg(args.get(1).copied(), app.mouse_enabled()) {
            Ok(next) => {
                app.set_mouse_enabled(next);
                crate::env_vars::set_var("HERMES_TUI_MOUSE", if next { "1" } else { "0" });
                emit_command_output(
                    app,
                    format!("Mouse interactions: {}", if next { "ON" } else { "OFF" }),
                );
            }
            Err(usage) => emit_command_output(app, usage),
        }
        return Ok(CommandResult::Handled);
    }

    if args.is_empty() || args[0].eq_ignore_ascii_case("status") {
        emit_command_output(
            app,
            format!(
                "Mouse interactions: {} (use `/mouse on` or `/mouse off`)",
                if app.mouse_enabled() { "ON" } else { "OFF" }
            ),
        );
        return Ok(CommandResult::Handled);
    }

    match parse_toggle_arg(args.first().copied(), app.mouse_enabled()) {
        Ok(next) => {
            app.set_mouse_enabled(next);
            crate::env_vars::set_var("HERMES_TUI_MOUSE", if next { "1" } else { "0" });
            emit_command_output(
                app,
                format!("Mouse interactions: {}", if next { "ON" } else { "OFF" }),
            );
        }
        Err(usage) => emit_command_output(app, usage),
    }
    Ok(CommandResult::Handled)
}

#[derive(Debug, Clone, Copy)]
struct CommandCatalogSection {
    title: &'static str,
    hint: &'static str,
    commands: &'static [&'static str],
}

const COMMAND_CATALOG_SECTIONS: &[CommandCatalogSection] = &[
    CommandCatalogSection {
        title: "Core Session",
        hint: "Session lifecycle, snapshots, rollback, and queue controls",
        commands: &[
            "/new",
            "/reset",
            "/retry",
            "/undo",
            "/history",
            "/recap",
            "/context",
            "/title",
            "/branch",
            "/timetravel",
            "/snapshot",
            "/rollback",
            "/queue",
            "/background",
            "/save",
            "/load",
            "/resume",
            "/sessions",
        ],
    },
    CommandCatalogSection {
        title: "Model/Auth",
        hint: "Provider, model, auth, and reasoning controls",
        commands: &[
            "/model",
            "/provider",
            "/auth",
            "/reasoning",
            "/gquota",
            "/qos",
            "/boot",
            "/walkthrough",
        ],
    },
    CommandCatalogSection {
        title: "Objective/Planning",
        hint: "Mission steering, objectives, planning, and simulation",
        commands: &[
            "/objective",
            "/goal",
            "/subgoal",
            "/plan",
            "/ask",
            "/steer",
            "/btw",
            "/simulate",
            "/specpatch",
            "/quorum",
            "/mission",
            "/autopilot",
            "/triage",
            "/subconscious",
        ],
    },
    CommandCatalogSection {
        title: "Tools/Skills/Integrations",
        hint: "Skills, tools, MCP, gateway adapters, and integration health",
        commands: &[
            "/skills",
            "/tools",
            "/toolcards",
            "/toolsets",
            "/plugins",
            "/mcp",
            "/platforms",
            "/integrations",
            "/reload",
            "/reload-mcp",
            "/runbook",
            "/ops",
            "/telemetry",
            "/dashboard",
        ],
    },
    CommandCatalogSection {
        title: "UX/Views",
        hint: "TUI surface controls and visibility toggles",
        commands: &[
            "/skin",
            "/voice",
            "/pet",
            "/image",
            "/mouse",
            "/verbose",
            "/statusbar",
            "/raw",
            "/redraw",
            "/copy",
            "/paste",
            "/commands",
            "/help",
            "/quit",
        ],
    },
];

fn command_catalog_matches_filter(command: &str, description: &str, query: &str) -> bool {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    let cmd = command.to_ascii_lowercase();
    let desc = description.to_ascii_lowercase();
    cmd.contains(&q) || desc.contains(&q.trim_start_matches('/'))
}

fn render_command_catalog(filter: Option<&str>) -> String {
    let query = filter.unwrap_or("").trim();
    let mut seen = HashSet::new();
    let mut out = String::new();
    out.push_str("Hermes Agent Ultra — Slash Command Palette\n");
    out.push_str("==========================================\n");
    if query.is_empty() {
        out.push_str(
            "Tip: type `/` in the composer to open completions and use arrows/Tab/Enter.\n",
        );
        out.push_str("Scoped search: `/commands <term>` (example: `/commands auth`).\n");
    } else {
        let _ = writeln!(out, "Filter: `{}`", query);
    }
    out.push('\n');

    for section in COMMAND_CATALOG_SECTIONS {
        let mut rendered = 0usize;
        for command in section.commands {
            let Some(description) = help_for(command) else {
                continue;
            };
            if !command_catalog_matches_filter(command, description, query) {
                continue;
            }
            if rendered == 0 {
                let _ = writeln!(out, "## {}\n{}\n", section.title, section.hint);
            }
            let _ = writeln!(out, "- `{:<16}` {}", command, description);
            seen.insert(*command);
            rendered += 1;
        }
        if rendered > 0 {
            out.push('\n');
        }
    }

    let mut extras = Vec::new();
    for (command, description) in SLASH_COMMANDS {
        if seen.contains(command) {
            continue;
        }
        if command_catalog_matches_filter(command, description, query) {
            extras.push((*command, *description));
        }
    }
    if !extras.is_empty() {
        out.push_str("## Other\nCommands that are available but not in the primary sections.\n\n");
        extras.sort_by(|a, b| a.0.cmp(b.0));
        for (command, description) in extras {
            let _ = writeln!(out, "- `{:<16}` {}", command, description);
        }
        out.push('\n');
    }
    out.push_str("You can also type plain text to send a normal chat message.");
    out
}

fn handle_commands_catalog_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let query = if args.is_empty() {
        None
    } else if args[0].eq_ignore_ascii_case("search") {
        let rest = args.get(1..).unwrap_or(&[]).join(" ");
        if rest.trim().is_empty() {
            None
        } else {
            Some(rest)
        }
    } else {
        let rest = args.join(" ");
        if rest.trim().is_empty() {
            None
        } else {
            Some(rest)
        }
    };
    emit_command_output(app, render_command_catalog(query.as_deref()));
    Ok(CommandResult::Handled)
}

fn print_help(app: &mut App) {
    emit_command_output(app, render_command_catalog(None));
}

// ---------------------------------------------------------------------------
// CLI subcommand handlers (dispatched from main.rs)
// ---------------------------------------------------------------------------

fn resolve_cli_chat_provider_model(
    config_model: Option<&str>,
    model_override: Option<&str>,
    provider_override: Option<&str>,
) -> Result<String, AgentError> {
    let provider_override = provider_override
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_ascii_lowercase());
    let model_override = model_override.map(str::trim).filter(|v| !v.is_empty());

    let mut current_model = config_model
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("gpt-4o")
        .to_string();

    if let Some(model) = model_override {
        current_model = model.to_string();
    } else if provider_override.is_none() {
        if let Ok(model_env) = std::env::var("HERMES_INFERENCE_MODEL") {
            let model_env = model_env.trim();
            if !model_env.is_empty() {
                current_model = model_env.to_string();
            }
        }
    }
    if let Some(provider) = provider_override.as_deref() {
        if let Some((_, model_name)) = current_model.split_once(':') {
            current_model = format!("{provider}:{}", model_name.trim());
        } else {
            current_model = format!("{provider}:{}", current_model.trim());
        }
    }
    if !current_model.contains(':') {
        current_model = normalize_provider_model(&current_model)?;
    }
    Ok(current_model)
}

fn apply_cli_chat_runtime_env(provider_model: &str) {
    let provider_model = provider_model.trim();
    if provider_model.is_empty() {
        return;
    }
    crate::env_vars::set_var("HERMES_MODEL", provider_model);
    crate::env_vars::set_var("HERMES_INFERENCE_MODEL", provider_model);
    if let Some((provider, _)) = provider_model.split_once(':') {
        let provider = provider.trim();
        if !provider.is_empty() {
            crate::env_vars::set_var("HERMES_INFERENCE_PROVIDER", provider);
            if std::env::var_os("HERMES_TUI_PROVIDER").is_some() {
                crate::env_vars::set_var("HERMES_TUI_PROVIDER", provider);
            }
        }
    }
}

const QUERY_ALLOW_TOOLS_ENV_KEY: &str = "HERMES_QUERY_ALLOW_TOOLS";
const QUERY_DISABLE_TOOLS_ENV_KEY: &str = "HERMES_QUERY_DISABLE_TOOLS";

fn query_mode_tools_enabled(query_mode: bool, allow_tools_flag: bool) -> bool {
    if !query_mode {
        return true;
    }
    if allow_tools_flag {
        return true;
    }
    if hermes_config::env_var_enabled(QUERY_DISABLE_TOOLS_ENV_KEY) {
        return false;
    }
    // Backward compatible explicit-enable override (now redundant with default-on).
    if hermes_config::env_var_enabled(QUERY_ALLOW_TOOLS_ENV_KEY) {
        return true;
    }
    true
}

fn query_mode_model_not_found(err: &hermes_core::AgentError) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    (msg.contains("model") && msg.contains("not found"))
        || msg.contains("requested model does not exist")
        || msg.contains("openrouter catalog")
}

async fn query_mode_remediation_target(provider_model: &str) -> Option<(String, Vec<String>)> {
    let (provider, model_id) = split_provider_model(provider_model);
    let provider = provider.trim().to_ascii_lowercase();
    if provider.is_empty() || model_id.trim().is_empty() {
        return None;
    }
    let catalog = provider_model_ids(&provider).await;
    if catalog.is_empty() {
        return None;
    }
    let close = rank_catalog_model_candidates(model_id.trim(), &catalog, 5);
    let selected = resolve_catalog_model_candidate(model_id.trim(), &catalog)
        .or_else(|| close.first().cloned())
        .or_else(|| catalog.first().cloned())?;
    let next = format!("{}:{}", provider, selected.trim());
    if next.eq_ignore_ascii_case(provider_model) {
        return None;
    }
    Some((next, close))
}

/// Handle `hermes chat [--query ...] [--preload-skill ...] [--yolo]`.
pub async fn handle_cli_chat(
    query: Option<String>,
    preload_skill: Option<String>,
    yolo: bool,
    model_override: Option<String>,
    provider_override: Option<String>,
    allow_tools_flag: bool,
) -> Result<(), hermes_core::AgentError> {
    use crate::runtime_tool_wiring::{wire_cron_scheduler_backend, wire_stdio_clarify_backend};
    use crate::terminal_backend::build_terminal_backend;
    use crate::tool_preview::{build_tool_preview_from_value, tool_emoji};
    use hermes_config::load_config;
    use hermes_core::MessageRole;
    use hermes_cron::cron_scheduler_for_data_dir;
    use hermes_skills::{FileSkillStore, SkillManager};
    use hermes_tools::ToolRegistry;

    if let Some(skill) = &preload_skill {
        println!("[Preloading skill: {}]", skill);
    }
    if yolo {
        println!("[YOLO mode: tool confirmations disabled]");
    }

    let mut config =
        load_config(None).map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;

    if yolo {
        config.approval.require_approval = false;
    }

    let query_mode = query.is_some();
    let tools_enabled = query_mode_tools_enabled(query_mode, allow_tools_flag);
    if query_mode && !tools_enabled {
        println!(
            "[Query mode tools are disabled by {}=1. Unset it or pass --allow-tools to re-enable.]",
            QUERY_DISABLE_TOOLS_ENV_KEY
        );
    }

    let current_model = resolve_cli_chat_provider_model(
        config.model.as_deref(),
        model_override.as_deref(),
        provider_override.as_deref(),
    )?;
    apply_cli_chat_runtime_env(&current_model);

    let tool_registry = Arc::new(ToolRegistry::new());
    let tool_schemas = if tools_enabled {
        let terminal_backend = build_terminal_backend(&config);
        let skill_store = Arc::new(FileSkillStore::new(FileSkillStore::default_dir()));
        let skill_provider: Arc<dyn hermes_core::SkillProvider> =
            Arc::new(SkillManager::new(skill_store));
        hermes_tools::register_builtin_tools(&tool_registry, terminal_backend, skill_provider);
        let live_count =
            crate::live_messaging::enable_live_messaging_tool(&config, &tool_registry).await;
        if live_count > 0 {
            println!(
                "[send_message live delivery enabled via {} configured adapter(s)]",
                live_count
            );
        }
        wire_stdio_clarify_backend(&tool_registry);
        let cron_data_dir = hermes_config::cron_dir();
        std::fs::create_dir_all(&cron_data_dir)
            .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
        let cron_scheduler = Arc::new(cron_scheduler_for_data_dir(cron_data_dir));
        cron_scheduler
            .load_persisted_jobs()
            .await
            .map_err(|e| hermes_core::AgentError::Config(format!("cron load: {e}")))?;
        cron_scheduler.start().await;
        wire_cron_scheduler_backend(
            &tool_registry,
            cron_scheduler,
            MessagingSessionContext::new(),
        );
        crate::platform_toolsets::resolve_platform_tool_schemas(&config, "cli", &tool_registry)
    } else {
        Vec::new()
    };
    let agent_tool_registry = Arc::new(crate::app::bridge_tool_registry(&tool_registry));

    let build_query_agent = |provider_model: &str| {
        let on_tool_start: Box<dyn Fn(&str, &serde_json::Value) + Send + Sync> =
            Box::new(move |name: &str, args: &serde_json::Value| {
                let emoji = tool_emoji(name);
                let preview = build_tool_preview_from_value(name, args, 56).unwrap_or_default();
                if preview.is_empty() {
                    println!("┊ {emoji} {name}");
                } else {
                    println!("┊ {emoji} {name:<16} {preview}");
                }
            });
        let on_tool_complete: Box<dyn Fn(&str, &str) + Send + Sync> =
            Box::new(move |name: &str, result: &str| {
                let mut snippet: String = result.trim().chars().take(96).collect();
                if result.trim().chars().count() > 96 {
                    snippet.push_str("...");
                }
                let emoji = tool_emoji(name);
                if snippet.is_empty() {
                    println!("┊ {emoji} {name:<16} done");
                } else {
                    println!("┊ {emoji} {name:<16} done: {snippet}");
                }
            });
        let callbacks = hermes_agent::AgentCallbacks {
            on_tool_start: Some(on_tool_start),
            on_tool_complete: Some(on_tool_complete),
            ..Default::default()
        };
        let agent_config = crate::app::build_agent_config(&config, provider_model);
        let provider = crate::app::build_provider(&config, provider_model);
        let base =
            hermes_agent::AgentLoop::new(agent_config, Arc::clone(&agent_tool_registry), provider)
                .with_async_tool_dispatch(crate::app::async_tool_dispatch_for(
                    tool_registry.clone(),
                ))
                .with_callbacks(callbacks);
        if query_mode {
            hermes_agent::attach_discovered_plugins(base)
        } else {
            hermes_agent::attach_agent_runtime(base)
        }
    };

    match query {
        Some(q) => {
            let mut active_model = current_model.clone();
            if let Some((next_model, close)) = query_mode_remediation_target(&active_model).await {
                println!(
                    "[Model remediation: {} -> {}. Close matches: {}]",
                    active_model,
                    next_model,
                    if close.is_empty() {
                        "(none)".to_string()
                    } else {
                        close.join(", ")
                    }
                );
                active_model = next_model;
            }
            apply_cli_chat_runtime_env(&active_model);
            let agent = build_query_agent(&active_model);
            let result = match agent
                .run_conversation(RunConversationParams {
                    user_message: q.clone(),
                    conversation_history: vec![],
                    task_id: None,
                    stream_callback: None,
                    persist_user_message: None,
                    tools: Some(tool_schemas.clone()),
                    persist_session: false,
                })
                .await
            {
                Ok(conv) => conv.into_loop_result(),
                Err(err) => {
                    if query_mode_model_not_found(&err) {
                        if let Some((next_model, close)) =
                            query_mode_remediation_target(&active_model).await
                        {
                            return Err(hermes_core::AgentError::Config(format!(
                                "{}\nModel remediation suggestion: {} -> {} (close matches: {})",
                                err,
                                active_model,
                                next_model,
                                if close.is_empty() {
                                    "(none)".to_string()
                                } else {
                                    close.join(", ")
                                }
                            )));
                        }
                    }
                    return Err(err);
                }
            };

            let reply = result
                .messages
                .iter()
                .rev()
                .find_map(|m| {
                    if m.role == MessageRole::Assistant {
                        m.content.clone()
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "(no assistant reply)".to_string());
            println!("{}", reply);
        }
        None => {
            println!("Starting interactive chat session...");
            println!("(Use `hermes` for the default interactive TUI)");
        }
    }
    Ok(())
}

/// Handle `hermes skills [action] [name] [--extra ...]`.
// handle_cli_skills moved to skills.rs

// ---------------------------------------------------------------------------
// Plugin discovery / surface rendering
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PluginSurfaceSource {
    User,
    Project,
    Entrypoint,
}

impl PluginSurfaceSource {
    fn label(&self) -> &'static str {
        match self {
            PluginSurfaceSource::User => "user",
            PluginSurfaceSource::Project => "project",
            PluginSurfaceSource::Entrypoint => "entrypoint",
        }
    }
}

#[derive(Debug, Clone)]
struct PluginSurfaceEntry {
    name: String,
    version: String,
    description: String,
    kind: Option<String>,
    source: PluginSurfaceSource,
    path: Option<PathBuf>,
    enabled: bool,
    entrypoint_value: Option<String>,
    entrypoint_dist: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PythonEntrypointPayload {
    #[serde(default)]
    entries: Vec<PythonEntrypointItem>,
}

#[derive(Debug, Deserialize)]
struct PythonEntrypointItem {
    name: String,
    value: String,
    #[serde(default)]
    dist: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PythonPluginCommandPayload {
    #[serde(default)]
    commands: Vec<PythonPluginCommandItem>,
}

#[derive(Debug, Deserialize, Clone)]
struct PythonPluginCommandItem {
    name: String,
    #[serde(default)]
    help: String,
}

fn coerce_memory_provider_kind(path: &Path, kind: Option<String>) -> Option<String> {
    let explicit_kind = kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    if explicit_kind.is_some() {
        return explicit_kind;
    }
    let init_file = path.join("__init__.py");
    let Ok(source) = std::fs::read_to_string(&init_file) else {
        return None;
    };
    let probe = if source.len() > 8192 {
        &source[..8192]
    } else {
        source.as_str()
    };
    if probe.contains("register_memory_provider") || probe.contains("MemoryProvider") {
        Some("exclusive".to_string())
    } else {
        None
    }
}

fn scan_plugin_manifest_root(root: &Path, source: PluginSurfaceSource) -> Vec<PluginSurfaceEntry> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("plugin.yaml");
        if !manifest_path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match serde_yaml::from_str(&content) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        let disabled_marker = path.join(".disabled");
        out.push(PluginSurfaceEntry {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            kind: coerce_memory_provider_kind(&path, manifest.kind.clone()),
            source,
            path: Some(path),
            enabled: !disabled_marker.exists(),
            entrypoint_value: None,
            entrypoint_dist: None,
        });
    }
    out
}

fn discover_python_entrypoint_plugins() -> Vec<PluginSurfaceEntry> {
    let script = r#"
import json
from importlib import metadata

def _entry_points():
    eps = metadata.entry_points()
    if hasattr(eps, "select"):
        return list(eps.select(group="hermes_agent.plugins"))
    if isinstance(eps, dict):
        return list(eps.get("hermes_agent.plugins", []))
    return [ep for ep in eps if getattr(ep, "group", "") == "hermes_agent.plugins"]

rows = []
try:
    for ep in _entry_points():
        dist = None
        try:
            if getattr(ep, "dist", None):
                dist = ep.dist.name
        except Exception:
            dist = None
        rows.append({
            "name": str(getattr(ep, "name", "") or ""),
            "value": str(getattr(ep, "value", "") or ""),
            "dist": dist,
        })
except Exception:
    rows = []
print(json.dumps({"entries": rows}))
"#;

    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let payload: PythonEntrypointPayload = match serde_json::from_slice(&output.stdout) {
        Ok(payload) => payload,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in payload.entries {
        let name = item.name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.push(PluginSurfaceEntry {
            name,
            version: "entrypoint".to_string(),
            description: String::new(),
            kind: None,
            source: PluginSurfaceSource::Entrypoint,
            path: None,
            enabled: true,
            entrypoint_value: Some(item.value),
            entrypoint_dist: item.dist,
        });
    }
    out
}

fn discover_plugin_surface(include_entrypoints: bool) -> Vec<PluginSurfaceEntry> {
    let mut rows = Vec::new();
    let user_root = hermes_config::hermes_home().join("plugins");
    rows.extend(scan_plugin_manifest_root(
        &user_root,
        PluginSurfaceSource::User,
    ));

    if hermes_config::env_var_enabled("HERMES_ENABLE_PROJECT_PLUGINS") {
        if let Ok(cwd) = std::env::current_dir() {
            let project_root = hermes_config::project_hermes_dir(&cwd).join("plugins");
            rows.extend(scan_plugin_manifest_root(
                &project_root,
                PluginSurfaceSource::Project,
            ));
        }
    }

    if include_entrypoints {
        rows.extend(discover_python_entrypoint_plugins());
    }

    rows.sort_by(|a, b| {
        a.source.cmp(&b.source).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    rows
}

fn resolve_local_plugin_path_by_name(name: &str) -> Option<PathBuf> {
    discover_plugin_surface(false)
        .into_iter()
        .filter_map(|row| {
            if row.name.eq_ignore_ascii_case(name) {
                row.path
            } else {
                None
            }
        })
        .next()
}

fn render_plugin_surface_table(rows: &[PluginSurfaceEntry]) -> String {
    if rows.is_empty() {
        return "  (no plugins discovered)".to_string();
    }
    let mut out = String::new();
    for row in rows {
        let status = if row.enabled { "enabled" } else { "disabled" };
        let mut meta_parts = vec![format!("source={}", row.source.label())];
        if let Some(kind) = row.kind.as_deref().filter(|k| !k.trim().is_empty()) {
            meta_parts.push(format!("kind={}", kind));
        }
        if let Some(dist) = row
            .entrypoint_dist
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            meta_parts.push(format!("dist={}", dist));
        }
        if let Some(value) = row
            .entrypoint_value
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            meta_parts.push(format!("entry={}", value));
        }
        let path = row
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        let version = if row.version.trim().is_empty() {
            "unknown".to_string()
        } else {
            row.version.clone()
        };
        let description = row.description.trim();
        let _ = writeln!(
            out,
            "  • {} v{} [{}; {}; path={}]",
            row.name,
            version,
            status,
            meta_parts.join(", "),
            path
        );
        if !description.is_empty() {
            let _ = writeln!(out, "    {}", description);
        }
    }
    out.trim_end().to_string()
}

fn set_plugin_enabled(path: &Path, enable: bool) -> Result<(), AgentError> {
    let marker = path.join(".disabled");
    if enable {
        if marker.exists() {
            std::fs::remove_file(&marker)
                .map_err(|e| AgentError::Io(format!("Failed to enable plugin: {}", e)))?;
        }
    } else {
        std::fs::write(&marker, "")
            .map_err(|e| AgentError::Io(format!("Failed to disable plugin: {}", e)))?;
    }
    Ok(())
}

fn parse_selection_indices(raw: &str, max: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for token in raw.split(|c: char| c == ',' || c.is_ascii_whitespace()) {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(idx) = trimmed.parse::<usize>() else {
            continue;
        };
        if idx == 0 || idx > max {
            continue;
        }
        out.push(idx - 1);
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn run_plugins_interactive_toggle() -> Result<(), AgentError> {
    let mut rows: Vec<PluginSurfaceEntry> = discover_plugin_surface(false)
        .into_iter()
        .filter(|row| row.path.is_some())
        .collect();
    if rows.is_empty() {
        println!("No plugin bundles discovered.");
        println!("Install one with: hermes plugins install <owner/repo>  (or a trusted git URL)");
        return Ok(());
    }

    rows.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });

    println!("Plugin toggle UI (interactive)");
    println!("------------------------------");
    println!("Source roots:");
    println!(
        "  - user:    {}",
        hermes_config::hermes_home().join("plugins").display()
    );
    if hermes_config::env_var_enabled("HERMES_ENABLE_PROJECT_PLUGINS") {
        if let Ok(cwd) = std::env::current_dir() {
            println!(
                "  - project: {}",
                hermes_config::project_hermes_dir(&cwd)
                    .join("plugins")
                    .display()
            );
        }
    } else {
        println!("  - project: disabled (set HERMES_ENABLE_PROJECT_PLUGINS=true)");
    }
    println!();

    let mut provider_indices = Vec::new();
    println!("General Plugins");
    for (idx, row) in rows.iter().enumerate() {
        let is_provider = row.kind.as_deref() == Some("exclusive");
        if is_provider {
            provider_indices.push(idx);
            continue;
        }
        let mark = if row.enabled { "✓" } else { " " };
        println!(
            "  {:>2}. [{}] {} (source={})",
            idx + 1,
            mark,
            row.name,
            row.source.label()
        );
    }

    if !provider_indices.is_empty() {
        println!();
        println!("Provider Plugins (single-select recommended)");
        for idx in &provider_indices {
            let row = &rows[*idx];
            let mark = if row.enabled { "✓" } else { " " };
            println!(
                "  {:>2}. [{}] {} (source={}, kind={})",
                idx + 1,
                mark,
                row.name,
                row.source.label(),
                row.kind.clone().unwrap_or_else(|| "provider".to_string())
            );
        }
    }

    use std::io::Write as _;
    print!("\nToggle plugin numbers (comma/space separated, Enter to skip): ");
    let _ = std::io::stdout().flush();
    let mut toggle_buf = String::new();
    std::io::stdin()
        .read_line(&mut toggle_buf)
        .map_err(|e| AgentError::Io(format!("Failed to read selection: {}", e)))?;
    let toggle_indices = parse_selection_indices(&toggle_buf, rows.len());
    for idx in toggle_indices {
        if let Some(path) = rows[idx].path.as_deref() {
            let target = !rows[idx].enabled;
            set_plugin_enabled(path, target)?;
            rows[idx].enabled = target;
        }
    }

    if !provider_indices.is_empty() {
        print!("Activate exactly one provider plugin number (Enter to keep current): ");
        let _ = std::io::stdout().flush();
        let mut provider_buf = String::new();
        std::io::stdin()
            .read_line(&mut provider_buf)
            .map_err(|e| AgentError::Io(format!("Failed to read provider selection: {}", e)))?;
        let selected = parse_selection_indices(&provider_buf, rows.len());
        if let Some(selected_idx) = selected.first().copied() {
            if provider_indices.contains(&selected_idx) {
                for idx in provider_indices {
                    if let Some(path) = rows[idx].path.as_deref() {
                        let should_enable = idx == selected_idx;
                        set_plugin_enabled(path, should_enable)?;
                        rows[idx].enabled = should_enable;
                    }
                }
            } else {
                println!(
                    "Selection {} is not a provider plugin row; keeping provider state unchanged.",
                    selected_idx + 1
                );
            }
        }
    }

    println!("\nUpdated plugin state:");
    println!("{}", render_plugin_surface_table(&rows));
    Ok(())
}

fn discover_python_plugin_cli_commands() -> Vec<PythonPluginCommandItem> {
    let script = r#"
import json
rows = []
try:
    from plugins.memory import discover_plugin_cli_commands
    for cmd in (discover_plugin_cli_commands() or []):
        name = str(cmd.get("name", "") or "").strip()
        if not name:
            continue
        help_text = str(cmd.get("help") or cmd.get("description") or "")
        rows.append({"name": name, "help": help_text})
except Exception:
    rows = []
print(json.dumps({"commands": rows}))
"#;
    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let payload: PythonPluginCommandPayload = match serde_json::from_slice(&output.stdout) {
        Ok(payload) => payload,
        Err(_) => return Vec::new(),
    };
    let mut rows = payload.commands;
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows.dedup_by(|a, b| a.name == b.name);
    rows
}

pub async fn handle_cli_external_plugin_subcommand(raw: Vec<String>) -> Result<(), AgentError> {
    if raw.is_empty() {
        return Err(AgentError::Config(
            "Unknown command. Run `hermes --help` for available commands.".to_string(),
        ));
    }
    let command_name = raw[0].trim().to_string();
    let command_args: Vec<String> = raw[1..].to_vec();
    let available = discover_python_plugin_cli_commands();
    if !available.iter().any(|row| row.name == command_name) {
        let catalog = if available.is_empty() {
            "none discovered".to_string()
        } else {
            available
                .iter()
                .map(|row| {
                    if row.help.trim().is_empty() {
                        format!("  - {}", row.name)
                    } else {
                        format!("  - {}: {}", row.name, row.help.trim())
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        return Err(AgentError::Config(format!(
            "Unknown command '{}'. Run `hermes --help` for core commands.\nDiscovered plugin commands:\n{}",
            command_name, catalog
        )));
    }

    let args_json = serde_json::to_string(&command_args)
        .map_err(|e| AgentError::Config(format!("Failed to serialize plugin CLI args: {}", e)))?;
    let script = r#"
import argparse
import json
import sys

try:
    from plugins.memory import discover_plugin_cli_commands
except Exception as exc:
    print(f"Plugin CLI bridge unavailable: {exc}", file=sys.stderr)
    sys.exit(2)

name = sys.argv[1]
argv = json.loads(sys.argv[2])

for item in (discover_plugin_cli_commands() or []):
    if str(item.get("name", "")).strip() != name:
        continue
    setup = item.get("setup_fn")
    if not callable(setup):
        print(f"Plugin command '{name}' is missing setup_fn", file=sys.stderr)
        sys.exit(2)
    parser = argparse.ArgumentParser(prog=name)
    setup(parser)
    ns = parser.parse_args(argv)
    handler = item.get("handler_fn")
    if callable(handler):
        handler(ns)
        sys.exit(0)
    if hasattr(ns, "func") and callable(getattr(ns, "func")):
        ns.func(ns)
        sys.exit(0)
    parser.print_help()
    sys.exit(0)

print(f"Unknown plugin command: {name}", file=sys.stderr)
sys.exit(3)
"#;

    let output = tokio::process::Command::new("python3")
        .args(["-c", script, &command_name, &args_json])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|e| AgentError::Io(format!("Failed to execute plugin command: {}", e)))?;
    if !output.success() {
        return Err(AgentError::Config(format!(
            "Plugin command '{}' failed with exit code {:?}.",
            command_name,
            output.code()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Plugin security (remote Git installs)
// ---------------------------------------------------------------------------

fn default_git_host_allowlist() -> Vec<&'static str> {
    vec![
        "github.com",
        "www.github.com",
        "raw.githubusercontent.com",
        "gitlab.com",
        "www.gitlab.com",
        "codeberg.org",
        "www.codeberg.org",
        "gitea.com",
        "bitbucket.org",
    ]
}

fn plugin_git_host_allowed(url: &str, allow_untrusted: bool) -> bool {
    if allow_untrusted {
        return true;
    }
    let extra = std::env::var("HERMES_PLUGIN_GIT_EXTRA_HOSTS").unwrap_or_default();
    let mut hosts: Vec<String> = default_git_host_allowlist()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    for part in extra.split(',') {
        let p = part.trim();
        if !p.is_empty() {
            hosts.push(p.to_lowercase());
        }
    }
    let lower = url.to_lowercase();
    let host_part = if lower.contains("://") {
        lower.split("://").nth(1).unwrap_or("")
    } else if lower.starts_with("git@") {
        lower
            .trim_start_matches("git@")
            .split(':')
            .next()
            .unwrap_or("")
    } else {
        return false;
    };
    let host = host_part
        .split('/')
        .next()
        .unwrap_or(host_part)
        .split('@')
        .last()
        .unwrap_or(host_part);
    let host = host.split(':').next().unwrap_or(host).to_lowercase();
    hosts
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{}", h)))
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// Static scan of a cloned plugin tree: risky patterns in scripts/config.
fn scan_plugin_security(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let manifest = root.join("plugin.yaml");
    if manifest.exists() {
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if text.contains("post_install") || text.contains("postInstall") {
                out.push(
                    "plugin.yaml declares post_install / postInstall — review before running the plugin"
                        .into(),
                );
            }
            if Regex::new(r"(?i)curl\s+[^|\n]*\|\s*(ba)?sh")
                .ok()
                .and_then(|re| re.find(&text))
                .is_some()
            {
                out.push("plugin.yaml references curl|sh style install — high risk".into());
            }
        }
    }

    let risky_file_patterns: &[(&str, &[(&str, &str)])] = &[(
        r"\.(sh|bash|zsh|py|rb|ps1|fish)$",
        &[
            (r"(?i)\bcurl\s+[^|\n]*\|\s*(ba)?sh", "curl piped to shell"),
            (r"(?i)\bwget\s+[^|\n]*\|\s*(ba)?sh", "wget piped to shell"),
            (r"(?i)\beval\s*\(", "eval("),
            (r"(?i)\bexec\s*\(", "exec("),
            (r"(?i)(base64[._-]?decode|atob)\s*\(", "base64 decode"),
            (r"(?i)\brm\s+-rf\s+/", "rm -rf on absolute path"),
        ],
    )];

    fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir.is_dir() && (name == ".git" || name == "target" || name == "node_modules") {
            return;
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, files);
                } else if p.is_file() {
                    files.push(p);
                }
            }
        }
    }

    let mut files = Vec::new();
    walk(root, &mut files);

    for fp in files {
        let fname = fp.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname == ".DS_Store" {
            continue;
        }
        let rel = fp.strip_prefix(root).unwrap_or(&fp).display().to_string();
        let Ok(content) = std::fs::read_to_string(&fp) else {
            continue;
        };
        for (ext_re, rules) in risky_file_patterns {
            if let Ok(re_ext) = Regex::new(ext_re) {
                if !re_ext.is_match(fname) {
                    continue;
                }
                for (pat, label) in *rules {
                    if let Ok(re) = Regex::new(pat) {
                        if re.is_match(&content) {
                            out.push(format!("{}: {}", rel, label));
                        }
                    }
                }
            }
        }
    }

    out.sort();
    out.dedup();
    out
}

async fn git_checkout_ref(repo_dir: &std::path::Path, git_ref: &str) -> Result<(), String> {
    let dir = repo_dir.to_string_lossy().to_string();
    let fetch = tokio::process::Command::new("git")
        .args(["-C", &dir, "fetch", "--depth", "1", "origin", git_ref])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !fetch.status.success() {
        let err = String::from_utf8_lossy(&fetch.stderr);
        return Err(format!("git fetch origin {}: {}", git_ref, err.trim()));
    }
    let co = tokio::process::Command::new("git")
        .args(["-C", &dir, "checkout", git_ref])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !co.status.success() {
        let err = String::from_utf8_lossy(&co.stderr);
        return Err(format!("git checkout {}: {}", git_ref, err.trim()));
    }
    Ok(())
}

/// Handle `hermes plugins [action] [name]`.
pub async fn handle_cli_plugins(
    action: Option<String>,
    name: Option<String>,
    git_ref: Option<String>,
    allow_untrusted_git_host: bool,
) -> Result<(), hermes_core::AgentError> {
    let plugins_dir = hermes_config::hermes_home().join("plugins");

    match action.as_deref() {
        None => {
            run_plugins_interactive_toggle()?;
        }
        Some("list") => {
            let rows = discover_plugin_surface(true);
            println!("Plugin surface ({} entries):", rows.len());
            println!("{}", render_plugin_surface_table(&rows));
        }
        Some("enable") => {
            let plugin_name = name.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing plugin name. Usage: hermes plugins enable <name>".into(),
                )
            })?;
            let target = resolve_local_plugin_path_by_name(&plugin_name)
                .unwrap_or_else(|| plugins_dir.join(&plugin_name));
            let disabled_marker = target.join(".disabled");
            if disabled_marker.exists() {
                std::fs::remove_file(&disabled_marker).map_err(|e| {
                    hermes_core::AgentError::Io(format!("Failed to enable plugin: {}", e))
                })?;
                println!("Plugin '{}' enabled.", plugin_name);
            } else {
                println!(
                    "Plugin '{}' is already enabled (or not installed).",
                    plugin_name
                );
            }
        }
        Some("disable") => {
            let plugin_name = name.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing plugin name. Usage: hermes plugins disable <name>".into(),
                )
            })?;
            let plugin_dir = resolve_local_plugin_path_by_name(&plugin_name)
                .unwrap_or_else(|| plugins_dir.join(&plugin_name));
            if !plugin_dir.exists() {
                println!("Plugin '{}' not found.", plugin_name);
                return Ok(());
            }
            let disabled_marker = plugin_dir.join(".disabled");
            std::fs::write(&disabled_marker, "").map_err(|e| {
                hermes_core::AgentError::Io(format!("Failed to disable plugin: {}", e))
            })?;
            println!("Plugin '{}' disabled.", plugin_name);
        }
        Some("install") => {
            let plugin_name = name.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing plugin name. Usage: hermes plugins install <name|url>".into(),
                )
            })?;
            println!("Installing plugin: {}...", plugin_name);

            let is_git_url = plugin_name.starts_with("http://")
                || plugin_name.starts_with("https://")
                || plugin_name.starts_with("git@");

            if is_git_url {
                if !plugin_git_host_allowed(&plugin_name, allow_untrusted_git_host) {
                    println!(
                        "  ✗ Git host is not on the default allow-list (github.com, gitlab.com, codeberg.org, …)."
                    );
                    println!(
                        "    Set comma-separated HERMES_PLUGIN_GIT_EXTRA_HOSTS or pass --allow-untrusted-git-host after you trust the source."
                    );
                    return Ok(());
                }
                // Extract repo name from URL for target directory
                let repo_name = plugin_name
                    .trim_end_matches('/')
                    .trim_end_matches(".git")
                    .rsplit('/')
                    .next()
                    .unwrap_or("unknown-plugin")
                    .to_string();

                // Also handle git@ SSH URLs like git@github.com:user/repo.git
                let repo_name = if repo_name.contains(':') {
                    repo_name
                        .rsplit(':')
                        .next()
                        .unwrap_or(&repo_name)
                        .trim_end_matches(".git")
                        .rsplit('/')
                        .next()
                        .unwrap_or(&repo_name)
                        .to_string()
                } else {
                    repo_name
                };

                let target = plugins_dir.join(&repo_name);
                if target.exists() {
                    println!(
                        "Plugin '{}' is already installed at {}",
                        repo_name,
                        target.display()
                    );
                    return Ok(());
                }

                std::fs::create_dir_all(&plugins_dir).map_err(|e| {
                    hermes_core::AgentError::Io(format!("Failed to create plugins dir: {}", e))
                })?;

                println!("  Cloning {} ...", plugin_name);
                let output = tokio::process::Command::new("git")
                    .args([
                        "clone",
                        "--depth",
                        "1",
                        &plugin_name,
                        &target.to_string_lossy(),
                    ])
                    .output()
                    .await
                    .map_err(|e| hermes_core::AgentError::Io(format!("git clone failed: {}", e)))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    println!("  ✗ git clone failed: {}", stderr.trim());
                    return Ok(());
                }

                if let Some(gr) = git_ref.as_deref() {
                    println!("  Checking out ref: {} ...", gr);
                    if let Err(e) = git_checkout_ref(&target, gr).await {
                        println!("  ✗ {}", e);
                        let _ = std::fs::remove_dir_all(&target);
                        return Ok(());
                    }
                }

                // Verify plugin.yaml exists
                let manifest_path = target.join("plugin.yaml");
                if !manifest_path.exists() {
                    println!("  ✗ No plugin.yaml found in cloned repository.");
                    println!("    Removing {}...", target.display());
                    let _ = std::fs::remove_dir_all(&target);
                    return Ok(());
                }

                // Parse and display plugin info
                let manifest_content = std::fs::read_to_string(&manifest_path)
                    .map_err(|e| hermes_core::AgentError::Io(format!("Read error: {}", e)))?;
                let manifest: serde_json::Value =
                    serde_yaml::from_str(&manifest_content).unwrap_or(serde_json::json!({}));

                let p_name = manifest
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&repo_name);
                let p_version = manifest
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let p_desc = manifest
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Security scan of cloned files
                let suspicious = scan_plugin_security(&target);
                let hard_block = suspicious.iter().any(|s| {
                    s.contains("curl piped to shell")
                        || s.contains("wget piped to shell")
                        || s.contains("curl|sh style install")
                });
                if hard_block && !allow_untrusted_git_host {
                    println!("\n  ✗ High-risk install patterns detected — clone removed.");
                    for warning in &suspicious {
                        println!("    - {}", warning);
                    }
                    println!(
                        "\n  If you reviewed the code manually, re-run with --allow-untrusted-git-host."
                    );
                    let _ = std::fs::remove_dir_all(&target);
                    return Ok(());
                }
                if !suspicious.is_empty() {
                    println!("\n  ⚠ Security warnings found ({}):", suspicious.len());
                    for warning in &suspicious {
                        println!("    - {}", warning);
                    }
                    println!("\n  Review the warnings above before enabling this plugin.");
                }

                println!("  ✓ Plugin installed successfully!");
                println!("    Name:        {}", p_name);
                println!("    Version:     {}", p_version);
                println!("    Description: {}", p_desc);
                println!("    Path:        {}", target.display());
            } else if plugin_name.starts_with("gh:") || plugin_name.contains('/') {
                // Convert gh:user/repo or user/repo to a GitHub HTTPS URL
                let repo_path = plugin_name.trim_start_matches("gh:");
                let git_url = format!("https://github.com/{}.git", repo_path);
                let repo_name = repo_path.rsplit('/').next().unwrap_or("unknown-plugin");
                let target = plugins_dir.join(repo_name);
                if target.exists() {
                    println!("Plugin '{}' is already installed.", repo_name);
                    return Ok(());
                }

                std::fs::create_dir_all(&plugins_dir).map_err(|e| {
                    hermes_core::AgentError::Io(format!("Failed to create plugins dir: {}", e))
                })?;

                println!("  Cloning from GitHub: {}", git_url);
                let output = tokio::process::Command::new("git")
                    .args(["clone", "--depth", "1", &git_url, &target.to_string_lossy()])
                    .output()
                    .await
                    .map_err(|e| hermes_core::AgentError::Io(format!("git clone failed: {}", e)))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    println!("  ✗ git clone failed: {}", stderr.trim());
                    return Ok(());
                }

                if let Some(gr) = git_ref.as_deref() {
                    println!("  Checking out ref: {} ...", gr);
                    if let Err(e) = git_checkout_ref(&target, gr).await {
                        println!("  ✗ {}", e);
                        let _ = std::fs::remove_dir_all(&target);
                        return Ok(());
                    }
                }

                let manifest_path = target.join("plugin.yaml");
                if !manifest_path.exists() {
                    println!("  ✗ No plugin.yaml found in cloned repository.");
                    let _ = std::fs::remove_dir_all(&target);
                    return Ok(());
                }

                let manifest_content = std::fs::read_to_string(&manifest_path).unwrap_or_default();
                let manifest: serde_json::Value =
                    serde_yaml::from_str(&manifest_content).unwrap_or(serde_json::json!({}));

                let p_name = manifest
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(repo_name);
                let p_version = manifest
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let p_desc = manifest
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let suspicious = scan_plugin_security(&target);
                let hard_block = suspicious.iter().any(|s| {
                    s.contains("curl piped to shell")
                        || s.contains("wget piped to shell")
                        || s.contains("curl|sh style install")
                });
                if hard_block && !allow_untrusted_git_host {
                    println!("\n  ✗ High-risk install patterns detected — clone removed.");
                    for warning in &suspicious {
                        println!("    - {}", warning);
                    }
                    println!(
                        "\n  If you reviewed the code manually, re-run with --allow-untrusted-git-host."
                    );
                    let _ = std::fs::remove_dir_all(&target);
                    return Ok(());
                }
                if !suspicious.is_empty() {
                    println!("\n  ⚠ Security warnings found ({}):", suspicious.len());
                    for warning in &suspicious {
                        println!("    - {}", warning);
                    }
                }

                println!("  ✓ Plugin installed successfully!");
                println!("    Name:        {}", p_name);
                println!("    Version:     {}", p_version);
                println!("    Description: {}", p_desc);
                println!("    Path:        {}", target.display());
            } else {
                let target = plugins_dir.join(&plugin_name);
                if target.exists() {
                    println!("Plugin '{}' is already installed.", plugin_name);
                    return Ok(());
                }
                // Registry lookup
                println!("  Looking up '{}' in plugin registry...", plugin_name);
                match reqwest::Client::new()
                    .get(&format!(
                        "https://plugins.hermes.run/api/v1/{}",
                        plugin_name
                    ))
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            let version = data
                                .get("version")
                                .and_then(|v| v.as_str())
                                .unwrap_or("latest");
                            let git_url = data.get("git_url").and_then(|v| v.as_str());
                            println!("  Found {} v{}", plugin_name, version);

                            if let Some(url) = git_url {
                                if !plugin_git_host_allowed(url, allow_untrusted_git_host) {
                                    println!(
                                        "  ✗ Registry git_url host is not allow-listed. Use --allow-untrusted-git-host or HERMES_PLUGIN_GIT_EXTRA_HOSTS."
                                    );
                                    return Ok(());
                                }
                                std::fs::create_dir_all(&plugins_dir)
                                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;

                                let output = tokio::process::Command::new("git")
                                    .args(["clone", "--depth", "1", url, &target.to_string_lossy()])
                                    .output()
                                    .await
                                    .map_err(|e| {
                                        hermes_core::AgentError::Io(format!(
                                            "git clone failed: {}",
                                            e
                                        ))
                                    })?;

                                if output.status.success() {
                                    if let Some(gr) = git_ref.as_deref() {
                                        println!("  Checking out ref: {} ...", gr);
                                        if let Err(e) = git_checkout_ref(&target, gr).await {
                                            println!("  ✗ {}", e);
                                            let _ = std::fs::remove_dir_all(&target);
                                            return Ok(());
                                        }
                                    }
                                    let suspicious = scan_plugin_security(&target);
                                    let hard_block = suspicious.iter().any(|s| {
                                        s.contains("curl piped to shell")
                                            || s.contains("wget piped to shell")
                                            || s.contains("curl|sh style install")
                                    });
                                    if hard_block && !allow_untrusted_git_host {
                                        println!("  ✗ High-risk patterns — removed clone.");
                                        let _ = std::fs::remove_dir_all(&target);
                                        return Ok(());
                                    }
                                    if !suspicious.is_empty() {
                                        println!("  ⚠ Security warnings: {}", suspicious.len());
                                        for w in &suspicious {
                                            println!("    - {}", w);
                                        }
                                    }
                                    println!(
                                        "  ✓ Plugin '{}' v{} installed.",
                                        plugin_name, version
                                    );
                                } else {
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    println!("  ✗ Clone failed: {}", stderr.trim());
                                }
                            } else {
                                println!("  No git_url in registry response. Cannot install.");
                            }
                        }
                    }
                    _ => {
                        println!("  Plugin '{}' not found in registry.", plugin_name);
                        println!("  Try installing from a URL or GitHub repo instead:");
                        println!("    hermes plugins install https://github.com/user/repo");
                        println!("    hermes plugins install gh:user/repo");
                    }
                }
            }
        }
        Some("remove") | Some("uninstall") => {
            let plugin_name = name.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing plugin name. Usage: hermes plugins remove <name>".into(),
                )
            })?;
            let target = resolve_local_plugin_path_by_name(&plugin_name)
                .unwrap_or_else(|| plugins_dir.join(&plugin_name));
            if target.exists() {
                std::fs::remove_dir_all(&target).map_err(|e| {
                    hermes_core::AgentError::Io(format!("Failed to remove plugin: {}", e))
                })?;
                println!("Plugin '{}' removed.", plugin_name);
            } else {
                println!("Plugin '{}' not found.", plugin_name);
            }
        }
        Some("update") => {
            let plugin_name = name.as_deref();
            let mut checked = 0u32;
            let mut updated = 0u32;
            if !plugins_dir.exists() {
                println!("No plugins installed.");
                return Ok(());
            }
            if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let dir_name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if let Some(target) = plugin_name {
                        if dir_name != target {
                            continue;
                        }
                    }
                    let manifest = path.join("plugin.yaml");
                    if manifest.exists() {
                        checked += 1;
                        println!("  Checking updates for '{}'...", dir_name);

                        let git_dir = path.join(".git");
                        if !git_dir.exists() {
                            println!("    Skipped: plugin is not a git checkout.");
                            continue;
                        }

                        let path_s = path.to_string_lossy().to_string();
                        let before = tokio::process::Command::new("git")
                            .args(["-C", &path_s, "rev-parse", "HEAD"])
                            .output()
                            .await
                            .map_err(|e| {
                                hermes_core::AgentError::Io(format!(
                                    "git rev-parse failed for {}: {}",
                                    dir_name, e
                                ))
                            })?;
                        if !before.status.success() {
                            let stderr = String::from_utf8_lossy(&before.stderr);
                            println!(
                                "    Skipped: cannot read current revision ({})",
                                stderr.trim()
                            );
                            continue;
                        }
                        let before_sha = String::from_utf8_lossy(&before.stdout).trim().to_string();

                        let pull = tokio::process::Command::new("git")
                            .args(["-C", &path_s, "pull", "--ff-only"])
                            .output()
                            .await
                            .map_err(|e| {
                                hermes_core::AgentError::Io(format!(
                                    "git pull failed for {}: {}",
                                    dir_name, e
                                ))
                            })?;

                        if !pull.status.success() {
                            let stderr = String::from_utf8_lossy(&pull.stderr);
                            println!("    Update failed: {}", stderr.trim());
                            continue;
                        }

                        let after = tokio::process::Command::new("git")
                            .args(["-C", &path_s, "rev-parse", "HEAD"])
                            .output()
                            .await
                            .map_err(|e| {
                                hermes_core::AgentError::Io(format!(
                                    "git rev-parse failed for {} after update: {}",
                                    dir_name, e
                                ))
                            })?;
                        if !after.status.success() {
                            let stderr = String::from_utf8_lossy(&after.stderr);
                            println!(
                                "    Updated but could not read final revision ({})",
                                stderr.trim()
                            );
                            continue;
                        }
                        let after_sha = String::from_utf8_lossy(&after.stdout).trim().to_string();

                        if before_sha == after_sha {
                            println!("    Up to date ({})", short_sha(&after_sha));
                        } else {
                            updated += 1;
                            println!(
                                "    Updated: {} -> {}",
                                short_sha(&before_sha),
                                short_sha(&after_sha)
                            );
                        }
                    }
                }
            }
            if checked == 0 {
                if let Some(n) = plugin_name {
                    println!("Plugin '{}' not found.", n);
                } else {
                    println!("No plugins to update.");
                }
            } else {
                println!("Checked {} plugin(s); updated {}.", checked, updated);
            }
        }
        Some("inspect") | Some("info") => {
            let plugin_name = name.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing plugin name. Usage: hermes plugins inspect <name>".into(),
                )
            })?;
            let surface_rows = discover_plugin_surface(true);
            if let Some(row) = surface_rows
                .iter()
                .find(|row| row.name.eq_ignore_ascii_case(&plugin_name))
            {
                println!("Plugin: {}", row.name);
                println!("Source: {}", row.source.label());
                println!(
                    "Status: {}",
                    if row.enabled { "enabled" } else { "disabled" }
                );
                let version = if row.version.trim().is_empty() {
                    "unknown"
                } else {
                    row.version.as_str()
                };
                println!("Version: {}", version);
                if let Some(kind) = row.kind.as_deref().filter(|k| !k.trim().is_empty()) {
                    println!("Kind: {}", kind);
                }
                if let Some(path) = row.path.as_deref() {
                    println!("Path: {}", path.display());
                }
                if let Some(value) = row
                    .entrypoint_value
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                {
                    println!("Entrypoint: {}", value);
                }
                if let Some(dist) = row
                    .entrypoint_dist
                    .as_deref()
                    .filter(|d| !d.trim().is_empty())
                {
                    println!("Distribution: {}", dist);
                }
                if !row.description.trim().is_empty() {
                    println!("Description: {}", row.description.trim());
                }
            }
            let target = resolve_local_plugin_path_by_name(&plugin_name)
                .unwrap_or_else(|| plugins_dir.join(&plugin_name));
            if !target.exists() {
                if surface_rows
                    .iter()
                    .any(|row| row.name.eq_ignore_ascii_case(&plugin_name))
                {
                    return Ok(());
                }
                println!("Plugin '{}' not found.", plugin_name);
                return Ok(());
            }
            let manifest_path = target.join("plugin.yaml");
            if manifest_path.exists() {
                let content = std::fs::read_to_string(&manifest_path)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                println!("Plugin: {}", plugin_name);
                println!("Path:   {}", target.display());
                let disabled = target.join(".disabled").exists();
                println!("Status: {}", if disabled { "disabled" } else { "enabled" });
                println!("\n--- plugin.yaml ---");
                println!("{}", content);
            } else {
                println!("Plugin '{}' has no plugin.yaml manifest.", plugin_name);
            }
        }
        Some(other) => {
            println!("Plugins action '{}' is not recognized.", other);
            println!("Available: list, install, remove, enable, disable, update, inspect");
        }
    }
    Ok(())
}

/// Handle `hermes memory [action]`.
pub async fn handle_cli_memory(
    action: Option<String>,
    target: Option<String>,
    yes: bool,
) -> Result<(), hermes_core::AgentError> {
    let hermes_home = hermes_config::hermes_home();
    let memories_dir = hermes_home.join("memories");
    let memory_md = memories_dir.join("MEMORY.md");
    let user_md = memories_dir.join("USER.md");
    let legacy_memory_db = hermes_home.join("memory.db");
    let disabled_marker = hermes_home.join(".memory_disabled");

    match action.as_deref().unwrap_or("status") {
        "status" => {
            if disabled_marker.exists() {
                println!("Memory provider: disabled");
                println!("  Marker: {}", disabled_marker.display());
                println!("Run `hermes memory setup` to re-enable.");
                return Ok(());
            }

            if memory_md.exists() || user_md.exists() {
                let mem_size = std::fs::metadata(&memory_md).map(|m| m.len()).unwrap_or(0);
                let user_size = std::fs::metadata(&user_md).map(|m| m.len()).unwrap_or(0);
                println!("Memory provider: files (MEMORY.md + USER.md)");
                println!("  Directory: {}", memories_dir.display());
                println!(
                    "  MEMORY.md: {} ({:.1} KB)",
                    memory_md.display(),
                    mem_size as f64 / 1024.0
                );
                println!(
                    "  USER.md:   {} ({:.1} KB)",
                    user_md.display(),
                    user_size as f64 / 1024.0
                );
                if legacy_memory_db.exists() {
                    println!(
                        "  Legacy file detected (unused by current memory backend): {}",
                        legacy_memory_db.display()
                    );
                }
            } else if legacy_memory_db.exists() {
                let size = std::fs::metadata(&legacy_memory_db)
                    .map(|m| m.len())
                    .unwrap_or(0);
                println!("Memory provider: legacy sqlite artifact only");
                println!("  File: {}", legacy_memory_db.display());
                println!("  Size: {} KB", size / 1024);
                println!("Run `hermes memory setup` to initialize the current file backend.");
            } else {
                println!("Memory provider: not configured");
                println!("Run `hermes memory setup` to initialize.");
            }
        }
        "setup" => {
            println!("Memory Provider Setup");
            println!("---------------------");
            std::fs::create_dir_all(&memories_dir)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            if !memory_md.exists() {
                std::fs::write(
                    &memory_md,
                    "# Hermes MEMORY\n\nStore durable assistant memory entries here.\n",
                )
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            }
            if !user_md.exists() {
                std::fs::write(
                    &user_md,
                    "# Hermes USER\n\nStore durable user profile entries here.\n",
                )
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            }
            if disabled_marker.exists() {
                let _ = std::fs::remove_file(&disabled_marker);
            }
            println!("Initialized file memory backend.");
            println!("  MEMORY.md: {}", memory_md.display());
            println!("  USER.md:   {}", user_md.display());
            println!("Memory is enabled for subsequent sessions.");
        }
        "off" => {
            std::fs::create_dir_all(&hermes_home)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            std::fs::write(
                &disabled_marker,
                format!(
                    "disabled_at={}\nreason=hermes memory off\n",
                    chrono::Utc::now().to_rfc3339()
                ),
            )
            .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            println!("Memory provider disabled.");
            println!("  Marker: {}", disabled_marker.display());
            println!("Run `hermes memory setup` to re-enable.");
        }
        "reset" => {
            if !yes {
                return Err(hermes_core::AgentError::Config(
                    "memory reset requires confirmation flag: use `hermes memory reset [all|memory|user] -y`"
                        .into(),
                ));
            }
            let reset_target = target
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("all")
                .to_ascii_lowercase();
            let reset_memory = reset_target == "all" || reset_target == "memory";
            let reset_user = reset_target == "all" || reset_target == "user";
            if !reset_memory && !reset_user {
                return Err(hermes_core::AgentError::Config(format!(
                    "Unknown memory reset target '{}'. Use all|memory|user",
                    reset_target
                )));
            }
            std::fs::create_dir_all(&memories_dir)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            if reset_memory && memory_md.exists() {
                let _ = std::fs::remove_file(&memory_md);
            }
            if reset_user && user_md.exists() {
                let _ = std::fs::remove_file(&user_md);
            }
            if reset_target == "all" && legacy_memory_db.exists() {
                let _ = std::fs::remove_file(&legacy_memory_db);
            }
            if disabled_marker.exists() {
                let _ = std::fs::remove_file(&disabled_marker);
            }
            if reset_memory {
                std::fs::write(
                    &memory_md,
                    "# Hermes MEMORY\n\nStore durable assistant memory entries here.\n",
                )
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            }
            if reset_user {
                std::fs::write(
                    &user_md,
                    "# Hermes USER\n\nStore durable user profile entries here.\n",
                )
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            }
            println!(
                "Memory reset complete (target={}). MEMORY.md={} USER.md={}",
                reset_target,
                if memory_md.exists() {
                    "present"
                } else {
                    "absent"
                },
                if user_md.exists() {
                    "present"
                } else {
                    "absent"
                }
            );
        }
        other => {
            println!("Unknown memory action '{}'.", other);
            println!("Available actions: status, setup, off, reset");
        }
    }
    Ok(())
}

/// Handle `hermes interest [list|status|clear|enable|preview|reject|pin]`.
pub async fn handle_cli_interest(
    action: Option<String>,
    mode: Option<String>,
    llm_on_session_end: bool,
    rest: Vec<String>,
) -> Result<(), hermes_core::AgentError> {
    let config = hermes_config::load_config(None).unwrap_or_default();
    let hermes_home = config
        .home_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(hermes_config::hermes_home);
    let db_path = hermes_home.join("interest.db");

    match action.as_deref().unwrap_or("list") {
        "status" | "list" => {
            if !config.interest.enabled {
                println!("User interest (POI): disabled in config (interest.enabled = false)");
                return Ok(());
            }
            println!("  Pipeline: Extract → Compare → Update (session-end commit)");
            println!("  Extract mode: {}", config.interest.extract_mode);
            println!(
                "  Per-turn buffer / persist: {} / {}",
                config.interest.per_turn_buffer, config.interest.per_turn_persist
            );
            println!(
                "  Session-end LLM: {}",
                if config.interest.session_end_llm_enabled() {
                    "on"
                } else {
                    "off"
                }
            );
            if !db_path.exists() {
                println!("User interest (POI): no topics yet");
                println!("  Database: {}", db_path.display());
                println!("  Topics are learned from conversations when interest.enabled is true.");
                return Ok(());
            }
            let store = hermes_agent::InterestStore::open(&db_path, config.interest.clone())
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            let topics = store
                .list_for_cli(true)
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            println!("User interest (POI): {} topic(s)", topics.len());
            println!("  Database: {}", db_path.display());
            for (idx, topic) in topics.iter().enumerate() {
                let pin = if topic.pinned { " pinned" } else { "" };
                println!(
                    "  {:>2}. [{:.2}] ({}{}) {} — {}",
                    idx + 1,
                    topic.weight,
                    topic.status.as_str(),
                    pin,
                    topic.label,
                    topic.summary
                );
                if !topic.tags.is_empty() {
                    println!("      tags: {}", topic.tags.join(", "));
                }
                println!("      id: {}", topic.id);
            }
        }
        "clear" => {
            if db_path.exists() {
                std::fs::remove_file(&db_path)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            }
            println!("Cleared interest store at {}", db_path.display());
        }
        "prune" => {
            if !db_path.exists() {
                println!("Nothing to prune (no interest.db).");
                return Ok(());
            }
            let store = hermes_agent::InterestStore::open(&db_path, config.interest.clone())
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            let removed = store
                .prune_rejected_topics()
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            println!(
                "Pruned {removed} non-POI topic row(s) from {}",
                db_path.display()
            );
        }
        "enable" => {
            let cfg_path = hermes_config::config_path();
            let mut disk = hermes_config::load_user_config_file(&cfg_path)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            disk.interest.enabled = true;
            disk.interest.per_turn_buffer = true;
            disk.interest.per_turn_persist = false;
            if let Some(m) = mode.as_deref() {
                let m = m.trim().to_ascii_lowercase();
                if matches!(m.as_str(), "rules" | "hybrid" | "llm") {
                    disk.interest.extract_mode = m;
                } else {
                    return Err(hermes_core::AgentError::Config(format!(
                        "interest --mode must be rules, hybrid, or llm (got {m})"
                    )));
                }
            }
            if llm_on_session_end {
                disk.interest.llm_on_session_end = true;
            }
            hermes_config::save_config_yaml(&cfg_path, &disk)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            println!("User interest (POI) extraction enabled (interest.enabled = true).");
            println!("  Extract mode: {}", disk.interest.extract_mode);
            println!(
                "  Session-end LLM: {}",
                if disk.interest.session_end_llm_enabled() {
                    "on"
                } else {
                    "off"
                }
            );
            println!("  Per-turn: buffer only (persist at session end)");
            println!("  Config: {}", cfg_path.display());
            if disk.interest.session_end_llm_enabled() {
                println!("  Note: user messages may be sent to the auxiliary LLM at session end.");
            }
        }
        "preview" => {
            use hermes_agent::{ExtractOptions, extract_signals_from_text};
            let sample = if rest.is_empty() {
                "Help me continue the Rust parity port in crates/hermes-parity-tests".to_string()
            } else {
                rest.join(" ")
            };
            let raw = extract_signals_from_text(&sample, 1.0, ExtractOptions::default());
            let filtered = hermes_agent::filter_persistable_signals(raw);
            println!("POI extract preview (not persisted):");
            println!("  Sample: {sample}");
            if filtered.is_empty() {
                println!("  No persistable signals after quality gate.");
            } else {
                for sig in &filtered {
                    println!(
                        "  - [{}] {} (conf {:.2}, Δweight {:.2})",
                        sig.source().as_str(),
                        sig.label,
                        sig.confidence,
                        sig.weight_delta
                    );
                }
            }
        }
        "reject" => {
            let topic_id = rest.first().map(String::as_str).ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "usage: hermes interest reject <topic-id>".to_string(),
                )
            })?;
            let store = hermes_agent::InterestStore::open(&db_path, config.interest.clone())
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            let ok = store
                .set_topic_status(topic_id, hermes_agent::TopicStatus::Rejected)
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            if ok {
                println!("Rejected topic {topic_id}");
            } else {
                println!("Topic not found: {topic_id}");
            }
        }
        "pin" => {
            let topic_id = rest.first().map(String::as_str).ok_or_else(|| {
                hermes_core::AgentError::Config("usage: hermes interest pin <topic-id>".to_string())
            })?;
            let store = hermes_agent::InterestStore::open(&db_path, config.interest.clone())
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            let ok = store
                .pin_topic(topic_id)
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            if ok {
                println!("Pinned topic {topic_id} (active, always shown in prompt)");
            } else {
                println!("Topic not found: {topic_id}");
            }
        }
        "disable" => {
            let cfg_path = hermes_config::config_path();
            let mut disk = hermes_config::load_user_config_file(&cfg_path)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            disk.interest.enabled = false;
            hermes_config::save_config_yaml(&cfg_path, &disk)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            println!("User interest (POI) extraction disabled (interest.enabled = false).");
            println!("  Existing topics remain in {}", db_path.display());
            println!("  Config: {}", cfg_path.display());
        }
        other => {
            println!("Unknown interest action '{}'.", other);
            println!(
                "Available actions: list, status, clear, prune, enable, disable, preview, reject, pin"
            );
            println!("  enable flags: --mode rules|hybrid|llm  --llm-on-session-end");
        }
    }
    Ok(())
}

fn hermes_home_from_config(config: &hermes_config::GatewayConfig) -> std::path::PathBuf {
    config
        .home_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(hermes_config::hermes_home)
}

/// Handle `hermes contribute [status|enable|disable|preview|flush|reset|revoke]`.
pub async fn handle_cli_contribute(
    action: Option<String>,
    poi_only: bool,
    skills_only: bool,
    _last_session: bool,
    outbox_clear: bool,
) -> Result<(), hermes_core::AgentError> {
    let config = hermes_config::load_config(None).unwrap_or_default();
    let hermes_home = hermes_home_from_config(&config);
    let contribution = config.insights.contribution.clone();

    match action.as_deref().unwrap_or("status") {
        "status" | "list" => {
            println!("Insights contribution (domain work packages → ops server)");
            println!("  Master enabled: {}", contribution.enabled);
            println!("  On session end: {}", contribution.on_session_end);
            println!("  Min evidence tier: {}", contribution.min_evidence_tier);
            println!(
                "  Require skill binding: {}",
                contribution.require_skill_binding
            );
            println!("  Min work turns: {}", contribution.min_work_turns);
            println!("  Redacted body: {}", contribution.redacted_body);
            println!(
                "  Endpoint: {}",
                if contribution.endpoint.trim().is_empty() {
                    "(not set — outbox only)".to_string()
                } else {
                    contribution.endpoint.clone()
                }
            );
            let auth_set = contribution.effective_token().is_some();
            println!(
                "  Authorization (Bearer): {}",
                if auth_set {
                    "(configured)".to_string()
                } else {
                    "(not set — required for upload)".to_string()
                }
            );
            println!("  Upload ready: {}", contribution.upload_ready());
            let svc = hermes_insights::ContributionService::open(
                hermes_home.clone(),
                contribution.clone(),
            )
            .map_err(|e| hermes_core::AgentError::Io(e))?;
            let counts = svc
                .outbox_counts()
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            println!(
                "  Outbox: {} pending, {} failed, {} sent",
                counts.pending, counts.failed, counts.sent
            );
            let install_id = hermes_insights::paths::load_or_create_installation_id(&hermes_home)
                .unwrap_or_else(|_| "(unknown)".to_string());
            println!("  Installation id: {install_id}");
            println!("  Local POI extraction: {}", config.interest.enabled);
        }
        "enable" | "on" => {
            let cfg_path = hermes_config::config_path();
            let mut disk = hermes_config::load_user_config_file(&cfg_path)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            let _ = poi_only;
            let _ = skills_only;
            disk.insights.contribution.enabled = true;
            hermes_config::save_config_yaml(&cfg_path, &disk)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            println!("Insights contribution updated.");
            println!(
                "  Consent version: {}",
                hermes_insights::INSIGHTS_CONSENT_VERSION
            );
            println!("  Upload type: domain_work_package (POI + skill + resolution verdict).");
            println!("  Config: {}", cfg_path.display());
            if disk.insights.contribution.endpoint.trim().is_empty() {
                println!("  Note: set endpoint via:");
                println!("    hermes config set insights.contribution.endpoint <url>");
                println!("    or env HERMES_INSIGHTS_ENDPOINT");
            }
            if disk.insights.contribution.effective_token().is_none() {
                println!(
                    "  Note: server requires Authorization Bearer (user JWT or flowy- API key):"
                );
                println!("    hermes config set insights.contribution.auth_token <jwt-or-api-key>");
                println!("    or export HERMES_INSIGHTS_TOKEN=...");
                println!("    (JWT may be hardcoded in config.yaml for now)");
            }
        }
        "disable" | "off" => {
            let cfg_path = hermes_config::config_path();
            let mut disk = hermes_config::load_user_config_file(&cfg_path)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            let _ = poi_only;
            let _ = skills_only;
            disk.insights.contribution.enabled = false;
            hermes_config::save_config_yaml(&cfg_path, &disk)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            println!(
                "Insights contribution settings saved to {}",
                cfg_path.display()
            );
        }
        "preview" => {
            let svc = hermes_insights::ContributionService::open(
                hermes_home.clone(),
                contribution.clone(),
            )
            .map_err(|e| hermes_core::AgentError::Io(e))?;
            let batch = svc.preview_batch_from_inputs(&[]);
            let json = serde_json::to_string_pretty(&batch)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            println!("{json}");
            println!(
                "\n(preview — run a session with skill_manage + domain task to populate packages)"
            );
        }
        "flush" | "upload" => {
            if contribution.endpoint.trim().is_empty() {
                println!("No insights.contribution.endpoint configured; skipping upload.");
                println!("Pending items remain in the local outbox.");
                return Ok(());
            }
            if contribution.effective_token().is_none() {
                println!("No Authorization Bearer configured; skipping upload.");
                println!(
                    "Set: hermes config set insights.contribution.auth_token <jwt-or-api-key>"
                );
                println!(" or: export HERMES_INSIGHTS_TOKEN=...");
                return Ok(());
            }
            let svc = hermes_insights::ContributionService::open(hermes_home, contribution)
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            match svc.flush().await {
                Ok(result) => {
                    if result.skipped_no_endpoint {
                        println!("Upload skipped (no endpoint).");
                    } else {
                        println!(
                            "Upload complete: {} accepted, {} duplicates, {} rejected",
                            result.uploaded, result.duplicates, result.rejected
                        );
                        if result.duplicates > 0 && result.uploaded == 0 {
                            println!(
                                "  Note: server dedupes by content_hash; rows were not updated."
                            );
                            println!(
                                "  Inspect local payload: ~/.hermes-agent-ultra/insights/last_batch.json"
                            );
                        } else {
                            println!(
                                "  Upload payload saved: ~/.hermes-agent-ultra/insights/last_batch.json"
                            );
                        }
                    }
                }
                Err(e) => {
                    return Err(hermes_core::AgentError::Io(e));
                }
            }
        }
        "revoke" => {
            if contribution.endpoint.trim().is_empty() {
                println!("No endpoint configured; cannot revoke installation on server.");
                return Ok(());
            }
            let svc = hermes_insights::ContributionService::open(hermes_home, contribution)
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            svc.revoke_installation()
                .await
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            println!("Installation revocation request sent to server.");
        }
        "reset" | "requeue" => {
            let svc = hermes_insights::ContributionService::open(hermes_home, contribution)
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            let n = svc
                .reset_outbox(outbox_clear)
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            if outbox_clear {
                println!("Outbox cleared ({n} row(s) deleted).");
            } else {
                println!("Outbox reset: {n} row(s) moved to pending (sent/failed → pending).");
            }
            let counts = svc
                .outbox_counts()
                .map_err(|e| hermes_core::AgentError::Io(e))?;
            println!(
                "  Outbox now: {} pending, {} failed, {} sent",
                counts.pending, counts.failed, counts.sent
            );
            println!("Run `hermes contribute flush` to upload again.");
        }
        other => {
            println!("Unknown contribute action '{}'.", other);
            println!("Available: status, enable, disable, preview, flush, reset, revoke");
            println!("Flags: --poi-only, --skills-only, --clear (with reset)");
        }
    }
    Ok(())
}

/// Handle `hermes mcp [action] [--server ...]`.
pub async fn handle_cli_mcp(
    action: Option<String>,
    name: Option<String>,
    server: Option<String>,
    url: Option<String>,
    command: Option<String>,
    parallel_tools: bool,
) -> Result<(), hermes_core::AgentError> {
    let config_dir = hermes_config::hermes_home();
    let mcp_config_path = config_dir.join("mcp_servers.json");
    let mcp_auth_path = config_dir.join("mcp_auth.json");
    let selected = name.clone().or(server.clone());

    match action.as_deref().unwrap_or("list") {
        "sentrux" | "setup-sentrux" | "sentrux-setup" => {
            let sentrux_present = upsert_sentrux_mcp_profile(&config_dir)?;
            if sentrux_present {
                println!(
                    "Detected '{}' on PATH. Configuring {} MCP profile...",
                    SENTRUX_MCP_COMMAND, SENTRUX_MCP_SERVER_NAME
                );
            } else {
                println!(
                    "Warning: '{}' is not currently on PATH. Adding MCP config anyway.",
                    SENTRUX_MCP_COMMAND
                );
                println!(
                    "Install sentrux, then run `hermes mcp test {}` to verify transport reachability.",
                    SENTRUX_MCP_SERVER_NAME
                );
            }

            println!(
                "Configured MCP server '{}' in:\n  - {}\n  - {}",
                SENTRUX_MCP_SERVER_NAME,
                mcp_config_path.display(),
                config_dir.join("config.yaml").display()
            );
            println!(
                "Runtime hint: use `/mcp` in-session to confirm, and `hermes mcp test {}` for transport checks.",
                SENTRUX_MCP_SERVER_NAME
            );
        }
        "sentrux-status" => {
            let (binary_on_path, from_json, from_yaml) = sentrux_mcp_status(&config_dir);
            println!(
                "Sentrux MCP status:\n  - binary_on_path: {}\n  - in_mcp_servers.json: {}\n  - in_config.yaml: {}",
                if binary_on_path { "yes" } else { "no" },
                yes_no(from_json),
                yes_no(from_yaml)
            );
        }
        "sentrux-remove" => {
            remove_sentrux_mcp_profile(&config_dir)?;
            println!(
                "Removed '{}' MCP profile from JSON + YAML config surfaces.",
                SENTRUX_MCP_SERVER_NAME
            );
        }
        "list" => {
            if !mcp_config_path.exists() {
                println!("No MCP servers configured ({})", mcp_config_path.display());
                println!("Add one with `hermes mcp add --server <name-or-url>`.");
                return Ok(());
            }
            let content = std::fs::read_to_string(&mcp_config_path)
                .map_err(|e| hermes_core::AgentError::Io(format!("Read error: {}", e)))?;
            let servers: serde_json::Value =
                serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
            if let Some(obj) = servers.as_object() {
                if obj.is_empty() {
                    println!("No MCP servers configured.");
                } else {
                    println!("MCP servers ({}):", mcp_config_path.display());
                    for (name, cfg) in obj {
                        let url = cfg.get("url").and_then(|v| v.as_str()).unwrap_or("(stdio)");
                        let parallel = cfg
                            .get("supports_parallel_tool_calls")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        println!(
                            "  • {} — {}  [parallel_tool_calls:{}]",
                            name,
                            url,
                            if parallel { "on" } else { "off" }
                        );
                    }
                }
            }
        }
        "add" => {
            let (entry_name, entry, yaml_command, yaml_url, yaml_parallel) = if let Some(name) =
                name.as_deref().map(str::trim).filter(|s| !s.is_empty())
            {
                let entry = if let Some(url) = url.clone().filter(|v| !v.trim().is_empty()) {
                    serde_json::json!({
                        "url": url,
                        "enabled": true,
                        "supports_parallel_tool_calls": parallel_tools
                    })
                } else if let Some(command) = command.clone().filter(|v| !v.trim().is_empty()) {
                    serde_json::json!({
                        "command": command,
                        "enabled": true,
                        "supports_parallel_tool_calls": parallel_tools
                    })
                } else {
                    return Err(hermes_core::AgentError::Config(
                        "mcp add with positional name requires --url or --command".into(),
                    ));
                };
                (
                    name.to_string(),
                    entry,
                    command.clone().filter(|v| !v.trim().is_empty()),
                    url.clone().filter(|v| !v.trim().is_empty()),
                    parallel_tools,
                )
            } else {
                let srv = server
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        hermes_core::AgentError::Config(
                            "Missing server. Usage: hermes mcp add <name> --url <url> | --command <cmd> [--parallel-tools] (legacy: --server <name-or-url>)".into(),
                        )
                    })?;
                let (entry, yaml_url) = if srv.starts_with("http://") || srv.starts_with("https://")
                {
                    (
                        serde_json::json!({
                            "url": srv,
                            "enabled": true,
                            "supports_parallel_tool_calls": parallel_tools
                        }),
                        Some(srv.to_string()),
                    )
                } else {
                    (
                        serde_json::json!({
                            "url": srv,
                            "enabled": true,
                            "supports_parallel_tool_calls": parallel_tools
                        }),
                        Some(srv.to_string()),
                    )
                };
                (srv.to_string(), entry, None, yaml_url, parallel_tools)
            };
            println!("Adding MCP server: {}", entry_name);
            let mut servers: serde_json::Value = if mcp_config_path.exists() {
                let content = std::fs::read_to_string(&mcp_config_path)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            if let Some(obj) = servers.as_object_mut() {
                obj.insert(entry_name.clone(), entry);
            }
            let json = serde_json::to_string_pretty(&servers)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            std::fs::write(&mcp_config_path, json)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            update_yaml_mcp_server(
                &config_dir,
                &entry_name,
                yaml_command,
                yaml_url,
                yaml_parallel,
                false,
            )?;
            println!(
                "MCP server '{}' added to {}",
                entry_name,
                mcp_config_path.display()
            );
            println!(
                "Synced MCP server '{}' into {}",
                entry_name,
                config_dir.join("config.yaml").display()
            );
        }
        "remove" => {
            let srv = selected.clone().ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing server name. Usage: hermes mcp remove <name>".into(),
                )
            })?;
            if !mcp_config_path.exists() {
                println!("No MCP config to modify.");
                return Ok(());
            }
            let content = std::fs::read_to_string(&mcp_config_path)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            let mut servers: serde_json::Value =
                serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
            if let Some(obj) = servers.as_object_mut() {
                if obj.remove(&srv).is_some() {
                    let json = serde_json::to_string_pretty(&servers)
                        .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
                    std::fs::write(&mcp_config_path, json)
                        .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                    update_yaml_mcp_server(&config_dir, &srv, None, None, false, true)?;
                    println!("MCP server '{}' removed.", srv);
                    if mcp_auth_path.exists() {
                        let raw = std::fs::read_to_string(&mcp_auth_path).unwrap_or_default();
                        let mut auth: serde_json::Value =
                            serde_json::from_str(&raw).unwrap_or(serde_json::json!({}));
                        if let Some(auth_obj) = auth.as_object_mut() {
                            auth_obj.remove(&srv);
                            let out = serde_json::to_string_pretty(&auth).unwrap_or_default();
                            let _ = std::fs::write(&mcp_auth_path, out);
                        }
                    }
                } else {
                    println!("MCP server '{}' not found.", srv);
                }
            }
        }
        "serve" => {
            use hermes_skills::{FileSkillStore, SkillManager};
            use hermes_tools::ToolRegistry;

            eprintln!("Starting Hermes as MCP server on stdio...");

            let config = hermes_config::load_config(None)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
            let tool_registry = Arc::new(ToolRegistry::new());
            let terminal_backend = crate::terminal_backend::build_terminal_backend(&config);
            let skill_store = Arc::new(FileSkillStore::new(FileSkillStore::default_dir()));
            let skill_provider: Arc<dyn hermes_core::SkillProvider> =
                Arc::new(SkillManager::new(skill_store));
            hermes_tools::register_builtin_tools(&tool_registry, terminal_backend, skill_provider);

            let mcp_server = hermes_mcp::McpServer::new(tool_registry);
            let transport = Box::new(hermes_mcp::ServerStdioTransport::new());
            mcp_server
                .start(transport)
                .await
                .map_err(|e| hermes_core::AgentError::Io(format!("MCP server error: {}", e)))?;
        }
        "test" => {
            let srv = selected.clone().ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing server name. Usage: hermes mcp test <name>".into(),
                )
            })?;
            println!("Testing MCP server: {}...", srv);
            if !mcp_config_path.exists() {
                println!("No MCP config found.");
                return Ok(());
            }
            let content = std::fs::read_to_string(&mcp_config_path)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            let servers: serde_json::Value =
                serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
            match servers.get(&srv) {
                Some(cfg) => {
                    let url = cfg.get("url").and_then(|v| v.as_str()).unwrap_or("(stdio)");
                    let enabled = cfg.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                    let parallel = cfg
                        .get("supports_parallel_tool_calls")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    println!("  Server: {}", srv);
                    println!("  URL: {}", url);
                    println!("  Enabled: {}", enabled);
                    println!(
                        "  Parallel tool calls: {}",
                        if parallel { "on" } else { "off" }
                    );
                    if url.starts_with("http") {
                        match reqwest::Client::new()
                            .get(url)
                            .timeout(std::time::Duration::from_secs(5))
                            .send()
                            .await
                        {
                            Ok(resp) => println!("  Status: {} (reachable)", resp.status()),
                            Err(e) => println!("  Status: unreachable ({})", e),
                        }
                    } else {
                        println!("  Status: stdio transport (not testable via HTTP)");
                    }
                }
                None => println!("Server '{}' not found in MCP config.", srv),
            }
        }
        "configure" => {
            let srv = selected.clone().ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing server name. Usage: hermes mcp configure <name>".into(),
                )
            })?;
            if !mcp_config_path.exists() {
                println!("No MCP config found. Add a server first with `hermes mcp add`.");
                return Ok(());
            }
            let content = std::fs::read_to_string(&mcp_config_path)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            let servers: serde_json::Value =
                serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
            match servers.get(&srv) {
                Some(cfg) => {
                    println!("Current config for '{}':", srv);
                    println!("{}", serde_json::to_string_pretty(cfg).unwrap_or_default());
                    println!("\nEdit {} to modify settings.", mcp_config_path.display());
                }
                None => println!("Server '{}' not found.", srv),
            }
        }
        "login" => {
            let srv = selected.clone().ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing server name. Usage: hermes mcp login <name>".into(),
                )
            })?;
            if !mcp_config_path.exists() {
                return Err(hermes_core::AgentError::Config(format!(
                    "No MCP config found at {}",
                    mcp_config_path.display()
                )));
            }
            let configured = std::fs::read_to_string(&mcp_config_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get(&srv).cloned())
                .is_some();
            if !configured {
                return Err(hermes_core::AgentError::Config(format!(
                    "MCP server '{}' is not configured",
                    srv
                )));
            }

            let env_key = format!("MCP_{}_TOKEN", srv.to_uppercase().replace('-', "_"));
            let token_from_env = std::env::var(&env_key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            let token = if let Some(v) = token_from_env {
                v
            } else {
                use std::io::{self, Write};
                print!("Token for '{}': ", srv);
                let _ = io::stdout().flush();
                let mut buf = String::new();
                io::stdin()
                    .read_line(&mut buf)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                buf.trim().to_string()
            };
            if token.is_empty() {
                return Err(hermes_core::AgentError::Config(
                    "Empty token; aborting mcp login".into(),
                ));
            }
            let mut auth: serde_json::Value = if mcp_auth_path.exists() {
                let raw = std::fs::read_to_string(&mcp_auth_path)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                serde_json::from_str(&raw).unwrap_or(serde_json::json!({}))
            } else {
                serde_json::json!({})
            };
            if let Some(obj) = auth.as_object_mut() {
                obj.insert(
                    srv.clone(),
                    serde_json::json!({
                        "token": token,
                        "updated_at": chrono::Utc::now().to_rfc3339(),
                    }),
                );
            }
            std::fs::write(
                &mcp_auth_path,
                serde_json::to_string_pretty(&auth)
                    .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?,
            )
            .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            println!(
                "Stored MCP auth token for '{}' in {}",
                srv,
                mcp_auth_path.display()
            );
        }
        other => {
            println!("MCP action '{}' is not recognized.", other);
            println!(
                "Available actions: list, add, remove, serve, test, configure, login, sentrux, sentrux-status, sentrux-remove"
            );
        }
    }
    Ok(())
}

fn command_on_path(command: &str) -> bool {
    if command.trim().is_empty() {
        return false;
    }
    let candidate = Path::new(command);
    if candidate.components().count() > 1 {
        return candidate.exists();
    }
    std::env::var_os("PATH").is_some_and(|path_var| {
        std::env::split_paths(&path_var)
            .map(|p| p.join(command))
            .any(|p| p.exists())
    })
}

fn sentrux_entry() -> serde_json::Value {
    serde_json::json!({
        "command": SENTRUX_MCP_COMMAND,
        "args": [SENTRUX_MCP_ARG],
        "enabled": true,
        "supports_parallel_tool_calls": true
    })
}

fn update_yaml_mcp_server(
    config_dir: &Path,
    name: &str,
    command: Option<String>,
    url: Option<String>,
    supports_parallel_tool_calls: bool,
    remove: bool,
) -> Result<(), hermes_core::AgentError> {
    let cfg_path = config_dir.join("config.yaml");
    let mut cfg = hermes_config::load_user_config_file(&cfg_path)
        .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
    cfg.mcp_servers.retain(|entry| entry.name != name);
    if !remove {
        cfg.mcp_servers.push(hermes_config::McpServerEntry {
            name: name.to_string(),
            command,
            url,
            supports_parallel_tool_calls,
        });
        cfg.mcp_servers.sort_by(|a, b| a.name.cmp(&b.name));
    }
    hermes_config::save_config_yaml(&cfg_path, &cfg)
        .map_err(|e| hermes_core::AgentError::Config(e.to_string()))
}

fn upsert_sentrux_mcp_profile(config_dir: &Path) -> Result<bool, hermes_core::AgentError> {
    let mcp_config_path = config_dir.join("mcp_servers.json");
    let mut servers: serde_json::Value = if mcp_config_path.exists() {
        let content = std::fs::read_to_string(&mcp_config_path)
            .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = servers.as_object_mut() {
        obj.insert(SENTRUX_MCP_SERVER_NAME.to_string(), sentrux_entry());
    }
    let json = serde_json::to_string_pretty(&servers)
        .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
    std::fs::write(&mcp_config_path, json)
        .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
    update_yaml_mcp_server(
        config_dir,
        SENTRUX_MCP_SERVER_NAME,
        Some(format!("{SENTRUX_MCP_COMMAND} {SENTRUX_MCP_ARG}")),
        None,
        true,
        false,
    )?;
    Ok(command_on_path(SENTRUX_MCP_COMMAND))
}

fn remove_sentrux_mcp_profile(config_dir: &Path) -> Result<(), hermes_core::AgentError> {
    let mcp_config_path = config_dir.join("mcp_servers.json");
    if mcp_config_path.exists() {
        let content = std::fs::read_to_string(&mcp_config_path)
            .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
        let mut servers: serde_json::Value =
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}));
        if let Some(obj) = servers.as_object_mut() {
            obj.remove(SENTRUX_MCP_SERVER_NAME);
        }
        let json = serde_json::to_string_pretty(&servers)
            .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
        std::fs::write(&mcp_config_path, json)
            .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
    }
    update_yaml_mcp_server(config_dir, SENTRUX_MCP_SERVER_NAME, None, None, false, true)
}

fn sentrux_mcp_status(config_dir: &Path) -> (bool, bool, bool) {
    let mcp_config_path = config_dir.join("mcp_servers.json");
    let from_json = std::fs::read_to_string(&mcp_config_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get(SENTRUX_MCP_SERVER_NAME).cloned())
        .is_some();
    let from_yaml = hermes_config::load_user_config_file(&config_dir.join("config.yaml"))
        .ok()
        .map(|cfg| {
            cfg.mcp_servers
                .iter()
                .any(|entry| entry.name == SENTRUX_MCP_SERVER_NAME)
        })
        .unwrap_or(false);
    (command_on_path(SENTRUX_MCP_COMMAND), from_json, from_yaml)
}

/// Handle `hermes sessions [action] [--id ...] [--name ...]`.
pub async fn handle_cli_sessions(
    action: Option<String>,
    id: Option<String>,
    name: Option<String>,
) -> Result<(), hermes_core::AgentError> {
    let sessions_dir = hermes_config::hermes_home().join("sessions");

    match action.as_deref().unwrap_or("list") {
        "list" => {
            if !sessions_dir.exists() {
                println!("No sessions directory found.");
                return Ok(());
            }
            let mut entries: Vec<(String, u64, std::time::SystemTime, bool, bool, usize)> =
                Vec::new();
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        let stem = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let meta = std::fs::metadata(&path);
                        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let modified = meta
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        let integrity = session::inspect_snapshot_integrity(&path);
                        let canonical = session::is_canonical_snapshot_name(&stem, &integrity);
                        entries.push((
                            stem,
                            size,
                            modified,
                            canonical,
                            integrity.valid,
                            integrity.message_count,
                        ));
                    }
                }
            }
            entries.sort_by(|a, b| {
                b.3.cmp(&a.3)
                    .then_with(|| b.5.cmp(&a.5))
                    .then_with(|| b.2.cmp(&a.2))
                    .then_with(|| a.0.cmp(&b.0))
            });
            if entries.is_empty() {
                println!("No saved sessions.");
            } else {
                let canonical_count = entries.iter().filter(|entry| entry.3).count();
                let artifact_count = entries.len().saturating_sub(canonical_count);
                println!(
                    "Saved sessions ({} total; {} canonical; {} artifacts):",
                    entries.len(),
                    canonical_count,
                    artifact_count
                );
                for (name, size, _, canonical, valid, messages) in &entries {
                    let kind = if *canonical {
                        "session"
                    } else if *valid {
                        "artifact"
                    } else {
                        "invalid"
                    };
                    println!("  • {} ({} bytes, {} msgs, {})", name, size, messages, kind);
                }
            }
        }
        "export" => {
            let session_id = id.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing session ID. Usage: hermes sessions export --id <id>".into(),
                )
            })?;
            let path = sessions_dir.join(format!("{}.json", session_id));
            if !path.exists() {
                println!("Session '{}' not found.", session_id);
                return Ok(());
            }
            let content = std::fs::read_to_string(&path)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            println!("{}", content);
        }
        "delete" => {
            let session_id = id.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing session ID. Usage: hermes sessions delete --id <id>".into(),
                )
            })?;
            let path = sessions_dir.join(format!("{}.json", session_id));
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                println!("Session '{}' deleted.", session_id);
            } else {
                println!("Session '{}' not found.", session_id);
            }
        }
        "stats" => {
            if !sessions_dir.exists() {
                println!("No sessions directory.");
                return Ok(());
            }
            let mut total_files = 0u32;
            let mut total_size = 0u64;
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    if entry
                        .path()
                        .extension()
                        .map(|e| e == "json")
                        .unwrap_or(false)
                    {
                        total_files += 1;
                        total_size += std::fs::metadata(entry.path())
                            .map(|m| m.len())
                            .unwrap_or(0);
                    }
                }
            }
            println!("Session statistics:");
            println!("  Total sessions: {}", total_files);
            println!("  Total size:     {} KB", total_size / 1024);
            println!("  Directory:      {}", sessions_dir.display());
        }
        "prune" => {
            let max_age_days: u64 = name
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(30);
            println!("Pruning sessions older than {} days...", max_age_days);
            if !sessions_dir.exists() {
                println!("No sessions directory.");
                return Ok(());
            }
            let cutoff = std::time::SystemTime::now()
                .checked_sub(std::time::Duration::from_secs(max_age_days * 86400))
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let mut pruned = 0u32;
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !path.extension().map(|e| e == "json").unwrap_or(false) {
                        continue;
                    }
                    if let Ok(meta) = std::fs::metadata(&path) {
                        let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        if modified < cutoff {
                            if std::fs::remove_file(&path).is_ok() {
                                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                                println!("  Pruned: {}", name);
                                pruned += 1;
                            }
                        }
                    }
                }
            }
            println!("Pruned {} session(s).", pruned);
        }
        "rename" => {
            let session_id = id.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing session ID. Usage: hermes sessions rename --id <id> --name <new>"
                        .into(),
                )
            })?;
            let new_name = name.ok_or_else(|| {
                hermes_core::AgentError::Config(
                    "Missing new name. Usage: hermes sessions rename --id <id> --name <new>".into(),
                )
            })?;
            let old_path = sessions_dir.join(format!("{}.json", session_id));
            let new_path = sessions_dir.join(format!("{}.json", new_name));
            if !old_path.exists() {
                println!("Session '{}' not found.", session_id);
                return Ok(());
            }
            std::fs::rename(&old_path, &new_path)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            println!("Session renamed: {} -> {}", session_id, new_name);
        }
        "browse" => {
            if !sessions_dir.exists() {
                println!("No sessions directory found.");
                return Ok(());
            }
            println!("Session Browser");
            println!("===============\n");
            let mut entries: Vec<(String, u64, std::time::SystemTime, usize)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
                for entry in rd.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !path.extension().map(|e| e == "json").unwrap_or(false) {
                        continue;
                    }
                    let stem = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let meta = std::fs::metadata(&path);
                    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    let modified = meta
                        .as_ref()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    let msg_count = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                        .and_then(|v| {
                            v.get("messages")
                                .and_then(|m| m.as_array())
                                .map(|a| a.len())
                        })
                        .unwrap_or(0);
                    entries.push((stem, size, modified, msg_count));
                }
            }
            entries.sort_by(|a, b| b.2.cmp(&a.2));
            if entries.is_empty() {
                println!("No sessions found.");
            } else {
                println!(
                    "{:3} {:30} {:>8} {:>6}  {}",
                    "#", "Session ID", "Size", "Msgs", "Modified"
                );
                println!("{}", "-".repeat(75));
                for (idx, (name, size, modified, msgs)) in entries.iter().enumerate() {
                    let age = modified.elapsed().unwrap_or_default();
                    let age_str = if age.as_secs() < 3600 {
                        format!("{}m ago", age.as_secs() / 60)
                    } else if age.as_secs() < 86400 {
                        format!("{}h ago", age.as_secs() / 3600)
                    } else {
                        format!("{}d ago", age.as_secs() / 86400)
                    };
                    println!(
                        "{:3} {:30} {:>6}KB {:>6}  {}",
                        idx + 1,
                        &name[..name.len().min(30)],
                        size / 1024,
                        msgs,
                        age_str,
                    );
                }
                println!("\nUse `hermes sessions export --id <id>` to view a session.");
            }
        }
        other => {
            println!("Sessions action '{}' is not recognized.", other);
            println!("Available actions: list, export, delete, prune, stats, rename, browse");
        }
    }
    Ok(())
}

/// Handle `hermes insights [--days N] [--source ...]`.
pub async fn handle_cli_insights(
    days: u32,
    source: Option<String>,
) -> Result<(), hermes_core::AgentError> {
    println!("Usage Insights (last {} days)", days);
    println!("=============================");
    if let Some(src) = &source {
        println!("Filter: source={}\n", src);
    }
    let sessions_dir = hermes_config::hermes_home().join("sessions");
    if !sessions_dir.exists() {
        println!("No sessions directory found.");
        return Ok(());
    }

    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(u64::from(days) * 86400))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let mut total_sessions = 0u32;
    let mut total_messages = 0u64;
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;
    let mut total_cost_cents = 0.0f64;
    let mut models_used: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut daily_counts: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();

    if let Ok(rd) = std::fs::read_dir(&sessions_dir) {
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.extension().map(|e| e == "json").unwrap_or(false) {
                continue;
            }
            let meta = std::fs::metadata(&path);
            let modified = meta
                .as_ref()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if modified < cutoff {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(src_filter) = &source {
                        let session_source = data
                            .get("source")
                            .and_then(|s| s.as_str())
                            .unwrap_or("unknown");
                        if session_source != src_filter.as_str() {
                            continue;
                        }
                    }

                    total_sessions += 1;

                    if let Some(msgs) = data.get("messages").and_then(|m| m.as_array()) {
                        total_messages += msgs.len() as u64;
                    }

                    if let Some(usage) = data.get("usage") {
                        total_input_tokens += usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        total_output_tokens += usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        total_cost_cents +=
                            usage.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    }

                    if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
                        *models_used.entry(model.to_string()).or_insert(0) += 1;
                    }

                    let dur = modified
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    let secs = dur.as_secs();
                    let day_secs = secs - (secs % 86400);
                    let day_key = format!("{}", day_secs / 86400);
                    *daily_counts.entry(day_key).or_insert(0) += 1;
                }
            }
        }
    }

    println!("Sessions:       {}", total_sessions);
    println!("Messages:       {}", total_messages);
    println!("Input tokens:   {}", total_input_tokens);
    println!("Output tokens:  {}", total_output_tokens);
    let total_tokens = total_input_tokens + total_output_tokens;
    println!("Total tokens:   {}", total_tokens);
    if total_cost_cents > 0.0 {
        println!("Estimated cost: ${:.4}", total_cost_cents / 100.0);
    }

    if !models_used.is_empty() {
        println!("\nModels Used:");
        let mut model_vec: Vec<_> = models_used.into_iter().collect();
        model_vec.sort_by(|a, b| b.1.cmp(&a.1));
        for (model, count) in &model_vec {
            println!("  {:30} {:>5} session(s)", model, count);
        }
    }

    if total_sessions > 0 {
        println!("\nAverages per session:");
        println!(
            "  Messages: {:.1}",
            total_messages as f64 / total_sessions as f64
        );
        println!(
            "  Tokens:   {:.0}",
            total_tokens as f64 / total_sessions as f64
        );
    }

    Ok(())
}

/// Handle `hermes login [provider]`.
pub async fn handle_cli_login(provider: Option<String>) -> Result<(), hermes_core::AgentError> {
    let provider = provider.unwrap_or_else(|| "openai".to_string());
    let creds_dir = hermes_config::hermes_home().join("credentials");
    std::fs::create_dir_all(&creds_dir).map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;

    println!("Login to: {}", provider);
    println!("----------{}", "-".repeat(provider.len()));

    match provider.as_str() {
        "openai" => {
            let env_key = std::env::var("HERMES_OPENAI_API_KEY")
                .ok()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok());
            if let Some(key) = env_key {
                let masked = if key.len() > 8 {
                    format!("{}...{}", &key[..4], &key[key.len() - 4..])
                } else {
                    "****".to_string()
                };
                println!(
                    "Found HERMES_OPENAI_API_KEY/OPENAI_API_KEY in environment: {}",
                    masked
                );
                let cred_file = creds_dir.join("openai.json");
                let cred = serde_json::json!({
                    "provider": "openai",
                    "api_key_masked": masked,
                    "stored_at": chrono::Utc::now().to_rfc3339(),
                    "source": "env",
                });
                std::fs::write(
                    &cred_file,
                    serde_json::to_string_pretty(&cred).unwrap_or_default(),
                )
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                println!("Credential reference stored at {}", cred_file.display());
            } else {
                println!("No HERMES_OPENAI_API_KEY/OPENAI_API_KEY found in environment.");
                println!("Set it with: export HERMES_OPENAI_API_KEY=sk-...");
                println!("Or use: hermes config set openai_api_key <key>");
            }
        }
        "anthropic" => {
            let env_key = std::env::var("ANTHROPIC_API_KEY").ok();
            if let Some(key) = env_key {
                let masked = if key.len() > 8 {
                    format!("{}...{}", &key[..4], &key[key.len() - 4..])
                } else {
                    "****".to_string()
                };
                println!("Found ANTHROPIC_API_KEY in environment: {}", masked);
                let cred_file = creds_dir.join("anthropic.json");
                let cred = serde_json::json!({
                    "provider": "anthropic",
                    "api_key_masked": masked,
                    "stored_at": chrono::Utc::now().to_rfc3339(),
                    "source": "env",
                });
                std::fs::write(
                    &cred_file,
                    serde_json::to_string_pretty(&cred).unwrap_or_default(),
                )
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                println!("Credential reference stored at {}", cred_file.display());
            } else {
                println!("No ANTHROPIC_API_KEY found in environment.");
                println!("Set it with: export ANTHROPIC_API_KEY=sk-ant-...");
            }
        }
        other => {
            let env_var = format!("{}_API_KEY", other.to_uppercase().replace('-', "_"));
            let env_key = std::env::var(&env_var).ok();
            if let Some(key) = env_key {
                let masked = if key.len() > 8 {
                    format!("{}...{}", &key[..4], &key[key.len() - 4..])
                } else {
                    "****".to_string()
                };
                println!("Found {} in environment: {}", env_var, masked);
                let cred_file = creds_dir.join(format!("{}.json", other));
                let cred = serde_json::json!({
                    "provider": other,
                    "api_key_masked": masked,
                    "stored_at": chrono::Utc::now().to_rfc3339(),
                    "source": "env",
                });
                std::fs::write(
                    &cred_file,
                    serde_json::to_string_pretty(&cred).unwrap_or_default(),
                )
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                println!("Credential reference stored.");
            } else {
                println!("No {} found in environment.", env_var);
                println!("Set it with: export {}=<your-key>", env_var);
            }
        }
    }
    Ok(())
}

/// Handle `hermes logout [provider]`.
pub async fn handle_cli_logout(provider: Option<String>) -> Result<(), hermes_core::AgentError> {
    let creds_dir = hermes_config::hermes_home().join("credentials");

    match provider.as_deref() {
        Some(p) => {
            let cred_file = creds_dir.join(format!("{}.json", p));
            if cred_file.exists() {
                std::fs::remove_file(&cred_file)
                    .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
                println!("Logged out from '{}'. Credential reference removed.", p);
            } else {
                println!("No stored credentials for '{}'.", p);
            }
            println!(
                "Note: Environment variables (e.g. {}_API_KEY) are not affected.",
                p.to_uppercase().replace('-', "_")
            );
        }
        None => {
            if creds_dir.exists() {
                let mut removed = 0u32;
                if let Ok(rd) = std::fs::read_dir(&creds_dir) {
                    for entry in rd.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if path.extension().map(|e| e == "json").unwrap_or(false) {
                            if std::fs::remove_file(&path).is_ok() {
                                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                                println!("  Removed credential: {}", name);
                                removed += 1;
                            }
                        }
                    }
                }
                if removed == 0 {
                    println!("No stored credentials to remove.");
                } else {
                    println!("Logged out from {} provider(s).", removed);
                }
            } else {
                println!("No credentials directory found.");
            }
            println!("Note: Environment variables are not affected.");
        }
    }
    Ok(())
}

/// Handle `hermes whatsapp [action]`.
pub async fn handle_cli_whatsapp(action: Option<String>) -> Result<(), hermes_core::AgentError> {
    match action.as_deref().unwrap_or("setup") {
        "setup" | "" => crate::whatsapp_wizard::whatsapp_baileys_wizard().await,
        "status" => crate::whatsapp_wizard::whatsapp_baileys_status().await,
        "pair" | "qr" => crate::whatsapp_wizard::whatsapp_baileys_wizard().await,
        "cloud" => crate::whatsapp_wizard::whatsapp_cloud_setup().await,
        other => {
            println!("WhatsApp action '{}' is not recognized.", other);
            println!("Available actions: setup, status, pair, cloud");
            Ok(())
        }
    }
}

/// Cloud API setup (optional feature `whatsapp-cloud`).
pub(crate) async fn whatsapp_cloud_setup_impl() -> Result<(), hermes_core::AgentError> {
    whatsapp_cloud_setup_legacy().await
}

async fn whatsapp_cloud_setup_legacy() -> Result<(), hermes_core::AgentError> {
    use std::io::{self, BufRead, Write};

    println!("WhatsApp Cloud API Setup");
    println!("========================\n");
    println!("You will need credentials from the Meta developer dashboard:");
    println!("  https://developers.facebook.com/apps/\n");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    print!("Phone Number ID: ");
    stdout.flush().ok();
    let phone_number_id = stdin
        .lock()
        .lines()
        .next()
        .and_then(|l| l.ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    if phone_number_id.is_empty() {
        println!("Aborted: phone number ID is required.");
        return Ok(());
    }

    print!("Business Account ID: ");
    stdout.flush().ok();
    let business_account_id = stdin
        .lock()
        .lines()
        .next()
        .and_then(|l| l.ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    if business_account_id.is_empty() {
        println!("Aborted: business account ID is required.");
        return Ok(());
    }

    print!("Access Token: ");
    stdout.flush().ok();
    let access_token = stdin
        .lock()
        .lines()
        .next()
        .and_then(|l| l.ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    if access_token.is_empty() {
        println!("Aborted: access token is required.");
        return Ok(());
    }

    println!("\nVerifying token against WhatsApp Cloud API...");
    let url = format!(
        "https://graph.facebook.com/v21.0/{}/messages",
        phone_number_id
    );
    let client = reqwest::Client::new();
    match client
        .get(&url)
        .bearer_auth(&access_token)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() || status.as_u16() == 400 {
                // 400 means the endpoint is reachable (POST required for actual messages)
                println!("  API reachable (HTTP {}).", status);
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                println!("  Warning: API returned {} — token may be invalid.", status);
                println!("  Saving anyway; you can re-run setup later.");
            } else {
                println!("  API returned HTTP {}. Saving config anyway.", status);
            }
        }
        Err(e) => {
            println!("  Could not reach API: {}", e);
            println!("  Saving config anyway — verify network connectivity.");
        }
    }

    let config_path = hermes_config::hermes_home().join("config.yaml");
    let mut config: serde_yaml::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| hermes_core::AgentError::Io(format!("Read error: {}", e)))?;
        serde_yaml::from_str(&content).unwrap_or(serde_yaml::Value::Mapping(Default::default()))
    } else {
        serde_yaml::Value::Mapping(Default::default())
    };

    let platforms = config
        .as_mapping_mut()
        .unwrap()
        .entry(serde_yaml::Value::String("platforms".into()))
        .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));

    let wa = platforms
        .as_mapping_mut()
        .unwrap()
        .entry(serde_yaml::Value::String("whatsapp".into()))
        .or_insert_with(|| serde_yaml::Value::Mapping(Default::default()));

    let wa_map = wa.as_mapping_mut().unwrap();
    wa_map.insert(
        serde_yaml::Value::String("phone_number_id".into()),
        serde_yaml::Value::String(phone_number_id.clone()),
    );
    wa_map.insert(
        serde_yaml::Value::String("business_account_id".into()),
        serde_yaml::Value::String(business_account_id),
    );
    wa_map.insert(
        serde_yaml::Value::String("access_token".into()),
        serde_yaml::Value::String(access_token),
    );
    wa_map.insert(
        serde_yaml::Value::String("enabled".into()),
        serde_yaml::Value::Bool(true),
    );

    let yaml_str = serde_yaml::to_string(&config)
        .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;
    std::fs::create_dir_all(hermes_config::hermes_home())
        .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
    std::fs::write(&config_path, &yaml_str)
        .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;

    println!(
        "\nWhatsApp configuration saved to {}",
        config_path.display()
    );
    println!("Phone Number ID: {}", phone_number_id);
    println!("\nRun `hermes whatsapp status` to verify.");
    Ok(())
}

/// Check whether WhatsApp is configured and verify connectivity.
async fn whatsapp_status() -> Result<(), hermes_core::AgentError> {
    let config_path = hermes_config::hermes_home().join("config.yaml");
    if !config_path.exists() {
        println!("WhatsApp: not configured");
        println!("Run `hermes whatsapp setup` to configure.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
    let config: serde_yaml::Value =
        serde_yaml::from_str(&content).unwrap_or(serde_yaml::Value::Mapping(Default::default()));

    let wa = config.get("platforms").and_then(|p| p.get("whatsapp"));

    match wa {
        None => {
            println!("WhatsApp: not configured");
            println!("Run `hermes whatsapp setup` to configure.");
        }
        Some(wa_cfg) => {
            let phone_id = wa_cfg
                .get("phone_number_id")
                .and_then(|v| v.as_str())
                .unwrap_or("(not set)");
            let enabled = wa_cfg
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let has_token = wa_cfg
                .get("access_token")
                .and_then(|v| v.as_str())
                .map(|t| !t.is_empty())
                .unwrap_or(false);

            println!("WhatsApp Status");
            println!("---------------");
            println!("  Configured:     yes");
            println!("  Enabled:        {}", enabled);
            println!("  Phone Number ID: {}", phone_id);
            println!(
                "  Access Token:   {}",
                if has_token { "present" } else { "missing" }
            );

            if has_token {
                let token = wa_cfg.get("access_token").unwrap().as_str().unwrap();
                let url = format!("https://graph.facebook.com/v21.0/{}/messages", phone_id);
                print!("  API Connectivity: ");
                match reqwest::Client::new()
                    .get(&url)
                    .bearer_auth(token)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(resp) => println!("reachable (HTTP {})", resp.status()),
                    Err(e) => println!("unreachable ({})", e),
                }
            }
        }
    }
    Ok(())
}

/// Connect to local bridge, fetch QR data, and render in terminal.
async fn whatsapp_qr() -> Result<(), hermes_core::AgentError> {
    let config_path = hermes_config::hermes_home().join("config.yaml");
    let bridge_url = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path).unwrap_or_default();
        let config: serde_yaml::Value = serde_yaml::from_str(&content)
            .unwrap_or(serde_yaml::Value::Mapping(Default::default()));
        config
            .get("platforms")
            .and_then(|p| p.get("whatsapp"))
            .and_then(|w| w.get("bridge_url"))
            .and_then(|u| u.as_str())
            .unwrap_or("http://localhost:3000")
            .to_string()
    } else {
        "http://localhost:3000".to_string()
    };

    let qr_url = format!("{}/qr", bridge_url);
    println!("Fetching QR code from {}...", qr_url);

    match reqwest::Client::new()
        .get(&qr_url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let body = resp
                .text()
                .await
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;

            let qr_data = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                json.get("qr")
                    .or_else(|| json.get("data"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&body)
                    .to_string()
            } else {
                body
            };

            println!();
            render_qr_to_terminal(&qr_data);
            println!();
            println!("Scan this QR code with WhatsApp on your phone:");
            println!("  WhatsApp → Settings → Linked Devices → Link a Device");
        }
        Ok(resp) => {
            println!(
                "Bridge returned HTTP {}. Is the bridge server running?",
                resp.status()
            );
            println!("Start it with: npx hermes-whatsapp-bridge");
        }
        Err(e) => {
            println!("Could not connect to bridge at {}: {}", bridge_url, e);
            println!("\nMake sure the WhatsApp Web bridge is running:");
            println!("  npx hermes-whatsapp-bridge");
            println!("  # or: docker run -p 3000:3000 hermes/whatsapp-bridge");
        }
    }
    Ok(())
}

/// Render QR data as Unicode block art in the terminal.
fn render_qr_to_terminal(data: &str) {
    let code = match qrcode::QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("QR 码生成失败: {}", e);
            return;
        }
    };
    let side = code.width() as usize;
    let modules = code.to_colors();
    let padded = side + 8;
    let is_dark = |r: usize, c: usize| modules[r * side + c] == qrcode::Color::Dark;
    let mut row = 0usize;
    while row < padded {
        let mut line = String::new();
        for col in 0..padded {
            let qr_row = row.wrapping_sub(4);
            let qr_col = col.wrapping_sub(4);
            let top = qr_row < side && qr_col < side && is_dark(qr_row, qr_col);
            let qr_row2 = (row + 1).wrapping_sub(4);
            let bottom = qr_row2 < side && qr_col < side && is_dark(qr_row2, qr_col);
            line.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        println!("  {}", line);
        row += 2;
    }
}

/// Handle `hermes pairing`.
///
/// Supports both:
/// - Legacy device pairing (`--device-id`)
/// - Python-compatible DM pairing (`approve <platform> <code>`)
pub async fn handle_cli_pairing(
    action: Option<String>,
    device_id: Option<String>,
    args: Vec<String>,
) -> Result<(), hermes_core::AgentError> {
    use crate::pairing_store::{PairingStatus, PairingStore};
    use hermes_gateway::DmPairingStore;

    let store = PairingStore::open_default();
    let dm_store = DmPairingStore::open_default();
    let action = action.unwrap_or_else(|| "list".to_string());

    match action.as_str() {
        "list" => {
            let devices = store.list().map_err(|e| hermes_core::AgentError::Io(e))?;
            if devices.is_empty() {
                println!("No paired devices.");
                println!("  Store: {}", PairingStore::default_path().display());
            } else {
                println!("Paired devices ({}):", devices.len());
                println!(
                    "  {:20} {:10} {:12} {}",
                    "Device ID", "Status", "Last Seen", "Name"
                );
                println!("  {}", "-".repeat(60));
                for d in &devices {
                    let last_seen = d.last_seen.as_deref().unwrap_or("never");
                    let name = d.name.as_deref().unwrap_or("(unnamed)");
                    let status_icon = match d.status {
                        PairingStatus::Pending => "⏳",
                        PairingStatus::Approved => "✓",
                        PairingStatus::Revoked => "✗",
                    };
                    println!(
                        "  {:20} {} {:8} {:12} {}",
                        d.device_id, status_icon, d.status, last_seen, name
                    );
                }
            }
            let pending = dm_store.list_pending(None);
            let approved = dm_store.list_approved(None);
            if pending.is_empty() && approved.is_empty() {
                println!("No DM pairing data found.");
            } else {
                if !pending.is_empty() {
                    println!("\nPending DM pairing requests ({}):", pending.len());
                    println!(
                        "  {:10} {:12} {:20} {:20} {}",
                        "Platform", "Code*", "User ID", "Name", "Age"
                    );
                    println!("  {}", "-".repeat(80));
                    for p in pending {
                        println!(
                            "  {:10} {:12} {:20} {:20} {}m",
                            p.platform, p.code, p.user_id, p.user_name, p.age_minutes
                        );
                    }
                    println!("  * code is hash prefix for display only");
                }
                if !approved.is_empty() {
                    println!("\nApproved DM users ({}):", approved.len());
                    println!("  {:10} {:24} {}", "Platform", "User ID", "Name");
                    println!("  {}", "-".repeat(60));
                    for a in approved {
                        println!("  {:10} {:24} {}", a.platform, a.user_id, a.user_name);
                    }
                }
            }
        }
        "approve" => {
            if let Some(did) = device_id {
                match store.approve(&did) {
                    Ok(dev) => {
                        println!("Device '{}' approved.", dev.device_id);
                        if let Some(secret) = &dev.shared_secret {
                            if secret_stdout_allowed() {
                                println!("  Shared secret: {}", secret);
                                println!(
                                    "  (plaintext output enabled via HERMES_ALLOW_SECRET_STDOUT=1)"
                                );
                            } else {
                                println!("  Shared secret: {}", mask_secret_value(secret));
                                println!(
                                    "  (set HERMES_ALLOW_SECRET_STDOUT=1 to reveal plaintext once)"
                                );
                            }
                            println!("  (Store this securely — it will not be shown again)");
                        }
                    }
                    Err(e) => println!("Failed to approve device: {}", e),
                }
            } else if args.len() >= 2 {
                let platform = &args[0];
                let code = &args[1];
                match dm_store
                    .approve_code(platform, code)
                    .map_err(hermes_core::AgentError::Io)?
                {
                    Some(user) => {
                        let display = if user.user_name.trim().is_empty() {
                            user.user_id.clone()
                        } else {
                            format!("{} ({})", user.user_name, user.user_id)
                        };
                        println!(
                            "Approved! User {} on {} can now use DM access.",
                            display, platform
                        );
                    }
                    None => {
                        println!(
                            "Code '{}' not found, expired, or locked out on '{}'.",
                            code, platform
                        );
                    }
                }
            } else {
                return Err(hermes_core::AgentError::Config(
                    "Missing args. Usage: hermes pairing approve --device-id <id> OR hermes pairing approve <platform> <code>".into(),
                ));
            }
        }
        "revoke" => {
            if let Some(did) = device_id {
                match store.revoke(&did) {
                    Ok(dev) => {
                        println!("Device '{}' revoked.", dev.device_id);
                        println!("  The device will no longer be able to connect.");
                    }
                    Err(e) => println!("Failed to revoke device: {}", e),
                }
            } else if args.len() >= 2 {
                let platform = &args[0];
                let user_id = &args[1];
                let revoked = dm_store
                    .revoke(platform, user_id)
                    .map_err(hermes_core::AgentError::Io)?;
                if revoked {
                    println!("Revoked DM access for {} on {}.", user_id, platform);
                } else {
                    println!("User {} was not approved on {}.", user_id, platform);
                }
            } else {
                return Err(hermes_core::AgentError::Config(
                    "Missing args. Usage: hermes pairing revoke --device-id <id> OR hermes pairing revoke <platform> <user_id>".into(),
                ));
            }
        }
        "clear-pending" => {
            match store.clear_pending() {
                Ok(count) => {
                    if count == 0 {
                        println!("No pending pairing requests to clear.");
                    } else {
                        println!("Cleared {} pending pairing request(s).", count);
                    }
                }
                Err(e) => println!("Failed to clear pending requests: {}", e),
            }
            let platform = args.first().map(|s| s.as_str());
            match dm_store.clear_pending(platform) {
                Ok(count) => {
                    if platform.is_some() {
                        println!("Cleared {} pending DM requests.", count);
                    } else {
                        println!(
                            "Cleared {} pending DM requests across all platforms.",
                            count
                        );
                    }
                }
                Err(e) => println!("Failed to clear DM pending requests: {}", e),
            }
        }
        other => {
            println!("Pairing action '{}' is not recognized.", other);
            println!("Available actions: list, approve, revoke, clear-pending");
        }
    }
    Ok(())
}

/// Handle `hermes claw [action]`.
pub async fn handle_cli_claw(action: Option<String>) -> Result<(), hermes_core::AgentError> {
    match action.as_deref().unwrap_or("status") {
        "migrate" => {
            claw_migrate_cmd()?;
        }
        "cleanup" => {
            claw_cleanup_cmd()?;
        }
        "status" => {
            claw_status_cmd();
        }
        other => {
            println!("Claw action '{}' is not recognized.", other);
            println!("Available actions: migrate, cleanup, status");
        }
    }
    Ok(())
}

/// Check for legacy OpenClaw artefacts and report findings.
fn claw_status_cmd() {
    use crate::claw_migrate::find_openclaw_dir;

    println!("OpenClaw Legacy Status");
    println!("======================\n");

    let home = dirs::home_dir();

    match find_openclaw_dir(None) {
        Some(dir) => {
            println!("  OpenClaw directory: {} (found)", dir.display());

            let config_yaml = dir.join("config.yaml");
            let sessions_dir = dir.join("sessions");
            let env_file = dir.join(".env");
            let skills_dir = dir.join("skills");

            println!(
                "  config.yaml:       {}",
                if config_yaml.exists() {
                    "present"
                } else {
                    "not found"
                }
            );
            println!(
                "  .env:              {}",
                if env_file.exists() {
                    "present"
                } else {
                    "not found"
                }
            );
            println!(
                "  skills/:           {}",
                if skills_dir.is_dir() {
                    "present"
                } else {
                    "not found"
                }
            );

            if sessions_dir.is_dir() {
                let count = std::fs::read_dir(&sessions_dir)
                    .map(|rd| rd.filter_map(|e| e.ok()).count())
                    .unwrap_or(0);
                println!("  sessions/:         {} file(s)", count);
            } else {
                println!("  sessions/:         not found");
            }

            println!("\n  Run `hermes claw migrate` to import into Hermes.");
            println!("  Run `hermes claw cleanup` to remove legacy files.");
        }
        None => {
            println!("  No OpenClaw directory found.");
            if let Some(h) = &home {
                println!(
                    "  Checked: ~/.openclaw, ~/.clawdbot, ~/.moldbot under {}",
                    h.display()
                );
            }
            println!("\n  Nothing to migrate.");
        }
    }

    // Also check for PATH entries in shell configs
    if let Some(h) = &home {
        let shell_files = [".bashrc", ".zshrc", ".profile", ".bash_profile"];
        let mut found_refs = Vec::new();
        for f in &shell_files {
            let path = h.join(f);
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains("openclaw") || content.contains("clawdbot") {
                    found_refs.push(f.to_string());
                }
            }
        }
        if !found_refs.is_empty() {
            println!("\n  Shell config references found:");
            for f in &found_refs {
                println!("    ~/{}", f);
            }
        }
    }
}

/// Run the full migration using `claw_migrate::run_migration`.
fn claw_migrate_cmd() -> Result<(), hermes_core::AgentError> {
    use crate::claw_migrate::{MigrateOptions, find_openclaw_dir, run_migration};

    println!("OpenClaw → Hermes Migration");
    println!("===========================\n");

    let source_dir = find_openclaw_dir(None);
    if source_dir.is_none() {
        println!("No OpenClaw directory found. Nothing to migrate.");
        return Ok(());
    }
    let source_dir = source_dir.unwrap();
    println!("Source: {}", source_dir.display());
    println!("Target: {}\n", hermes_config::hermes_home().display());

    // Also copy sessions if they exist
    let src_sessions = source_dir.join("sessions");
    let dst_sessions = hermes_config::hermes_home().join("sessions");
    let mut session_count = 0usize;

    if src_sessions.is_dir() {
        std::fs::create_dir_all(&dst_sessions).map_err(|e| {
            hermes_core::AgentError::Io(format!("Failed to create sessions dir: {}", e))
        })?;
        if let Ok(entries) = std::fs::read_dir(&src_sessions) {
            for entry in entries.flatten() {
                let src = entry.path();
                let dst = dst_sessions.join(entry.file_name());
                if src.is_file() && !dst.exists() {
                    if std::fs::copy(&src, &dst).is_ok() {
                        session_count += 1;
                    }
                }
            }
        }
    }

    let options = MigrateOptions {
        source: Some(source_dir),
        dry_run: false,
        preset: "full".to_string(),
        overwrite: false,
    };

    let result = run_migration(&options);

    if !result.migrated.is_empty() {
        println!("Migrated:");
        for item in &result.migrated {
            let src = item
                .source
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let dst = item
                .destination
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let extra = item.reason.as_deref().unwrap_or("");
            println!("  ✓ {} → {} {}", src, dst, extra);
        }
    }

    if !result.skipped.is_empty() {
        println!("Skipped:");
        for item in &result.skipped {
            let reason = item.reason.as_deref().unwrap_or("");
            println!("  ⊘ {} — {}", item.kind, reason);
        }
    }

    if !result.errors.is_empty() {
        println!("Errors:");
        for item in &result.errors {
            let reason = item.reason.as_deref().unwrap_or("unknown error");
            println!("  ✗ {} — {}", item.kind, reason);
        }
    }

    if session_count > 0 {
        println!("\nSessions copied: {}", session_count);
    }

    let total = result.migrated.len() + session_count;
    println!(
        "\nMigration complete: {} item(s) migrated, {} skipped, {} error(s).",
        total,
        result.skipped.len(),
        result.errors.len()
    );

    Ok(())
}

/// Remove legacy OpenClaw files after confirmation.
fn claw_cleanup_cmd() -> Result<(), hermes_core::AgentError> {
    use crate::claw_migrate::find_openclaw_dir;
    use std::io::{self, BufRead, Write};

    let source_dir = find_openclaw_dir(None);
    if source_dir.is_none() {
        println!("No OpenClaw directory found. Nothing to clean up.");
        return Ok(());
    }
    let source_dir = source_dir.unwrap();

    println!("OpenClaw Cleanup");
    println!("================\n");
    println!("The following will be PERMANENTLY deleted:");
    println!("  Directory: {}", source_dir.display());

    // Count contents
    let file_count = count_files_recursive(&source_dir);
    println!("  Contains:  ~{} file(s)\n", file_count);

    // Check shell configs
    let home = dirs::home_dir();
    let shell_files = [".bashrc", ".zshrc", ".profile", ".bash_profile"];
    let mut affected_shells: Vec<String> = Vec::new();
    if let Some(h) = &home {
        for f in &shell_files {
            let path = h.join(f);
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains("openclaw") || content.contains("clawdbot") {
                    affected_shells.push(f.to_string());
                    println!("  Shell config: ~/{} (contains openclaw references)", f);
                }
            }
        }
    }

    print!("\nProceed with cleanup? [y/N]: ");
    io::stdout().flush().ok();
    let answer = io::stdin()
        .lock()
        .lines()
        .next()
        .and_then(|l| l.ok())
        .unwrap_or_default();

    if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
        println!("Cleanup cancelled.");
        return Ok(());
    }

    // Remove the directory
    match std::fs::remove_dir_all(&source_dir) {
        Ok(_) => println!("  ✓ Removed {}", source_dir.display()),
        Err(e) => println!("  ✗ Failed to remove {}: {}", source_dir.display(), e),
    }

    // Clean shell configs
    if let Some(h) = &home {
        for f in &affected_shells {
            let path = h.join(f);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let cleaned: Vec<&str> = content
                    .lines()
                    .filter(|line| {
                        let lower = line.to_lowercase();
                        !lower.contains("openclaw") && !lower.contains("clawdbot")
                    })
                    .collect();
                let new_content = cleaned.join("\n") + "\n";
                match std::fs::write(&path, new_content) {
                    Ok(_) => println!("  ✓ Cleaned ~/{}", f),
                    Err(e) => println!("  ✗ Failed to clean ~/{}: {}", f, e),
                }
            }
        }
    }

    println!("\nCleanup complete.");
    Ok(())
}

/// Recursively count files in a directory.
fn count_files_recursive(dir: &std::path::Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_files_recursive(&path);
            } else {
                count += 1;
            }
        }
    }
    count
}

const ACP_MULTIMODAL_PREFIX: &str = "__hermes_acp_parts_json__:";

fn looks_like_openai_parts(parts: &[serde_json::Value]) -> bool {
    !parts.is_empty()
        && parts.iter().all(|part| {
            part.as_object()
                .and_then(|obj| obj.get("type"))
                .and_then(|v| v.as_str())
                .is_some()
        })
}

fn flatten_openai_parts_to_text(parts: &[serde_json::Value]) -> String {
    let mut chunks: Vec<String> = Vec::new();
    for part in parts {
        let Some(obj) = part.as_object() else {
            continue;
        };
        let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        chunks.push(text.to_string());
                    }
                }
            }
            "image_url" | "input_image" => {
                let url = obj
                    .get("image_url")
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str())
                    .or_else(|| obj.get("image_url").and_then(|v| v.as_str()))
                    .or_else(|| obj.get("url").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !url.is_empty() {
                    chunks.push(format!("[Attached image]\nURL: {url}"));
                }
            }
            _ => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    if !text.is_empty() {
                        chunks.push(text.to_string());
                    }
                }
            }
        }
    }
    chunks.join("\n")
}

fn acp_history_to_messages(
    history: &[serde_json::Value],
    fallback_user_text: &str,
) -> Vec<hermes_core::Message> {
    let mut messages = Vec::new();

    for item in history {
        let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content_value = item.get("content").or_else(|| item.get("text"));
        let content = match content_value {
            Some(serde_json::Value::String(s)) => s.to_string(),
            Some(serde_json::Value::Array(parts)) if looks_like_openai_parts(parts) => {
                if role == "user" {
                    match serde_json::to_string(parts) {
                        Ok(serialized) => format!("{ACP_MULTIMODAL_PREFIX}{serialized}"),
                        Err(_) => flatten_openai_parts_to_text(parts),
                    }
                } else {
                    flatten_openai_parts_to_text(parts)
                }
            }
            Some(serde_json::Value::Object(obj)) => obj
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        };

        match role {
            "system" if !content.is_empty() => messages.push(hermes_core::Message::system(content)),
            "user" if !content.is_empty() => messages.push(hermes_core::Message::user(content)),
            "assistant" => {
                if let Some(tool_calls_val) = item.get("tool_calls") {
                    if let Ok(tool_calls) =
                        serde_json::from_value::<Vec<hermes_core::ToolCall>>(tool_calls_val.clone())
                    {
                        let assistant = hermes_core::Message::assistant_with_tool_calls(
                            if content.is_empty() {
                                None
                            } else {
                                Some(content)
                            },
                            tool_calls,
                        );
                        messages.push(assistant);
                        continue;
                    }
                }
                if !content.is_empty() {
                    messages.push(hermes_core::Message::assistant(content));
                }
            }
            "tool" if !content.is_empty() => {
                let tool_call_id = item
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool_call");
                messages.push(hermes_core::Message::tool_result(tool_call_id, content));
            }
            _ => {}
        }
    }

    let has_user_tail = messages
        .last()
        .map(|m| matches!(m.role, hermes_core::MessageRole::User))
        .unwrap_or(false);
    if !has_user_tail && !fallback_user_text.trim().is_empty() {
        messages.push(hermes_core::Message::user(fallback_user_text));
    }

    messages
}

struct CliAcpPromptExecutor {
    config: Arc<hermes_config::GatewayConfig>,
    tool_registry: Arc<hermes_tools::ToolRegistry>,
    tool_schemas: Vec<hermes_core::ToolSchema>,
}

#[async_trait::async_trait]
impl hermes_acp::AcpPromptExecutor for CliAcpPromptExecutor {
    async fn execute_prompt(
        &self,
        session: &hermes_acp::SessionState,
        user_text: &str,
        history: &[serde_json::Value],
    ) -> Result<hermes_acp::PromptExecutionOutput, String> {
        let model = session
            .model
            .clone()
            .or_else(|| self.config.model.clone())
            .unwrap_or_else(|| "gpt-4o".to_string());

        let provider = crate::app::build_provider(&self.config, &model);
        let mut agent_config = crate::app::build_agent_config(&self.config, &model);
        agent_config.session_id = Some(session.session_id.clone());

        let agent_tools = Arc::new(crate::app::bridge_tool_registry(&self.tool_registry));
        let agent = hermes_agent::attach_agent_runtime(
            hermes_agent::AgentLoop::new(agent_config, agent_tools, provider)
                .with_async_tool_dispatch(crate::app::async_tool_dispatch_for(
                    self.tool_registry.clone(),
                )),
        );
        let messages = acp_history_to_messages(history, user_text);
        let (conversation_history, user_message) =
            split_messages_for_run_conversation(&messages)
                .ok_or_else(|| "ACP prompt has no user message for run_conversation".to_string())?;
        let task_id = Some(session.session_id.clone());
        let conv = agent
            .run_conversation(RunConversationParams {
                user_message,
                conversation_history,
                task_id,
                stream_callback: None,
                persist_user_message: None,
                tools: Some(self.tool_schemas.clone()),
                persist_session: false,
            })
            .await
            .map_err(|e| e.to_string())?;
        let result = conv.into_loop_result();
        let response_text = result
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, hermes_core::MessageRole::Assistant))
            .and_then(|m| m.content.clone())
            .unwrap_or_default();

        let usage = result.usage.map(|u| hermes_acp::Usage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            thought_tokens: None,
            cached_read_tokens: None,
        });

        Ok(hermes_acp::PromptExecutionOutput {
            response_text,
            usage,
            total_turns: Some(result.total_turns),
            events: Vec::new(),
        })
    }
}

/// Handle `hermes acp [action]`.
pub async fn handle_cli_acp(action: Option<String>) -> Result<(), hermes_core::AgentError> {
    match action.as_deref().unwrap_or("status") {
        "start" => {
            let config = hermes_config::load_config(None)
                .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;

            let model = config.model.clone().unwrap_or_else(|| "gpt-4o".to_string());
            let max_turns = config.max_turns as usize;

            println!(
                "Starting ACP server (model={}, max_turns={})...",
                model, max_turns
            );

            let tool_registry = Arc::new(hermes_tools::ToolRegistry::new());
            let terminal_backend = crate::terminal_backend::build_terminal_backend(&config);
            let skill_store = Arc::new(hermes_skills::FileSkillStore::new(
                hermes_skills::FileSkillStore::default_dir(),
            ));
            let skill_provider: Arc<dyn hermes_core::SkillProvider> =
                Arc::new(hermes_skills::SkillManager::new(skill_store));
            hermes_tools::register_builtin_tools(&tool_registry, terminal_backend, skill_provider);
            crate::runtime_tool_wiring::wire_stdio_clarify_backend(&tool_registry);
            let cron_data_dir = hermes_config::cron_dir();
            std::fs::create_dir_all(&cron_data_dir)
                .map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;
            let cron_scheduler = Arc::new(hermes_cron::cron_scheduler_for_data_dir(cron_data_dir));
            cron_scheduler
                .load_persisted_jobs()
                .await
                .map_err(|e| hermes_core::AgentError::Config(format!("cron load: {e}")))?;
            cron_scheduler.start().await;
            crate::runtime_tool_wiring::wire_cron_scheduler_backend(
                &tool_registry,
                cron_scheduler,
                MessagingSessionContext::new(),
            );
            let tool_schemas = crate::platform_toolsets::resolve_platform_tool_schemas(
                &config,
                "cli",
                &tool_registry,
            );

            let prompt_executor = Arc::new(CliAcpPromptExecutor {
                config: Arc::new(config.clone()),
                tool_registry,
                tool_schemas,
            });

            let session_manager = Arc::new(hermes_acp::SessionManager::new());
            let event_sink = Arc::new(hermes_acp::EventSink::default());
            let permission_store = Arc::new(hermes_acp::PermissionStore::new());
            let handler = Arc::new(
                hermes_acp::HermesAcpHandler::new(
                    session_manager.clone(),
                    event_sink.clone(),
                    permission_store.clone(),
                )
                .with_prompt_executor(prompt_executor),
            );
            let server = hermes_acp::AcpServer::with_components(
                handler,
                session_manager,
                event_sink,
                permission_store,
            );

            server
                .run()
                .await
                .map_err(|e| hermes_core::AgentError::Io(format!("ACP server error: {}", e)))?;
        }
        "status" => {
            println!("ACP server: not running");
            println!("ACP runs as a stdio JSON-RPC server in the foreground.");
            println!("Start with `hermes acp start`.");
        }
        "stop" => {
            println!("ACP stop is not a separate command in stdio mode.");
            println!("If running, stop it by closing the parent process or sending Ctrl+C.");
        }
        "restart" => {
            println!("ACP restart in stdio mode is equivalent to stop + start.");
            println!("Use:");
            println!("  1) Stop the current process (Ctrl+C)");
            println!("  2) Run `hermes acp start`");
        }
        other => {
            println!("Unknown ACP action '{}'.", other);
            println!("Available actions: start, status, stop, restart");
        }
    }
    Ok(())
}

/// Handle `hermes backup [output]`.
pub async fn handle_cli_backup(output: Option<String>) -> Result<(), hermes_core::AgentError> {
    let hermes_dir = hermes_config::hermes_home();
    if !hermes_dir.exists() {
        println!(
            "Hermes home directory not found at {}",
            hermes_dir.display()
        );
        return Ok(());
    }
    let out = output.unwrap_or_else(|| {
        format!(
            "hermes-backup-{}.tar.gz",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        )
    });
    println!("Backing up {} -> {}", hermes_dir.display(), out);

    let tar_gz = std::fs::File::create(&out)
        .map_err(|e| hermes_core::AgentError::Io(format!("Cannot create {}: {}", out, e)))?;
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);
    tar.append_dir_all("hermes", &hermes_dir)
        .map_err(|e| hermes_core::AgentError::Io(format!("Tar error: {}", e)))?;
    tar.finish()
        .map_err(|e| hermes_core::AgentError::Io(format!("Tar finish error: {}", e)))?;

    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!("Backup complete: {} ({} KB)", out, size / 1024);
    Ok(())
}

/// Handle `hermes import <path>`.
pub async fn handle_cli_import(path: String) -> Result<(), hermes_core::AgentError> {
    let src = std::path::Path::new(&path);
    if !src.exists() {
        return Err(hermes_core::AgentError::Io(format!(
            "Backup archive not found: {}",
            path
        )));
    }
    println!("Importing configuration from: {}", path);

    let hermes_dir = hermes_config::hermes_home();
    std::fs::create_dir_all(&hermes_dir).map_err(|e| hermes_core::AgentError::Io(e.to_string()))?;

    let file = std::fs::File::open(src)
        .map_err(|e| hermes_core::AgentError::Io(format!("Cannot open {}: {}", path, e)))?;
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    archive
        .unpack(&hermes_dir)
        .map_err(|e| hermes_core::AgentError::Io(format!("Extract error: {}", e)))?;

    println!(
        "Import complete. Files restored to {}",
        hermes_dir.display()
    );
    Ok(())
}

/// Handle `hermes version`.
pub fn handle_cli_version() -> Result<(), hermes_core::AgentError> {
    println!("hermes {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

/// Handle `hermes meeting <action> [options]`.
///
/// Actions:
/// - `notes --audio <path> [--title "..."]`  — process an audio file offline
/// - `record [--mode offline|realtime] [--title "..."]`  — start live recording
pub async fn handle_cli_meeting(
    action: Option<String>,
    audio: Option<String>,
    title: Option<String>,
    mode: Option<String>,
    diarize: bool,
) -> Result<(), hermes_core::AgentError> {
    use hermes_config::{DiarizationProvider, MeetingConfig, MeetingTranscriptionMode, SttConfig};
    use hermes_tools::tools::meeting_notes::run_offline_pipeline;

    let hermes_home = hermes_config::hermes_home();
    let action = action.as_deref().unwrap_or("notes");

    match action {
        "notes" => {
            let audio_path = audio.ok_or_else(|| {
                hermes_core::AgentError::Config("meeting notes requires --audio <path>".into())
            })?;
            let title = title.unwrap_or_else(|| "会议".to_string());

            let mut meeting_cfg = MeetingConfig::default();
            if let Some(m) = mode.as_deref() {
                meeting_cfg.transcription_mode = Some(match m {
                    "realtime" => MeetingTranscriptionMode::Realtime,
                    _ => MeetingTranscriptionMode::Offline,
                });
            }
            if diarize {
                meeting_cfg.diarization_provider = Some(DiarizationProvider::Pyannote);
            }

            let llm_base = std::env::var("MEETING_LLM_BASE_URL")
                .or_else(|_| std::env::var("OPENAI_BASE_URL"))
                .unwrap_or_else(|_| "https://api.openai.com/v1".into());
            let llm_key = std::env::var("MEETING_LLM_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .unwrap_or_default();
            let llm_model =
                std::env::var("MEETING_LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());

            println!("▶ Generating meeting notes for: {}", audio_path);
            let notes = run_offline_pipeline(
                &audio_path,
                &title,
                SttConfig::default(),
                meeting_cfg,
                &llm_base,
                &llm_key,
                &llm_model,
                &hermes_home,
                |state| {
                    use hermes_tools::tools::meeting_notes::SummarizeState;
                    match &state {
                        SummarizeState::Transcribing => println!("  ⟳ 转录中…"),
                        SummarizeState::Diarizing => println!("  ⟳ 说话人识别中…"),
                        SummarizeState::SummarizingChunk(i, n) => println!("  ⟳ 总结片段 {i}/{n}…"),
                        SummarizeState::MergingSummaries => println!("  ⟳ 合并摘要…"),
                        SummarizeState::WritingMemory => println!("  ⟳ 写入记忆…"),
                        SummarizeState::Done => println!("  ✓ 完成"),
                        SummarizeState::Warning(w) => println!("  ⚠ {w}"),
                    }
                },
            )
            .await
            .map_err(|e| hermes_core::AgentError::Config(e.to_string()))?;

            println!("\n# {}\n", notes.title);
            println!("**日期**: {}", notes.date);
            println!("\n## 摘要\n{}", notes.summary);

            if !notes.key_decisions.is_empty() {
                println!("\n## 关键决策");
                for d in &notes.key_decisions {
                    println!("- {d}");
                }
            }
            if !notes.action_items.is_empty() {
                println!("\n## 行动项");
                for a in &notes.action_items {
                    println!("- {a}");
                }
            }
            if !notes.risks.is_empty() {
                println!("\n## 风险");
                for r in &notes.risks {
                    println!("- {r}");
                }
            }
            if let Some(tf) = &notes.transcript_file {
                println!("\n📁 转录文件: {tf}");
            }
            println!("\n✓ 已写入记忆系统 (holographic facts + MEMORY.md)");
        }
        "record" => {
            println!("⚠ `hermes meeting record` requires a microphone source (Phase 2 runtime).");
            println!("  Run `hermes meeting notes --audio <recorded.wav>` after recording.");
        }
        _ => {
            println!("Unknown meeting action '{action}'. Available: notes, record");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn insert_quick_command(app: &mut App, name: &str, command: hermes_config::QuickCommandConfig) {
        let mut config = (*app.config).clone();
        config.quick_commands.insert(name.to_string(), command);
        app.config = Arc::new(config);
    }

    #[tokio::test]
    async fn quick_alias_rewrites_to_builtin_and_passes_args() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;
        insert_quick_command(
            &mut app,
            "sc",
            hermes_config::QuickCommandConfig {
                kind: "alias".to_string(),
                target: Some("/queue".to_string()),
                ..Default::default()
            },
        );

        handle_slash_command(&mut app, "/sc", &["some", "args"])
            .await
            .expect("alias command");

        assert!(latest_ui_assistant_text(&app).contains("some args"));
    }

    #[test]
    fn test_autocomplete_empty() {
        let results = autocomplete("");
        assert_eq!(results.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn test_autocomplete_partial() {
        let results = autocomplete("/m");
        assert!(results.contains(&"/model"));
    }

    #[test]
    fn test_contextual_autocomplete_swarm_subcommands() {
        let results = autocomplete_contextual("/swarm ");
        assert!(results.contains(&"/swarm status ".to_string()));
        assert!(results.contains(&"/swarm run ".to_string()));
    }

    #[test]
    fn test_contextual_autocomplete_swarm_nested_modes() {
        let results = autocomplete_contextual("/swarm plan ");
        assert!(results.contains(&"/swarm plan graph ".to_string()));
        assert!(results.contains(&"/swarm plan sequential ".to_string()));
    }

    #[test]
    fn test_contextual_autocomplete_objective_behavior_modes() {
        let results = autocomplete_contextual("/objective behavior ");
        assert!(results.contains(&"/objective behavior strict ".to_string()));
        assert!(results.contains(&"/objective behavior sigma ".to_string()));
    }

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

    #[tokio::test]
    async fn promoted_queue_command_shows_usage_and_status() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let usage = handle_queue_command(&mut app, &[]).expect("queue usage");
        assert_eq!(usage, CommandResult::Handled);
        assert!(latest_ui_assistant_text(&app).contains("Usage: /queue <prompt>"));

        let status = handle_queue_command(&mut app, &["status"]).expect("queue status");
        assert_eq!(status, CommandResult::Handled);
        assert!(latest_ui_assistant_text(&app).contains("Background queue status:"));
    }

    #[tokio::test]
    async fn slash_auth_status_command_is_handled() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_slash_command(&mut app, "/auth", &["status"])
            .await
            .expect("auth status");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("Auth status"));
    }

    #[tokio::test]
    async fn slash_runbook_and_telemetry_commands_are_handled() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let runbook = handle_slash_command(&mut app, "/runbook", &["list"])
            .await
            .expect("runbook list");
        assert_eq!(runbook, CommandResult::Handled);
        assert!(latest_ui_assistant_text(&app).contains("Runbooks"));

        let telemetry = handle_slash_command(&mut app, "/telemetry", &["status"])
            .await
            .expect("telemetry status");
        assert_eq!(telemetry, CommandResult::Handled);
        assert!(latest_ui_assistant_text(&app).contains("Telemetry snapshot"));
    }

    #[tokio::test]
    async fn slash_agents_pause_resume_and_status_are_handled() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;
        crate::env_vars::remove_var("HERMES_DELEGATION_PAUSED");

        let status = handle_slash_command(&mut app, "/agents", &["status"])
            .await
            .expect("agents status");
        assert_eq!(status, CommandResult::Handled);
        assert!(latest_ui_assistant_text(&app).contains("Delegation spawning: active"));

        let pause = handle_slash_command(&mut app, "/agents", &["pause"])
            .await
            .expect("agents pause");
        assert_eq!(pause, CommandResult::Handled);
        assert_eq!(
            std::env::var("HERMES_DELEGATION_PAUSED").ok().as_deref(),
            Some("1")
        );
        assert!(latest_ui_assistant_text(&app).contains("paused for this runtime"));

        let resume = handle_slash_command(&mut app, "/agents", &["resume"])
            .await
            .expect("agents resume");
        assert_eq!(resume, CommandResult::Handled);
        assert_eq!(
            std::env::var("HERMES_DELEGATION_PAUSED").ok().as_deref(),
            Some("0")
        );
        assert!(latest_ui_assistant_text(&app).contains("resumed for this runtime"));
    }

    #[tokio::test]
    async fn promoted_paste_command_uses_test_clipboard_override() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        crate::env_vars::set_var("HERMES_TEST_CLIPBOARD_TEXT", "alpha clipboard payload");
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_paste_command(&mut app, &[]).expect("paste command");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("Clipboard captured:"));
        assert!(output.contains("alpha clipboard payload"));
    }

    #[tokio::test]
    async fn promoted_gquota_command_emits_provider_diagnostics() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_gquota_command(&mut app, &[]).await.expect("gquota");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("Gemini quota/auth diagnostics"));
        assert!(output.contains("active provider:"));
    }

    #[tokio::test]
    async fn promoted_image_command_queues_and_consumes_hint() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result =
            handle_image_command(&mut app, &["/tmp/example-image.png"]).expect("image queue");
        assert_eq!(result, CommandResult::Handled);
        assert_eq!(app.pending_image_hint(), Some("/tmp/example-image.png"));
        assert!(latest_ui_assistant_text(&app).contains("Image hint queued"));

        let prepared = app.prepare_user_message("analyze the screenshot");
        assert!(prepared.starts_with("[IMAGE_HINT] path=/tmp/example-image.png"));
        assert!(app.pending_image_hint().is_none());

        let cleared = handle_image_command(&mut app, &["clear"]).expect("image clear");
        assert_eq!(cleared, CommandResult::Handled);
        assert!(latest_ui_assistant_text(&app).contains("Cleared pending image hint"));
    }

    #[tokio::test]
    async fn promoted_feedback_command_writes_feedback_log() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_feedback_command(&mut app, &["solid", "repro", "steps"])
            .expect("feedback write");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("Feedback captured in"));

        let path = feedback_log_path();
        let raw = std::fs::read_to_string(&path).expect("read feedback log");
        assert!(raw.contains("\"note\":\"solid repro steps\""));
    }

    #[tokio::test]
    async fn promoted_debug_dump_command_writes_session_snapshot() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        app.messages.push(hermes_core::Message::user("hello"));
        let result = handle_debug_dump_command(&mut app, &[]).expect("debug dump");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("Debug snapshot written."));

        let sessions_dir = app.state_root.join("sessions");
        let count = std::fs::read_dir(sessions_dir)
            .expect("sessions dir")
            .filter_map(|entry| entry.ok())
            .count();
        assert!(count > 0);
    }

    #[tokio::test]
    async fn promoted_plan_status_command_emits_queue_summary() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_plan_command(&mut app, &["status"]).expect("plan status");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("Planner queue status"));
        assert!(output.contains("queued="));
    }

    #[tokio::test]
    async fn promoted_lsp_status_command_emits_index_details() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let result = handle_lsp_command(&mut app, &["status"]).expect("lsp status");
        assert_eq!(result, CommandResult::Handled);
        let output = latest_ui_assistant_text(&app);
        assert!(output.contains("LSP/code-index status"));
        assert!(output.contains("code_index_enabled"));
    }

    #[tokio::test]
    async fn promoted_approve_and_deny_commands_operate_on_pairing_store() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        let store = PairingStore::open_default();
        store
            .save(&[crate::pairing_store::PairedDevice {
                device_id: "device-01".to_string(),
                name: Some("Test device".to_string()),
                status: PairingStatus::Pending,
                created_at: chrono::Utc::now().to_rfc3339(),
                last_seen: None,
                shared_secret: None,
            }])
            .expect("seed pairing store");

        handle_approve_command(&mut app, &["device-01"]).expect("approve");
        assert!(latest_ui_assistant_text(&app).contains("Approved device 'device-01'"));

        handle_deny_command(&mut app, &["device-01"]).expect("deny");
        assert!(latest_ui_assistant_text(&app).contains("Revoked device 'device-01'"));
    }

    #[test]
    fn test_acp_history_to_messages_preserves_multimodal_user_content_marker() {
        let history = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "check this"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}}
            ]
        })];
        let messages = acp_history_to_messages(&history, "");
        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_deref().unwrap_or("");
        assert!(content.starts_with(ACP_MULTIMODAL_PREFIX));
    }

    #[test]
    fn test_acp_history_to_messages_flattens_assistant_parts_to_text() {
        let history = vec![serde_json::json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "done"},
                {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}}
            ]
        })];
        let messages = acp_history_to_messages(&history, "");
        assert_eq!(messages.len(), 1);
        let content = messages[0].content.as_deref().unwrap_or("");
        assert!(content.contains("done"));
        assert!(content.contains("Attached image"));
    }

    #[test]
    fn test_pet_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/pet"));
        let results = autocomplete("/pe");
        assert!(results.contains(&"/pet"));
    }

    #[test]
    fn test_kanban_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/kanban"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/tasks"));
        let results = autocomplete("/kan");
        assert!(results.contains(&"/kanban"));
    }

    #[test]
    fn test_parse_kanban_add_defaults() {
        let input = parse_kanban_add(&["Ship", "kanban"]).expect("parse");
        assert_eq!(input.title, "Ship kanban");
        assert_eq!(input.lane, KanbanLane::Todo);
        assert_eq!(input.priority, 3);
    }

    #[test]
    fn test_parse_kanban_add_flags() {
        let input = parse_kanban_add(&[
            "Task",
            "--lane",
            "doing",
            "--priority",
            "2",
            "--assignee",
            "runner",
            "--depends",
            "K-0001,K-0002",
            "--desc",
            "note",
        ])
        .expect("parse");
        assert_eq!(input.title, "Task");
        assert_eq!(input.lane, KanbanLane::Doing);
        assert_eq!(input.priority, 2);
        assert_eq!(input.assignee.as_deref(), Some("runner"));
        assert_eq!(input.depends_on, vec!["K-0001", "K-0002"]);
        assert_eq!(input.description.as_deref(), Some("note"));
    }

    #[test]
    fn test_reset_alias_maps_to_new() {
        assert_eq!(canonical_command("/reset"), "/new");
    }

    #[tokio::test]
    async fn slash_reset_rotates_session_id_like_new() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;
        app.messages = vec![hermes_core::Message::user("prior turn")];
        let old_session_id = app.session_id.clone();

        let result = handle_slash_command(&mut app, "/reset", &[])
            .await
            .expect("reset handled");
        assert_eq!(result, CommandResult::Handled);
        assert_ne!(app.session_id, old_session_id);
        assert!(app.messages.is_empty());
        assert!(latest_ui_assistant_text(&app).contains("Session reset"));
    }

    #[test]
    fn test_mission_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/mission"));
        let results = autocomplete("/mis");
        assert!(results.contains(&"/mission"));
    }

    #[test]
    fn test_skins_alias_maps_to_skin() {
        assert_eq!(canonical_command("/skins"), "/skin");
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/skins"));
    }

    #[test]
    fn test_whoami_alias_maps_to_profile() {
        assert_eq!(canonical_command("/whoami"), "/profile");
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/whoami"));
        let results = autocomplete("/who");
        assert!(results.contains(&"/whoami"));
    }

    #[test]
    fn test_resume_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/resume"));
        let results = autocomplete("/res");
        assert!(results.contains(&"/resume"));
    }

    #[test]
    fn test_timetravel_command_and_alias_are_registered() {
        assert!(
            SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == "/timetravel")
        );
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/tt"));
        assert_eq!(canonical_command("/tt"), "/timetravel");
        let results = autocomplete("/time");
        assert!(results.contains(&"/timetravel"));
    }

    #[test]
    fn test_simulate_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/simulate"));
        let results = autocomplete("/sim");
        assert!(results.contains(&"/simulate"));
    }

    #[test]
    fn test_qos_and_eval_commands_are_registered() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/qos"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/eval"));
        let qos = autocomplete("/qo");
        assert!(qos.contains(&"/qos"));
        let eval = autocomplete("/eva");
        assert!(eval.contains(&"/eval"));
    }

    #[test]
    fn test_sessions_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/sessions"));
        let results = autocomplete("/sess");
        assert!(results.contains(&"/sessions"));
    }

    #[test]
    fn test_browser_command_is_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/browser"));
        let results = autocomplete("/bro");
        assert!(results.contains(&"/browser"));
    }

    #[test]
    fn test_p0_p1_surface_commands_registered_and_completable() {
        for command in [
            "/commands",
            "/boot",
            "/walkthrough",
            "/triage",
            "/subconscious",
            "/integrations",
        ] {
            assert!(
                SLASH_COMMANDS.iter().any(|(name, _)| *name == command),
                "missing slash command: {command}"
            );
        }
        assert_eq!(canonical_command("/onboard"), "/walkthrough");
        assert!(autocomplete("/boo").contains(&"/boot"));
        assert!(autocomplete("/wal").contains(&"/walkthrough"));
        assert!(autocomplete("/tri").contains(&"/triage"));
        assert!(autocomplete("/subc").contains(&"/subconscious"));
        assert!(autocomplete("/inte").contains(&"/integrations"));
    }

    #[tokio::test]
    async fn p0_walkthrough_and_integrations_commands_emit_expected_sections() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        handle_slash_command(&mut app, "/walkthrough", &["start", "quick"])
            .await
            .expect("walkthrough start");
        let started = latest_ui_assistant_text(&app);
        assert!(started.contains("walkthrough"));
        assert!(started.contains("Use `/walkthrough done <step-id>`"));

        handle_slash_command(&mut app, "/integrations", &["status"])
            .await
            .expect("integrations status");
        let integrations = latest_ui_assistant_text(&app);
    }
    #[test]

    fn p1_trigger_triage_escalates_high_severity_events() {
        let _guard = env_test_lock();
        crate::env_vars::set_var("HERMES_TRIGGER_TRIAGE_MODE", "strict");
        let assessment = evaluate_trigger_triage(
            "webhook",
            "critical outage with secret key leak and panic in runtime",
        );
        assert_eq!(assessment.decision, TriggerTriageDecision::Escalate);
        assert!(assessment.requires_approval);
        assert!(assessment.severity >= 7);
        crate::env_vars::remove_var("HERMES_TRIGGER_TRIAGE_MODE");
    }

    #[test]
    fn p2_trigger_triage_feedback_persists_bias_and_influences_scoring() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let baseline = evaluate_trigger_triage("webhook", "timeout error while polling");
        let feedback_assessment = evaluate_trigger_triage("webhook", "critical outage and panic");
        append_triage_learning_feedback(
            "webhook",
            "critical outage and panic",
            "critical",
            &feedback_assessment,
        )
        .expect("append triage feedback");
        let (bias, _) = triage_learning_bias("webhook", "timeout error while polling");
        assert!(bias > 0);
        let after = evaluate_trigger_triage("webhook", "timeout error while polling");
        assert!(after.severity >= baseline.severity);
        assert!(
            trigger_triage_learning_state_path().exists(),
            "triage learning state file should be persisted"
        );
    }

    #[tokio::test]
    async fn p2_subconscious_profile_dry_run_blocks_high_risk_tasks() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;
        let now = chrono::Utc::now().to_rfc3339();
        let state = SubconsciousQueueState {
            tasks: vec![SubconsciousTask {
                id: "sc-risky".to_string(),
                source: "test".to_string(),
                prompt: "rotate key and deploy to prod".to_string(),
                score: 4.2,
                risk: "high".to_string(),
                requires_approval: false,
                status: "pending".to_string(),
                job_id: None,
                created_at: now.clone(),
                updated_at: now,
            }],
        };
        save_subconscious_state(&state).expect("save subconscious state");

        handle_slash_command(
            &mut app,
            "/subconscious",
            &["run", "1", "--dry-run", "profile=strict"],
        )
        .await
        .expect("subconscious dry-run");
        let out = latest_ui_assistant_text(&app);
        assert!(out.contains("Dry-run subconscious run profile=strict"));
        assert!(out.contains("blocked=1"));
    }

    #[tokio::test]
    async fn p2_walkthrough_insights_persists_events() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        handle_slash_command(&mut app, "/walkthrough", &["start", "quick"])
            .await
            .expect("walkthrough start");
        handle_slash_command(&mut app, "/walkthrough", &["done", "boot-gate"])
            .await
            .expect("walkthrough done");
        handle_slash_command(&mut app, "/walkthrough", &["insights"])
            .await
            .expect("walkthrough insights");
        let out = latest_ui_assistant_text(&app);
        assert!(out.contains("Walkthrough insights"));
        assert!(out.contains("resume_hint:"));
        assert!(walkthrough_events_path().exists());
    }

    #[tokio::test]
    async fn p2_integrations_snapshot_and_repair_commands_work() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        handle_slash_command(&mut app, "/integrations", &["snapshot"])
            .await
            .expect("integrations snapshot");
        let snapshot_out = latest_ui_assistant_text(&app);
        assert!(snapshot_out.contains("Integration snapshot exported"));

        handle_slash_command(&mut app, "/integrations", &["repair"])
            .await
            .expect("integrations repair");
        let repair_out = latest_ui_assistant_text(&app);
        assert!(repair_out.contains("Integrations repair plan"));
    }

    #[test]
    fn p2_oauth_runtime_gate_manifest_override_is_honored() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let manifest = tmp.path().join("oauth-manifest.json");
        std::fs::write(
            &manifest,
            r#"{
  "default_min_version": "99.0.0",
  "required_oauth_provider_ids": ["nous"],
  "provider_min_versions": { "nous": "99.0.0" }
}"#,
        )
        .expect("write manifest");
        crate::env_vars::set_var("HERMES_OAUTH_GATE_MANIFEST_PATH", &manifest);
        let (ok, detail) = policy::oauth_runtime_gate_for_provider("nous").expect("oauth gate");
        assert!(!ok);
        assert!(detail.contains("required>=99.0.0"));
        assert!(detail.contains("oauth-manifest.json"));
        crate::env_vars::remove_var("HERMES_OAUTH_GATE_MANIFEST_PATH");
    }

    #[test]
    fn test_debug_alias_maps_to_debug_dump() {
        assert_eq!(canonical_command("/debug"), "/debug-dump");
    }

    #[test]
    fn test_upstream_compat_aliases_are_mapped() {
        assert_eq!(canonical_command("/topic"), "/title");
        assert_eq!(canonical_command("/reload-skills"), "/reload");
        assert_eq!(canonical_command("/reload_skills"), "/reload");
        assert_eq!(canonical_command("/swarms"), "/swarm");
        assert_eq!(canonical_command("/summary"), "/recap");
        assert_eq!(canonical_command("/whoami"), "/profile");
        assert_eq!(canonical_command("/footer"), "/statusbar");
        assert_eq!(canonical_command("/indicator"), "/statusbar");
        assert_eq!(canonical_command("/tasks"), "/kanban");
        assert_eq!(canonical_command("/kanban"), "/kanban");
        assert_eq!(canonical_command("/busy"), "/status");
        assert_eq!(canonical_command("/bg"), "/background");
        assert_eq!(canonical_command("/curator"), "/curator");
        assert_eq!(canonical_command("/tt"), "/timetravel");
        assert_eq!(canonical_command("/rb"), "/runbook");
    }

    #[test]
    fn p3_swarm_commands_registered_and_completable() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/swarm"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/swarms"));
        assert!(autocomplete("/swa").contains(&"/swarm"));
    }

    #[tokio::test]
    async fn p3_swarm_status_plan_run_cancel_surface_is_handled() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuard::new(tmp.path());
        let mut app = build_test_app_with_stream(tmp.path()).await;

        handle_slash_command(&mut app, "/swarm", &["status"])
            .await
            .expect("swarm status");
        let status = latest_ui_assistant_text(&app);
        assert!(status.contains("Swarm runtime"));

        handle_slash_command(&mut app, "/swarm", &["plan", "graph"])
            .await
            .expect("swarm plan");
        let plan = latest_ui_assistant_text(&app);
        assert!(plan.contains("Swarm execution plan"));
        assert!(plan.contains("\"mode\": \"graph\""));

        handle_slash_command(&mut app, "/swarm", &["on"])
            .await
            .expect("swarm on");
        handle_slash_command(&mut app, "/swarm", &["run", "4", "sequential"])
            .await
            .expect("swarm run");
        assert!(app.quorum_armed_once, "swarm run should arm quorum fanout");
        let run_msg = latest_ui_assistant_text(&app);
        assert!(run_msg.contains("Swarm run armed."));
        assert!(run_msg.contains("mode=sequential"));

        handle_slash_command(&mut app, "/swarm", &["cancel"])
            .await
            .expect("swarm cancel");
        assert!(!app.quorum_armed_once, "cancel should disarm run");
    }

    #[test]
    fn repo_review_budget_profile_application_sets_expected_env() {
        let _guard = env_test_lock();
        apply_repo_review_budget_profile(RepoReviewBudgetProfile::Aggressive);
        let runtime = RepoReviewBudgetRuntime::from_env();
        assert_eq!(runtime.profile, RepoReviewBudgetProfile::Aggressive);
        assert_eq!(runtime.repeat_threshold, 1);
        assert_eq!(runtime.low_signal_threshold, 1);
        assert_eq!(runtime.keep_repeat, 1);
        assert_eq!(runtime.keep_low_signal, 1);
        assert!(runtime.min_signal_score >= 0.34);

        apply_repo_review_budget_profile(RepoReviewBudgetProfile::Balanced);
        let runtime_balanced = RepoReviewBudgetRuntime::from_env();
        assert_eq!(runtime_balanced.profile, RepoReviewBudgetProfile::Balanced);
        assert_eq!(runtime_balanced.repeat_threshold, 2);
        assert_eq!(runtime_balanced.low_signal_threshold, 2);
    }

    #[test]
    fn task_depth_profile_application_sets_expected_env() {
        let _guard = env_test_lock();
        apply_task_depth_profile(TaskDepthProfile::Max);
        assert_eq!(
            std::env::var("HERMES_TASK_DEPTH_PROFILE").ok().as_deref(),
            Some("max")
        );
        assert_eq!(
            std::env::var("HERMES_MAX_ITERATIONS").ok().as_deref(),
            Some("250")
        );
        assert_eq!(
            std::env::var("HERMES_REPO_REVIEW_BUDGET_PROFILE")
                .ok()
                .as_deref(),
            Some("off")
        );

        apply_task_depth_profile(TaskDepthProfile::Balanced);
        assert_eq!(
            std::env::var("HERMES_TASK_DEPTH_PROFILE").ok().as_deref(),
            Some("balanced")
        );
        assert_eq!(
            std::env::var("HERMES_MAX_ITERATIONS").ok().as_deref(),
            Some("250")
        );
    }

    #[test]
    fn test_recap_and_context_commands_are_registered() {
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/recap"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/context"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/auth"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/telemetry"));
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/runbook"));
        let recap = autocomplete("/rec");
        assert!(recap.contains(&"/recap"));
        let context = autocomplete("/cont");
        assert!(context.contains(&"/context"));
        let auth = autocomplete("/au");
        assert!(auth.contains(&"/auth"));
        let telemetry = autocomplete("/tele");
        assert!(telemetry.contains(&"/telemetry"));
        let runbook = autocomplete("/runb");
        assert!(runbook.contains(&"/runbook"));
    }

    #[test]
    fn alpha_loop_defaults_are_written_and_loadable() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        crate::env_vars::set_var("HERMES_HOME", tmp.path());
        let path = crate::alpha_runtime::write_default_alpha_loops(true).expect("write defaults");
        assert!(path.exists());
        let loops = crate::alpha_runtime::load_alpha_loops().expect("load defaults");
        assert_eq!(loops.len(), 3);
        assert!(loops.iter().any(|l| l.id == "primary-objective-loop"));
        assert!(loops.iter().all(|l| !l.trading_sensitive));
        crate::env_vars::remove_var("HERMES_HOME");
    }

    #[test]
    fn test_autocomplete_includes_evolve() {
        let results = autocomplete("/evo");
        assert!(results.contains(&"/evolve"));
    }

    #[test]
    fn summarize_self_evolution_report_formats_fields() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("self-evolution-loop-test.json");
        std::fs::write(
            &path,
            r#"{
  "ok": false,
  "generated_at": "2026-05-02T00:00:00Z",
  "summary": { "intelligence_index": 66.67 },
  "recommendations": [{"id":"PARITY_DRIFT"}]
}"#,
        )
        .expect("write report");
        let line = summarize_self_evolution_report(&path, "self_evolution").expect("summary");
        assert!(line.contains("self_evolution=fail"));
        assert!(line.contains("idx=66.67"));
        assert!(line.contains("recs=1"));
    }

    #[test]
    fn self_evolution_recommendations_extracts_lines() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("self-evolution-loop-test.json");
        std::fs::write(
            &path,
            r#"{
  "recommendations": [
    {
      "id": "EVAL_REGRESSION",
      "severity": "P0",
      "title": "Recover eval trend before promotion",
      "command": "python3 scripts/run-eval-trend-gate.py --json"
    }
  ]
}"#,
        )
        .expect("write report");
        let lines = self_evolution_recommendations(&path);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("EVAL_REGRESSION"));
        assert!(lines[0].contains("python3 scripts/run-eval-trend-gate.py --json"));
    }

    #[test]
    fn summarize_performance_autopilot_report_formats_fields() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("performance-autopilot-test.json");
        std::fs::write(
            &path,
            r#"{
  "ok": true,
  "generated_at": "2026-05-08T00:00:00Z",
  "recommendations": [
    {"id":"PERF_STABLE", "severity":"P3", "title":"stable", "recommendation":"none"}
  ]
}"#,
        )
        .expect("write report");
        let line = summarize_performance_autopilot_report(&path, "autopilot").expect("summary");
        assert!(line.contains("autopilot=pass"));
        assert!(line.contains("recs=1"));
        assert!(line.contains("severe=0"));
    }

    #[test]
    fn performance_autopilot_recommendations_extract_lines() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("performance-autopilot-test.json");
        std::fs::write(
            &path,
            r#"{
  "recommendations": [
    {
      "id":"HOTPATH_SLOW",
      "severity":"P1",
      "title":"Tool policy hot-path latency above target",
      "recommendation":"Keep HERMES_TOOL_POLICY_PRESET=standard"
    }
  ]
}"#,
        )
        .expect("write report");
        let recs = performance_autopilot_recommendations(&path);
        assert_eq!(recs.len(), 1);
        assert!(recs[0].contains("HOTPATH_SLOW"));
        assert!(recs[0].contains("recommendation"));
    }

    #[test]
    fn parse_env_file_kv_ignores_comments() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("autopilot.env");
        std::fs::write(
            &path,
            "# comment\nHERMES_TOOL_POLICY_PRESET=standard\n \nINVALID_LINE\nHERMES_REPLAY_ENABLED=1\n",
        )
        .expect("write env");
        let kvs = parse_env_file_kv(&path);
        assert_eq!(kvs.len(), 2);
        assert_eq!(kvs[0].0, "HERMES_TOOL_POLICY_PRESET");
        assert_eq!(kvs[1].0, "HERMES_REPLAY_ENABLED");
    }

    #[test]
    fn test_autocomplete_includes_autopilot() {
        let results = autocomplete("/auto");
        assert!(results.contains(&"/autopilot"));
    }

    #[test]
    fn canonical_command_maps_pilot_alias() {
        assert_eq!(canonical_command("/pilot"), "/autopilot");
    }

    #[test]
    fn test_autocomplete_includes_raw_controls() {
        let results = autocomplete("/ra");
        assert!(results.contains(&"/raw"));
    }

    #[test]
    fn test_autocomplete_ops_control_plane() {
        let results = autocomplete("/op");
        assert!(results.contains(&"/ops"));
    }

    #[test]
    fn test_autocomplete_fuzzy_prefers_close_matches() {
        let results = autocomplete("/mdl");
        assert!(!results.is_empty());
        assert_eq!(results[0], "/model");
    }

    #[test]
    fn test_autocomplete_matches_description_terms() {
        let results = autocomplete("/quota");
        assert!(results.contains(&"/gquota"));
    }

    #[test]
    fn test_autocomplete_exact() {
        let results = autocomplete("/help");
        assert!(!results.is_empty());
        assert_eq!(results[0], "/help");
    }

    #[test]
    fn test_autocomplete_no_match() {
        let results = autocomplete("/xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_help_for_known_command() {
        assert!(help_for("/help").is_some());
        assert!(help_for("/model").is_some());
    }

    #[test]
    fn test_help_for_unknown_command() {
        assert!(help_for("/unknown").is_none());
    }

    #[test]
    fn test_command_result_equality() {
        assert_eq!(CommandResult::Handled, CommandResult::Handled);
        assert_ne!(CommandResult::Handled, CommandResult::Quit);
    }

    #[tokio::test]
    async fn test_mcp_sentrux_setup_syncs_json_and_yaml() {
        let tmp = tempdir().expect("tempdir");
        let config_dir = tmp.path().join("hermes-home");
        std::fs::create_dir_all(&config_dir).expect("create config dir");

        upsert_sentrux_mcp_profile(&config_dir).expect("sentrux setup helper");

        let mcp_json = config_dir.join("mcp_servers.json");
        assert!(mcp_json.exists(), "mcp_servers.json should be created");
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&mcp_json).expect("read mcp_servers.json"),
        )
        .expect("parse mcp json");
        let sentrux = json
            .get(SENTRUX_MCP_SERVER_NAME)
            .expect("sentrux entry should exist");
        assert_eq!(
            sentrux.get("command").and_then(|v| v.as_str()),
            Some(SENTRUX_MCP_COMMAND)
        );
        assert_eq!(
            sentrux
                .get("args")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str()),
            Some(SENTRUX_MCP_ARG)
        );
        assert_eq!(
            sentrux
                .get("supports_parallel_tool_calls")
                .and_then(|v| v.as_bool()),
            Some(true)
        );

        let cfg = hermes_config::load_user_config_file(&config_dir.join("config.yaml"))
            .expect("load config.yaml");
        assert!(
            cfg.mcp_servers
                .iter()
                .any(|entry| entry.name == SENTRUX_MCP_SERVER_NAME
                    && entry.command.as_deref() == Some("sentrux --mcp")
                    && entry.supports_parallel_tool_calls),
            "config.yaml mcp_servers should include sentrux command"
        );
    }

    #[tokio::test]
    async fn test_mcp_sentrux_remove_syncs_json_and_yaml() {
        let tmp = tempdir().expect("tempdir");
        let config_dir = tmp.path().join("hermes-home");
        std::fs::create_dir_all(&config_dir).expect("create config dir");

        upsert_sentrux_mcp_profile(&config_dir).expect("sentrux setup helper");
        remove_sentrux_mcp_profile(&config_dir).expect("sentrux remove helper");

        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(config_dir.join("mcp_servers.json")).expect("read mcp json"),
        )
        .expect("parse mcp json");
        assert!(
            json.get(SENTRUX_MCP_SERVER_NAME).is_none(),
            "mcp_servers.json should remove sentrux"
        );

        let cfg = hermes_config::load_user_config_file(&config_dir.join("config.yaml"))
            .expect("load config.yaml");
        assert!(
            cfg.mcp_servers
                .iter()
                .all(|entry| entry.name != SENTRUX_MCP_SERVER_NAME),
            "config.yaml mcp_servers should remove sentrux"
        );
    }

    #[test]
    fn test_default_skill_tap_present_in_merged_list() {
        let merged = merged_skill_taps(&[]);
        assert!(
            merged
                .iter()
                .any(|tap| tap == "https://github.com/MiniMax-AI/cli::skill")
        );
    }

    #[test]
    fn test_autoresearch_default_skill_tap_present_in_merged_list() {
        let merged = merged_skill_taps(&[]);
        assert!(
            merged
                .iter()
                .any(|tap| tap == "https://github.com/github/awesome-copilot::skills")
        );
    }

    #[test]
    fn test_nous_official_default_skill_taps_present_in_merged_list() {
        let merged = merged_skill_taps(&[]);
        assert!(
            merged
                .iter()
                .any(|tap| tap == "https://github.com/NousResearch/hermes-agent::skills")
        );
        assert!(
            merged
                .iter()
                .any(|tap| tap == "https://github.com/NousResearch/hermes-agent::optional-skills")
        );
    }

    #[test]
    fn test_official_skill_path_candidates_cover_skills_and_optional() {
        let candidates = official_skill_path_candidates("creative/comfyui");
        assert_eq!(
            candidates,
            vec![
                "skills/creative/comfyui".to_string(),
                "optional-skills/creative/comfyui".to_string(),
            ]
        );

        let rooted = official_skill_path_candidates("optional-skills/security/1password");
        assert_eq!(
            rooted,
            vec!["optional-skills/security/1password".to_string()]
        );
    }

    #[test]
    fn test_mattpocock_default_skill_tap_present_in_merged_list() {
        let merged = merged_skill_taps(&[]);
        assert!(
            merged
                .iter()
                .any(|tap| tap == "https://github.com/mattpocock/skills::skills")
        );
    }

    #[test]
    fn test_merged_skill_taps_deduplicates_default() {
        let merged = merged_skill_taps(&vec![
            "https://github.com/MiniMax-AI/cli::skill".to_string(),
        ]);
        assert_eq!(
            merged
                .iter()
                .filter(|tap| tap.as_str() == "https://github.com/MiniMax-AI/cli::skill")
                .count(),
            1
        );
    }

    #[test]
    fn parse_skill_tap_spec_parses_github_url_with_override() {
        let parsed =
            parse_skill_tap_spec("https://github.com/openai/skills::skills").expect("tap parse");
        assert_eq!(parsed.repo, "openai/skills");
        assert_eq!(parsed.path, "skills");
    }

    #[test]
    fn parse_skill_tap_spec_parses_tree_url() {
        let parsed = parse_skill_tap_spec("https://github.com/anthropics/skills/tree/main/skills")
            .expect("tap parse");
        assert_eq!(parsed.repo, "anthropics/skills");
        assert_eq!(parsed.path, "skills");
    }

    #[test]
    fn read_skill_taps_accepts_upstream_object_shape() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("skill_taps.json");
        std::fs::write(
            &path,
            r#"{
  "taps": [
    { "repo": "MiniMax-AI/cli", "path": "skill/" },
    { "repo": "openai/skills", "path": "skills/" },
    { "repo": "anthropics/skills" },
    { "url": "https://github.com/garrytan/gstack::" }
  ]
}"#,
        )
        .expect("write");

        let taps = read_skill_taps(&path);
        assert!(taps.contains(&"https://github.com/MiniMax-AI/cli::skill".to_string()));
        assert!(taps.contains(&"https://github.com/openai/skills::skills".to_string()));
        assert!(taps.contains(&"https://github.com/anthropics/skills::skills".to_string()));
        assert!(taps.contains(&"https://github.com/garrytan/gstack::".to_string()));
    }

    #[test]
    fn write_skill_taps_writes_canonical_object_shape() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("skill_taps.json");
        let taps = vec![
            "https://github.com/MiniMax-AI/cli::skill".to_string(),
            "https://github.com/github/awesome-copilot::skills".to_string(),
            "https://github.com/garrytan/gstack::".to_string(),
        ];
        write_skill_taps(&path, &taps).expect("write taps");

        let raw = std::fs::read_to_string(&path).expect("read");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
        let arr = value
            .get("taps")
            .and_then(|v| v.as_array())
            .expect("taps array");
        assert_eq!(arr.len(), 3);

        let first = arr[0].as_object().expect("first object");
        assert_eq!(
            first.get("repo").and_then(|v| v.as_str()),
            Some("MiniMax-AI/cli")
        );
        assert_eq!(first.get("path").and_then(|v| v.as_str()), Some("skill/"));
    }

    #[test]
    fn read_skill_subscriptions_accepts_array_and_object_shapes() {
        let tmp = tempdir().expect("tempdir");
        let array_path = tmp.path().join("subscriptions-array.json");
        std::fs::write(
            &array_path,
            r#"[
  { "source": "https://github.com/example/skills::skills", "added_at": "now" },
  { "url": "https://github.com/example/more-skills::skills" },
  "https://github.com/example/string-entry::skills"
]"#,
        )
        .expect("write array shape");
        let arr = read_skill_subscriptions(&array_path);
        assert!(arr.contains(&"https://github.com/example/skills::skills".to_string()));
        assert!(arr.contains(&"https://github.com/example/more-skills::skills".to_string()));
        assert!(arr.contains(&"https://github.com/example/string-entry::skills".to_string()));

        let object_path = tmp.path().join("subscriptions-object.json");
        std::fs::write(
            &object_path,
            r#"{
  "subscriptions": [
    { "tap": "https://github.com/example/object-shape::skills" }
  ]
}"#,
        )
        .expect("write object shape");
        let obj = read_skill_subscriptions(&object_path);
        assert_eq!(
            obj,
            vec!["https://github.com/example/object-shape::skills".to_string()]
        );
    }

    #[test]
    fn effective_skill_taps_merges_defaults_custom_and_subscriptions() {
        let tmp = tempdir().expect("tempdir");
        let taps_file = tmp.path().join("skill_taps.json");
        let subscriptions_file = tmp.path().join("subscriptions.json");

        write_skill_taps(
            &taps_file,
            &["https://github.com/example/custom-skills::skills".to_string()],
        )
        .expect("write taps");
        std::fs::write(
            &subscriptions_file,
            r#"[
  { "source": "https://github.com/example/subscribed-skills::skills" },
  { "source": "not-a-tap-registry://ignored" }
]"#,
        )
        .expect("write subscriptions");

        let merged = effective_skill_taps(&taps_file, &subscriptions_file);
        assert!(merged.contains(&"https://github.com/openai/skills::skills".to_string()));
        assert!(merged.contains(&"https://github.com/example/custom-skills::skills".to_string()));
        assert!(
            merged.contains(&"https://github.com/example/subscribed-skills::skills".to_string())
        );
        assert!(!merged.contains(&"not-a-tap-registry://ignored".to_string()));
    }

    #[test]
    fn subscription_source_to_tap_filters_registry_prefixes_and_non_github_schemes() {
        assert_eq!(
            subscription_source_to_tap("https://github.com/example/skills::skills"),
            Some("https://github.com/example/skills::skills".to_string())
        );
        assert_eq!(subscription_source_to_tap("official/coder"), None);
        assert_eq!(subscription_source_to_tap("skills.sh/foo/bar"), None);
        assert_eq!(
            subscription_source_to_tap("not-a-tap-registry://ignored"),
            None
        );
    }

    #[test]
    fn sort_registry_skill_records_uses_router_priority_tie_break() {
        let mut records = vec![
            RegistrySkillRecord {
                identifier: "lobehub/a".to_string(),
                description: "".to_string(),
                source: "lobehub".to_string(),
                score: 700,
                install_source: RegistryInstallSource::LobeHub {
                    slug: "a".to_string(),
                },
            },
            RegistrySkillRecord {
                identifier: "skills.sh/b".to_string(),
                description: "".to_string(),
                source: "skills.sh".to_string(),
                score: 700,
                install_source: RegistryInstallSource::GitHub(ResolvedSkillSource {
                    repo: "openai/skills".to_string(),
                    branch: "main".to_string(),
                    skill_dir: "skills/b".to_string(),
                }),
            },
            RegistrySkillRecord {
                identifier: "github/c".to_string(),
                description: "".to_string(),
                source: "github".to_string(),
                score: 700,
                install_source: RegistryInstallSource::GitHub(ResolvedSkillSource {
                    repo: "openai/skills".to_string(),
                    branch: "main".to_string(),
                    skill_dir: "skills/c".to_string(),
                }),
            },
        ];

        sort_registry_skill_records(&mut records);
        let ordered_sources: Vec<String> = records.into_iter().map(|r| r.source).collect();
        assert_eq!(
            ordered_sources,
            vec![
                "skills.sh".to_string(),
                "github".to_string(),
                "lobehub".to_string()
            ]
        );
    }

    #[test]
    fn parse_explicit_github_skill_owner_repo_path() {
        let parsed = parse_explicit_github_skill("openai/skills/skills/.system/skill-creator")
            .expect("explicit parse");
        assert_eq!(parsed.0, "openai/skills");
        assert_eq!(parsed.1, None);
        assert_eq!(parsed.2, "skills/.system/skill-creator");
    }

    #[test]
    fn registry_prefixed_install_identifiers_override_github_slug_parse() {
        let registry_prefixed = parse_registry_prefixed_skill("official/creative/comfyui");
        assert_eq!(
            registry_prefixed,
            Some(("official".to_string(), "creative/comfyui".to_string()))
        );
        let explicit = if registry_prefixed.is_some() {
            None
        } else {
            parse_explicit_github_skill("official/creative/comfyui")
        };
        assert!(explicit.is_none());
    }

    #[test]
    fn registry_prefixed_install_identifiers_override_github_slug_parse_pretext() {
        let registry_prefixed = parse_registry_prefixed_skill("official/creative/pretext");
        assert_eq!(
            registry_prefixed,
            Some(("official".to_string(), "creative/pretext".to_string()))
        );
        assert!(parse_explicit_github_skill("official/creative/pretext").is_none());
    }

    #[test]
    fn parse_skill_name_and_version_handles_repo_plus_skill() {
        let (name, suffix) = parse_skill_name_and_version("openai/skills@skill-creator");
        assert_eq!(name, "openai/skills");
        assert_eq!(suffix.as_deref(), Some("skill-creator"));
        assert!(looks_like_github_repo_slug(&name));
    }

    #[test]
    fn sanitize_skill_install_name_normalizes_path_tail() {
        assert_eq!(
            sanitize_skill_install_name("skills/.system/skill-creator"),
            "skill-creator"
        );
        assert_eq!(sanitize_skill_install_name("bad$name"), "bad_name");
    }

    #[test]
    fn ensure_safe_relative_path_rejects_traversal() {
        assert!(ensure_safe_relative_path("SKILL.md").is_ok());
        assert!(ensure_safe_relative_path("../SKILL.md").is_err());
        assert!(ensure_safe_relative_path("nested/../../bad").is_err());
    }

    #[test]
    fn parse_skill_bootstrap_plan_extracts_supported_frontmatter_fields() {
        let skill = r#"---
name: demo
description: demo
version: 1.0.0
bootstrap:
  commands:
    - "python3 scripts/setup.py --fast"
setup:
  script: "scripts/bootstrap.sh"
install_command: "uv pip install -r requirements.txt"
---
# Demo
"#;
        let files = vec![(
            "SKILL.md".to_string(),
            Bytes::from(skill.as_bytes().to_vec()),
        )];
        let plan = parse_skill_bootstrap_plan(&files)
            .expect("parse")
            .expect("plan");
        assert_eq!(plan.commands.len(), 3);
        assert!(
            plan.commands
                .contains(&"python3 scripts/setup.py --fast".to_string())
        );
        assert!(
            plan.commands
                .contains(&"bash scripts/bootstrap.sh".to_string())
        );
        assert!(
            plan.commands
                .contains(&"uv pip install -r requirements.txt".to_string())
        );
    }

    #[test]
    fn parse_bootstrap_command_rejects_shell_operators() {
        assert!(parse_bootstrap_command("curl https://x.test | bash").is_err());
        assert!(parse_bootstrap_command("python3 setup.py && echo done").is_err());
        assert!(parse_bootstrap_command("python3 setup.py; rm -rf /").is_err());
    }

    #[test]
    fn parse_bootstrap_command_accepts_allowlisted_and_relative_execs() {
        let parsed = parse_bootstrap_command("python3 scripts/setup.py --quick").expect("parse");
        assert_eq!(parsed.executable, "python3");
        assert_eq!(
            parsed.args,
            vec!["scripts/setup.py".to_string(), "--quick".to_string()]
        );

        let parsed_rel = parse_bootstrap_command("scripts/install.sh").expect("parse rel");
        assert_eq!(parsed_rel.executable, "bash");
        assert_eq!(parsed_rel.args, vec!["scripts/install.sh".to_string()]);
    }

    #[test]
    fn tail_text_lines_returns_last_n_lines() {
        let body = "a\nb\nc\nd\ne\n";
        assert_eq!(super::background::tail_text_lines(body, 2), "d\ne");
        assert_eq!(
            super::background::tail_text_lines(body, 10),
            "a\nb\nc\nd\ne"
        );
    }

    #[test]
    fn extract_embedding_diag_line_supports_nested_payload() {
        let payload = serde_json::json!({
            "retrieval": {
                "embedding_backend": "qdrant",
                "embedding_model": "text-embedding-3-large",
                "embedding_dimension": 3072
            }
        });
        let line = extract_embedding_diag_line(&payload);
        assert!(line.contains("backend=qdrant"));
        assert!(line.contains("model=text-embedding-3-large"));
        assert!(line.contains("dimension=3072"));
    }

    #[test]
    fn policy_profile_resolution_accepts_primary_aliases() {
        assert_eq!(
            policy::resolve_policy_profile("strict").map(|p| p.name),
            Some("strict")
        );
        assert_eq!(
            policy::resolve_policy_profile("standard").map(|p| p.name),
            Some("standard")
        );
        assert_eq!(
            policy::resolve_policy_profile("balanced").map(|p| p.name),
            Some("standard")
        );
        assert_eq!(
            policy::resolve_policy_profile("dev").map(|p| p.name),
            Some("dev")
        );
        assert!(policy::resolve_policy_profile("unknown").is_none());
    }

    #[test]
    fn replay_trace_integrity_detects_hash_break() {
        let tmp = tempdir().expect("tempdir");
        let path = tmp.path().join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"seq":1,"event":"user","prev_hash":"seed","event_hash":"h1","payload":{"turn":1}}
{"seq":2,"event":"assistant","prev_hash":"BROKEN","event_hash":"h2","payload":{"turn":1}}
"#,
        )
        .expect("write replay");
        let (entries, parse_errors, chain_breaks) =
            replay_trace_integrity(&path).expect("integrity");
        assert_eq!(entries, 2);
        assert_eq!(parse_errors, 0);
        assert_eq!(chain_breaks, 1);
    }

    #[test]
    fn parse_toggle_arg_supports_status_and_explicit_values() {
        assert_eq!(parse_toggle_arg(None, true).expect("toggle"), false);
        assert_eq!(
            parse_toggle_arg(Some("toggle"), false).expect("toggle"),
            true
        );
        assert_eq!(parse_toggle_arg(Some("on"), false).expect("on"), true);
        assert_eq!(parse_toggle_arg(Some("off"), true).expect("off"), false);
        assert!(parse_toggle_arg(Some("bad-value"), true).is_err());
    }

    #[test]
    fn parse_reasoning_effort_accepts_levels_and_auto_clear() {
        assert_eq!(
            parse_reasoning_effort("minimal").expect("minimal"),
            Some("minimal")
        );
        assert_eq!(parse_reasoning_effort("low").expect("low"), Some("low"));
        assert_eq!(
            parse_reasoning_effort("medium").expect("medium"),
            Some("medium")
        );
        assert_eq!(parse_reasoning_effort("high").expect("high"), Some("high"));
        assert_eq!(
            parse_reasoning_effort("xhigh").expect("xhigh"),
            Some("xhigh")
        );
        assert_eq!(parse_reasoning_effort("auto").expect("auto"), None);
        assert!(parse_reasoning_effort("turbo").is_err());
    }

    #[test]
    fn parse_pet_species_and_mood_validate_catalog_entries() {
        assert_eq!(parse_pet_species("fox").as_deref(), Some("fox"));
        assert!(parse_pet_species("dragon").is_none());
        assert_eq!(parse_pet_mood("ready").as_deref(), Some("ready"));
        assert!(parse_pet_mood("sleeping-beauty").is_none());
    }

    #[test]
    fn parse_pet_dock_accepts_left_or_right() {
        assert_eq!(parse_pet_dock("left"), Some(PetDock::Left));
        assert_eq!(parse_pet_dock("right"), Some(PetDock::Right));
        assert_eq!(parse_pet_dock("center"), None);
    }

    #[test]
    fn resolve_cli_chat_provider_model_defaults_to_config_when_no_overrides() {
        let _lock = env_test_lock();
        let prev_inference_model = std::env::var("HERMES_INFERENCE_MODEL").ok();
        crate::env_vars::remove_var("HERMES_INFERENCE_MODEL");
        let resolved =
            resolve_cli_chat_provider_model(Some("nous:moonshotai/kimi-k2.6"), None, None)
                .expect("resolve");
        assert_eq!(resolved, "nous:moonshotai/kimi-k2.6");
        match prev_inference_model {
            Some(value) => crate::env_vars::set_var("HERMES_INFERENCE_MODEL", value),
            None => crate::env_vars::remove_var("HERMES_INFERENCE_MODEL"),
        }
    }

    #[test]
    fn resolve_cli_chat_provider_model_applies_provider_override() {
        let resolved = resolve_cli_chat_provider_model(Some("gpt-4o"), None, Some("anthropic"))
            .expect("resolve");
        assert_eq!(resolved, "anthropic:gpt-4o");
    }

    #[test]
    fn resolve_cli_chat_provider_model_prefers_model_override_with_provider_prefix() {
        let resolved = resolve_cli_chat_provider_model(
            Some("openai:gpt-4o"),
            Some("moonshotai/kimi-k2.6"),
            Some("nous"),
        )
        .expect("resolve");
        assert_eq!(resolved, "nous:moonshotai/kimi-k2.6");
    }

    #[test]
    fn resolve_cli_chat_provider_model_uses_inference_model_env_when_no_flag_override() {
        let _lock = env_test_lock();
        crate::env_vars::set_var("HERMES_INFERENCE_MODEL", "nous:moonshotai/kimi-k2.6");
        let resolved =
            resolve_cli_chat_provider_model(Some("openai:gpt-4o"), None, None).expect("resolve");
        assert_eq!(resolved, "nous:moonshotai/kimi-k2.6");
        crate::env_vars::remove_var("HERMES_INFERENCE_MODEL");
    }

    #[test]
    fn apply_cli_chat_runtime_env_sets_provider_model() {
        let _lock = env_test_lock();
        let keys = [
            "HERMES_MODEL",
            "HERMES_INFERENCE_MODEL",
            "HERMES_INFERENCE_PROVIDER",
            "HERMES_TUI_PROVIDER",
        ];
        for key in keys {
            crate::env_vars::remove_var(key);
        }
        crate::env_vars::set_var("HERMES_TUI_PROVIDER", "openai");

        apply_cli_chat_runtime_env("nous:openai/gpt-5.5");

        assert_eq!(
            std::env::var("HERMES_MODEL").ok().as_deref(),
            Some("nous:openai/gpt-5.5")
        );
        assert_eq!(
            std::env::var("HERMES_INFERENCE_MODEL").ok().as_deref(),
            Some("nous:openai/gpt-5.5")
        );
        assert_eq!(
            std::env::var("HERMES_INFERENCE_PROVIDER").ok().as_deref(),
            Some("nous")
        );
        assert_eq!(
            std::env::var("HERMES_TUI_PROVIDER").ok().as_deref(),
            Some("nous")
        );

        for key in keys {
            crate::env_vars::remove_var(key);
        }
    }

    #[test]
    fn query_mode_tools_enabled_defaults_on_for_query_mode() {
        let _lock = env_test_lock();
        crate::env_vars::remove_var("HERMES_QUERY_DISABLE_TOOLS");
        crate::env_vars::remove_var("HERMES_QUERY_ALLOW_TOOLS");
        assert!(query_mode_tools_enabled(true, false));
        assert!(query_mode_tools_enabled(false, false));
    }

    #[test]
    fn query_mode_tools_enabled_respects_disable_env_and_flag_override() {
        let _lock = env_test_lock();
        crate::env_vars::remove_var("HERMES_QUERY_ALLOW_TOOLS");
        crate::env_vars::set_var("HERMES_QUERY_DISABLE_TOOLS", "1");
        assert!(!query_mode_tools_enabled(true, false));
        assert!(query_mode_tools_enabled(true, true));
        crate::env_vars::remove_var("HERMES_QUERY_DISABLE_TOOLS");
    }

    #[test]
    fn query_mode_tools_enabled_respects_legacy_allow_env() {
        let _lock = env_test_lock();
        crate::env_vars::remove_var("HERMES_QUERY_DISABLE_TOOLS");
        crate::env_vars::remove_var("HERMES_QUERY_ALLOW_TOOLS");
        assert!(query_mode_tools_enabled(true, false));
        crate::env_vars::set_var("HERMES_QUERY_ALLOW_TOOLS", "1");
        assert!(query_mode_tools_enabled(true, false));
        crate::env_vars::remove_var("HERMES_QUERY_ALLOW_TOOLS");
    }

    #[test]
    fn format_personality_catalog_includes_current_and_usage_hint() {
        let catalog = format_personality_catalog(
            Some("technical"),
            &[("coder", "Use when building or debugging code.")],
        );
        assert!(catalog.contains("## Built-in personalities"));
        assert!(catalog.contains("Current: `technical`"));
        assert!(catalog.contains("Use `/personality <name>` to switch."));
    }

    #[test]
    fn format_personality_catalog_renders_multiline_entries() {
        let catalog = format_personality_catalog(
            None,
            &[
                ("coder", "Use when building or debugging code."),
                ("writer", "Use when drafting polished prose."),
            ],
        );
        assert!(catalog.contains("- `coder`\n  Use when building or debugging code."));
        assert!(catalog.contains("- `writer`\n  Use when drafting polished prose."));
    }

    #[test]
    fn secret_stdout_gate_defaults_false() {
        let _lock = env_test_lock();
        crate::env_vars::remove_var("HERMES_ALLOW_SECRET_STDOUT");
        assert!(!secret_stdout_allowed());
    }

    #[test]
    fn secret_stdout_gate_accepts_truthy_values() {
        let _lock = env_test_lock();
        crate::env_vars::set_var("HERMES_ALLOW_SECRET_STDOUT", "yes");
        assert!(secret_stdout_allowed());
        crate::env_vars::remove_var("HERMES_ALLOW_SECRET_STDOUT");
    }

    #[test]
    fn mask_secret_value_hides_payload() {
        let raw = "very-secret-value";
        let masked = mask_secret_value(raw);
        assert!(!masked.contains(raw));
        assert!(masked.contains("***"));
    }

    #[test]
    fn specpatch_block_reason_flags_destructive_patterns() {
        assert!(specpatch_block_reason("echo safe").is_none());
        assert!(specpatch_block_reason("rm -rf /").is_some());
        assert!(specpatch_block_reason("rm -rf /tmp").is_some());
        assert!(specpatch_block_reason("git reset --hard HEAD").is_some());
    }

    #[test]
    fn extract_marker_paths_captures_path_and_file_tokens() {
        let text = "PATCH_VERIFIED: path=/tmp/a.rs file=src/main.rs cmd=rg -n foo";
        let paths = extract_marker_paths(text);
        assert!(paths.contains(&"/tmp/a.rs".to_string()));
        assert!(paths.contains(&"src/main.rs".to_string()));
    }

    #[test]
    fn normalize_repo_relative_path_handles_absolute_and_relative() {
        let root = std::env::temp_dir().join("hermes-repo-path-test");
        let rel = normalize_repo_relative_path(&root, "src/main.rs").expect("relative");
        assert_eq!(rel, "src/main.rs");
        let abs_path = root.join("src").join("lib.rs");
        let abs = normalize_repo_relative_path(
            &root,
            abs_path.to_str().expect("absolute path should be utf-8"),
        )
        .expect("abs");
        let normalized = abs.replace('\\', "/");
        assert_eq!(normalized, "src/lib.rs");
    }
}
