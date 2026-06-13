//! Compression-related slash command handlers.
//!
//! Extracted from `commands.rs` into a sub-module.
//! Handles `/compress`, `/compress-rules`, and `/autocompact` commands.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use hermes_core::AgentError;
use serde::{Deserialize, Serialize};

use crate::commands::{CommandResult, emit_command_output};

// ---------------------------------------------------------------------------
// CompressionRulePlaneConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CompressionRulePlaneConfig {
    #[serde(default)]
    max_assistant_render_lines: Option<usize>,
    #[serde(default)]
    max_tool_output_lines: Option<usize>,
    #[serde(default)]
    max_tool_output_line_chars: Option<usize>,
    #[serde(default)]
    max_tool_output_total_chars: Option<usize>,
}

// ---------------------------------------------------------------------------
// CompressionRenderPolicy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CompressionRenderPolicy {
    max_assistant_render_lines: usize,
    max_tool_output_lines: usize,
    max_tool_output_line_chars: usize,
    max_tool_output_total_chars: usize,
}

impl CompressionRenderPolicy {
    fn builtin_defaults() -> Self {
        Self {
            max_assistant_render_lines: 260,
            max_tool_output_lines: 180,
            max_tool_output_line_chars: 600,
            max_tool_output_total_chars: 48_000,
        }
    }

    fn apply_plane(&mut self, plane: &CompressionRulePlaneConfig) {
        if let Some(v) = plane.max_assistant_render_lines {
            self.max_assistant_render_lines = v.clamp(40, 4000);
        }
        if let Some(v) = plane.max_tool_output_lines {
            self.max_tool_output_lines = v.clamp(20, 5000);
        }
        if let Some(v) = plane.max_tool_output_line_chars {
            self.max_tool_output_line_chars = v.clamp(120, 4000);
        }
        if let Some(v) = plane.max_tool_output_total_chars {
            self.max_tool_output_total_chars = v.clamp(2000, 500_000);
        }
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn compression_rules_dir() -> PathBuf {
    hermes_config::hermes_home().join("compression")
}

fn compression_user_rules_path() -> PathBuf {
    compression_rules_dir().join("user-rules.json")
}

fn compression_project_rules_path() -> Option<PathBuf> {
    super::detect_repo_root_from_cwd()
        .map(|root| root.join(".hermes-ultra").join("compression-rules.json"))
}

// ---------------------------------------------------------------------------
// Load / Save
// ---------------------------------------------------------------------------

fn load_compression_plane(path: &Path) -> Option<CompressionRulePlaneConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<CompressionRulePlaneConfig>(&raw).ok()
}

fn save_compression_plane(
    path: &Path,
    plane: &CompressionRulePlaneConfig,
) -> Result<(), AgentError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", parent.display(), e)))?;
    }
    let payload = serde_json::to_string_pretty(plane)
        .map_err(|e| AgentError::Io(format!("Failed to encode compression rules: {}", e)))?;
    std::fs::write(path, payload)
        .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", path.display(), e)))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Merge / Apply
// ---------------------------------------------------------------------------

fn merged_compression_policy() -> (
    CompressionRenderPolicy,
    Option<CompressionRulePlaneConfig>,
    Option<CompressionRulePlaneConfig>,
) {
    let mut merged = CompressionRenderPolicy::builtin_defaults();
    let user = load_compression_plane(&compression_user_rules_path());
    let project = compression_project_rules_path()
        .as_deref()
        .and_then(load_compression_plane);
    if let Some(ref user_plane) = user {
        merged.apply_plane(user_plane);
    }
    if let Some(ref project_plane) = project {
        merged.apply_plane(project_plane);
    }
    (merged, user, project)
}

fn apply_compression_policy_env(policy: &CompressionRenderPolicy) {
    crate::env_vars::set_var(
        "HERMES_TUI_MAX_ASSISTANT_RENDER_LINES",
        policy.max_assistant_render_lines.to_string(),
    );
    crate::env_vars::set_var(
        "HERMES_TUI_MAX_TOOL_OUTPUT_LINES",
        policy.max_tool_output_lines.to_string(),
    );
    crate::env_vars::set_var(
        "HERMES_TUI_MAX_TOOL_OUTPUT_LINE_CHARS",
        policy.max_tool_output_line_chars.to_string(),
    );
    crate::env_vars::set_var(
        "HERMES_TUI_MAX_TOOL_OUTPUT_TOTAL_CHARS",
        policy.max_tool_output_total_chars.to_string(),
    );
}

// ---------------------------------------------------------------------------
// Status / Render
// ---------------------------------------------------------------------------

fn render_compression_policy_status() -> String {
    let (merged, user, project) = merged_compression_policy();
    let project_path = compression_project_rules_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(repo root unavailable)".to_string());
    let mut out = String::new();
    out.push_str("Compression policy planes\n");
    out.push_str("-------------------------\n");
    out.push_str("builtin: max_assistant_render_lines=260, max_tool_output_lines=180, max_tool_output_line_chars=600, max_tool_output_total_chars=48000\n");
    let _ = writeln!(
        out,
        "user: {} ({})",
        if user.is_some() {
            "configured"
        } else {
            "not configured"
        },
        compression_user_rules_path().display()
    );
    let _ = writeln!(
        out,
        "project: {} ({})",
        if project.is_some() {
            "configured"
        } else {
            "not configured"
        },
        project_path
    );
    let _ = writeln!(
        out,
        "\nmerged:\n  - max_assistant_render_lines={}\n  - max_tool_output_lines={}\n  - max_tool_output_line_chars={}\n  - max_tool_output_total_chars={}",
        merged.max_assistant_render_lines,
        merged.max_tool_output_lines,
        merged.max_tool_output_line_chars,
        merged.max_tool_output_total_chars
    );
    out.push_str(
        "\nUse `/compress rules recommend` to generate heuristics from current transcript shape.\n\
         Use `/compress rules autotune` for dry-run tuning, or `/compress rules autotune apply [user|project]` to persist + apply.\n\
         Use `/compress rules apply` to push merged settings into live runtime env.\n\
         Use `/compress rules set user <key> <value>` or `/compress rules set project <key> <value>`.\n\
         Keys: assistant_lines | tool_lines | tool_line_chars | tool_total_chars",
    );
    out
}

fn parse_compression_rule_key(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "assistant_lines" | "max_assistant_render_lines" | "assistant" => Some("assistant_lines"),
        "tool_lines" | "max_tool_output_lines" | "tool" => Some("tool_lines"),
        "tool_line_chars" | "max_tool_output_line_chars" | "tool_chars" => Some("tool_line_chars"),
        "tool_total_chars" | "max_tool_output_total_chars" | "tool_total" => {
            Some("tool_total_chars")
        }
        _ => None,
    }
}

fn set_compression_rule_field(
    plane: &mut CompressionRulePlaneConfig,
    key: &str,
    value: usize,
) -> Result<(), AgentError> {
    let normalized = parse_compression_rule_key(key).ok_or_else(|| {
        AgentError::Config(format!(
            "Unknown compression rule key '{}'. Use assistant_lines|tool_lines|tool_line_chars|tool_total_chars.",
            key
        ))
    })?;
    match normalized {
        "assistant_lines" => plane.max_assistant_render_lines = Some(value.clamp(40, 4000)),
        "tool_lines" => plane.max_tool_output_lines = Some(value.clamp(20, 5000)),
        "tool_line_chars" => plane.max_tool_output_line_chars = Some(value.clamp(120, 4000)),
        "tool_total_chars" => plane.max_tool_output_total_chars = Some(value.clamp(2000, 500_000)),
        _ => {}
    }
    Ok(())
}

fn resolve_compression_plane_path(target: &str) -> Result<PathBuf, AgentError> {
    let normalized = target.trim().to_ascii_lowercase();
    if normalized == "user" {
        return Ok(compression_user_rules_path());
    }
    if normalized == "project" {
        return compression_project_rules_path().ok_or_else(|| {
            AgentError::Config(
                "Project plane unavailable: run inside a repository checkout.".to_string(),
            )
        });
    }
    Err(AgentError::Config(
        "Plane must be `user` or `project`.".to_string(),
    ))
}

// ---------------------------------------------------------------------------
// Recommendation
// ---------------------------------------------------------------------------

fn recommend_compression_policy_for_app(
    host: &impl crate::app::SessionRuntime,
    base: &CompressionRenderPolicy,
) -> CompressionRenderPolicy {
    let mut next = base.clone();
    let mut assistant_msgs = 0usize;
    let mut assistant_lines = 0usize;
    let mut assistant_peak_line_chars = 0usize;
    let mut tool_msgs = 0usize;
    let mut tool_lines = 0usize;
    let mut tool_peak_line_chars = 0usize;
    let mut tool_total_chars = 0usize;

    for msg in host.messages() {
        let Some(content) = msg.content.as_ref() else {
            continue;
        };
        let lines = content.lines().count().max(1);
        let peak_line_chars = content
            .lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or_else(|| content.chars().count());
        match msg.role {
            hermes_core::MessageRole::Assistant => {
                assistant_msgs = assistant_msgs.saturating_add(1);
                assistant_lines = assistant_lines.saturating_add(lines);
                assistant_peak_line_chars = assistant_peak_line_chars.max(peak_line_chars);
            }
            hermes_core::MessageRole::Tool => {
                tool_msgs = tool_msgs.saturating_add(1);
                tool_lines = tool_lines.saturating_add(lines);
                tool_peak_line_chars = tool_peak_line_chars.max(peak_line_chars);
                tool_total_chars = tool_total_chars.saturating_add(content.chars().count());
            }
            _ => {}
        }
    }

    if assistant_msgs >= 6 {
        let avg = assistant_lines / assistant_msgs.max(1);
        if avg > 60 {
            next.max_assistant_render_lines = next.max_assistant_render_lines.max(320).min(4000);
        } else if avg < 24 {
            next.max_assistant_render_lines = next.max_assistant_render_lines.min(220).max(40);
        }
        if assistant_peak_line_chars > 160 {
            next.max_tool_output_line_chars = next.max_tool_output_line_chars.max(720).min(4000);
        }
    }

    if tool_msgs >= 2 {
        let avg_tool_lines = tool_lines / tool_msgs.max(1);
        if avg_tool_lines > 120 {
            next.max_tool_output_lines = next.max_tool_output_lines.max(260).min(5000);
        } else if avg_tool_lines < 40 {
            next.max_tool_output_lines = next.max_tool_output_lines.min(160).max(20);
        }
        if tool_peak_line_chars > 720 {
            next.max_tool_output_line_chars = next.max_tool_output_line_chars.max(920).min(4000);
        }
        if tool_total_chars > 120_000 {
            next.max_tool_output_total_chars =
                next.max_tool_output_total_chars.max(96_000).min(500_000);
        } else if tool_total_chars < 24_000 {
            next.max_tool_output_total_chars =
                next.max_tool_output_total_chars.min(40_000).max(2000);
        }
    }

    if host.messages().len() >= 140 {
        next.max_assistant_render_lines = next.max_assistant_render_lines.min(240).max(40);
        next.max_tool_output_total_chars = next.max_tool_output_total_chars.min(64_000).max(2000);
    }
    next
}

fn render_compression_recommendation(
    current: &CompressionRenderPolicy,
    recommended: &CompressionRenderPolicy,
) -> String {
    let mut out = String::new();
    out.push_str("Compression policy recommendation\n");
    out.push_str("---------------------------------\n");
    let _ = writeln!(
        out,
        "assistant_lines: {} -> {}",
        current.max_assistant_render_lines, recommended.max_assistant_render_lines
    );
    let _ = writeln!(
        out,
        "tool_lines: {} -> {}",
        current.max_tool_output_lines, recommended.max_tool_output_lines
    );
    let _ = writeln!(
        out,
        "tool_line_chars: {} -> {}",
        current.max_tool_output_line_chars, recommended.max_tool_output_line_chars
    );
    let _ = writeln!(
        out,
        "tool_total_chars: {} -> {}",
        current.max_tool_output_total_chars, recommended.max_tool_output_total_chars
    );
    out.push_str(
        "\nApply with `/compress rules autotune apply` (user plane) or `/compress rules autotune apply project`.",
    );
    out
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

fn handle_compress_rules_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match action.as_str() {
        "status" | "show" | "preview" => {
            emit_command_output(host, render_compression_policy_status());
        }
        "recommend" => {
            let (merged, _, _) = merged_compression_policy();
            let rec = recommend_compression_policy_for_app(host, &merged);
            emit_command_output(host, render_compression_recommendation(&merged, &rec));
        }
        "autotune" => {
            let (merged, _, _) = merged_compression_policy();
            let rec = recommend_compression_policy_for_app(host, &merged);
            if args
                .get(1)
                .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "apply" | "--apply"))
            {
                let target = args.get(2).copied().unwrap_or("user").to_ascii_lowercase();
                let path = match resolve_compression_plane_path(&target) {
                    Ok(path) => path,
                    Err(err) => {
                        emit_command_output(host, err.to_string());
                        return Ok(CommandResult::Handled);
                    }
                };
                let plane = CompressionRulePlaneConfig {
                    max_assistant_render_lines: Some(rec.max_assistant_render_lines),
                    max_tool_output_lines: Some(rec.max_tool_output_lines),
                    max_tool_output_line_chars: Some(rec.max_tool_output_line_chars),
                    max_tool_output_total_chars: Some(rec.max_tool_output_total_chars),
                };
                save_compression_plane(&path, &plane)?;
                apply_compression_policy_env(&rec);
                emit_command_output(
                    host,
                    format!(
                        "{}\n\nAutotune applied to {} plane ({}) and runtime env updated.",
                        render_compression_recommendation(&merged, &rec),
                        target,
                        path.display()
                    ),
                );
            } else {
                emit_command_output(
                    host,
                    format!(
                        "{}\n\nDry-run only. Add `apply` to persist: `/compress rules autotune apply [user|project]`.",
                        render_compression_recommendation(&merged, &rec)
                    ),
                );
            }
        }
        "apply" => {
            let (merged, _, _) = merged_compression_policy();
            apply_compression_policy_env(&merged);
            emit_command_output(
                host,
                format!(
                    "Applied compression policy to runtime env.\n\
                     HERMES_TUI_MAX_ASSISTANT_RENDER_LINES={}\n\
                     HERMES_TUI_MAX_TOOL_OUTPUT_LINES={}\n\
                     HERMES_TUI_MAX_TOOL_OUTPUT_LINE_CHARS={}\n\
                     HERMES_TUI_MAX_TOOL_OUTPUT_TOTAL_CHARS={}",
                    merged.max_assistant_render_lines,
                    merged.max_tool_output_lines,
                    merged.max_tool_output_line_chars,
                    merged.max_tool_output_total_chars
                ),
            );
        }
        "set" => {
            let Some(plane_name) = args.get(1).copied() else {
                emit_command_output(
                    host,
                    "Usage: /compress rules set <user|project> <key> <value>",
                );
                return Ok(CommandResult::Handled);
            };
            let Some(key) = args.get(2).copied() else {
                emit_command_output(
                    host,
                    "Usage: /compress rules set <user|project> <key> <value>",
                );
                return Ok(CommandResult::Handled);
            };
            let Some(value_raw) = args.get(3).copied() else {
                emit_command_output(
                    host,
                    "Usage: /compress rules set <user|project> <key> <value>",
                );
                return Ok(CommandResult::Handled);
            };
            let value = value_raw.parse::<usize>().map_err(|_| {
                AgentError::Config(format!(
                    "Invalid value '{}' (expected positive integer).",
                    value_raw
                ))
            })?;
            let target = plane_name.trim().to_ascii_lowercase();
            let path = match resolve_compression_plane_path(&target) {
                Ok(path) => path,
                Err(err) => {
                    emit_command_output(host, err.to_string());
                    return Ok(CommandResult::Handled);
                }
            };
            let mut plane = load_compression_plane(&path).unwrap_or_default();
            set_compression_rule_field(&mut plane, key, value)?;
            save_compression_plane(&path, &plane)?;
            emit_command_output(
                host,
                format!(
                    "Updated {} compression rule: {}={} ({})",
                    target,
                    key,
                    value,
                    path.display()
                ),
            );
        }
        "clear" => {
            let Some(plane_name) = args.get(1).copied() else {
                emit_command_output(host, "Usage: /compress rules clear <user|project>");
                return Ok(CommandResult::Handled);
            };
            let target = plane_name.trim().to_ascii_lowercase();
            let path = match resolve_compression_plane_path(&target) {
                Ok(path) => path,
                Err(err) => {
                    emit_command_output(host, err.to_string());
                    return Ok(CommandResult::Handled);
                }
            };
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| {
                    AgentError::Io(format!("Failed to remove {}: {}", path.display(), e))
                })?;
                emit_command_output(
                    host,
                    format!("Cleared {} plane rules at {}.", target, path.display()),
                );
            } else {
                emit_command_output(host, format!("{} plane rules already clear.", target));
            }
        }
        _ => emit_command_output(
            host,
            "Usage: /compress rules [status|preview|recommend|autotune [apply [user|project]]|apply|set <user|project> <key> <value>|clear <user|project>]",
        ),
    }
    Ok(CommandResult::Handled)
}

pub(crate) async fn handle_compress_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args
        .first()
        .map(|v| v.eq_ignore_ascii_case("rules"))
        .unwrap_or(false)
    {
        return handle_compress_rules_command(host, &args[1..]);
    }
    let (pre_len, post_len, did_compress) = host.compress_conversation_context().await?;
    if pre_len <= 2 {
        emit_command_output(
            host,
            format!("Context too small to compress ({} messages).", pre_len),
        );
        return Ok(CommandResult::Handled);
    }
    if did_compress {
        emit_command_output(
            host,
            format!(
                "Compressed context: {} messages -> {} (session_id={}).",
                pre_len,
                post_len,
                host.session_id()
            ),
        );
    } else {
        emit_command_output(
            host,
            format!(
                "Compression skipped or no-op ({} messages unchanged). \
                 Context may be below threshold or another path holds the compression lock.",
                pre_len
            ),
        );
    }
    Ok(CommandResult::Handled)
}

// ---------------------------------------------------------------------------
// Compaction Governance
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionGovernanceMode {
    Off,
    Advisory,
    Enforce,
}

impl CompactionGovernanceMode {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "disable" | "disabled" | "0" => Some(Self::Off),
            "on" | "advisory" | "warn" | "1" => Some(Self::Advisory),
            "enforce" | "strict" => Some(Self::Enforce),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Advisory => "advisory",
            Self::Enforce => "enforce",
        }
    }
}

pub(crate) fn compaction_governance_mode() -> CompactionGovernanceMode {
    std::env::var("HERMES_CONTEXTLATTICE_COMPACTION_GOVERNANCE")
        .ok()
        .as_deref()
        .and_then(CompactionGovernanceMode::parse)
        .unwrap_or(CompactionGovernanceMode::Advisory)
}

pub(crate) async fn handle_autocompact_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "status".to_string());
    match action.as_str() {
        "status" | "show" => {
            let mode = compaction_governance_mode();
            emit_command_output(
                host,
                format!(
                    "Auto-compaction: enabled.\n\
                 Trigger policy: when context exceeds 80% of budget.\n\
                 Runs: once before first LLM call and after each turn.\n\
                 ContextLattice governance: {}.\n\
                 Manual override: `/autocompact now` or `/compress`.",
                    mode.as_str()
                ),
            );
            Ok(CommandResult::Handled)
        }
        "now" | "run" => handle_compress_command(host, &[]).await,
        "governance" | "govern" => {
            let Some(next) = args.get(1).copied() else {
                emit_command_output(
                    host,
                    format!(
                        "Compaction governance: {}.\nUsage: `/autocompact governance [off|advisory|enforce]`",
                        compaction_governance_mode().as_str()
                    ),
                );
                return Ok(CommandResult::Handled);
            };
            let Some(mode) = CompactionGovernanceMode::parse(next) else {
                emit_command_output(
                    host,
                    format!(
                        "Unknown governance mode '{}'. Use `off`, `advisory`, or `enforce`.",
                        next
                    ),
                );
                return Ok(CommandResult::Handled);
            };
            crate::env_vars::set_var("HERMES_CONTEXTLATTICE_COMPACTION_GOVERNANCE", mode.as_str());
            emit_command_output(
                host,
                format!("Compaction governance mode set to `{}`.", mode.as_str()),
            );
            Ok(CommandResult::Handled)
        }
        "help" => {
            emit_command_output(
                host,
                "Usage: `/autocompact [status|now|governance]`\n\
                 - `status`: show current auto-compaction behavior\n\
                 - `now`: run immediate compaction pass\n\
                 - `governance [off|advisory|enforce]`: ContextLattice checkpoint posture for compaction events",
            );
            Ok(CommandResult::Handled)
        }
        other => {
            emit_command_output(
                host,
                format!(
                    "Unknown /autocompact action '{}'. Use `status`, `now`, `governance`, or `help`.",
                    other
                ),
            );
            Ok(CommandResult::Handled)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::commands::handle_slash_command;
    use crate::test_env_lock;
    use tempfile::tempdir;

    fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        test_env_lock::lock()
    }

    fn build_test_app_with_stream_in_compress(
        home: &Path,
    ) -> impl std::future::Future<Output = App> {
        use clap::Parser;
        use tokio::sync::mpsc;
        async move {
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
    }

    struct TempHomeGuardInCompress {
        previous_home: Option<String>,
        previous_clipboard_mock: Option<String>,
        previous_runtime_env: Vec<(&'static str, Option<String>)>,
    }

    impl TempHomeGuardInCompress {
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

    impl Drop for TempHomeGuardInCompress {
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

    fn latest_ui_assistant_text(host: &impl crate::app::SessionRuntime) -> String {
        host.ui_messages()
            .iter()
            .rev()
            .find(|row| row.message.role == hermes_core::MessageRole::Assistant)
            .and_then(|row| row.message.content.clone())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn p0_compress_rules_set_and_apply_updates_runtime_env() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuardInCompress::new(tmp.path());
        let mut host = build_test_app_with_stream_in_compress(tmp.path()).await;
        crate::env_vars::remove_var("HERMES_TUI_MAX_ASSISTANT_RENDER_LINES");

        handle_slash_command(
            &mut host,
            "/compress",
            &["rules", "set", "user", "assistant_lines", "320"],
        )
        .await
        .expect("compress rules set user");
        let set_output = latest_ui_assistant_text(&host);
        assert!(set_output.contains("Updated user compression rule"));
        assert!(set_output.contains("assistant_lines=320"));

        handle_slash_command(&mut host, "/compress", &["rules", "apply"])
            .await
            .expect("compress rules apply");
        let applied = latest_ui_assistant_text(&host);
        assert!(applied.contains("Applied compression policy to runtime env"));
        assert_eq!(
            std::env::var("HERMES_TUI_MAX_ASSISTANT_RENDER_LINES")
                .ok()
                .as_deref(),
            Some("320")
        );
    }

    #[tokio::test]
    async fn p2_compress_rules_autotune_apply_updates_runtime_env() {
        let _guard = env_test_lock();
        let tmp = tempdir().expect("tempdir");
        let _home_guard = TempHomeGuardInCompress::new(tmp.path());
        let mut host = build_test_app_with_stream_in_compress(tmp.path()).await;
        crate::env_vars::remove_var("HERMES_TUI_MAX_TOOL_OUTPUT_TOTAL_CHARS");

        handle_slash_command(
            &mut host,
            "/compress",
            &["rules", "autotune", "apply", "user"],
        )
        .await
        .expect("compress autotune apply");
        let out = latest_ui_assistant_text(&host);
        assert!(out.contains("Autotune applied"));
        assert!(
            std::env::var("HERMES_TUI_MAX_TOOL_OUTPUT_TOTAL_CHARS")
                .ok()
                .is_some(),
            "autotune should write runtime compression env"
        );
    }
}
