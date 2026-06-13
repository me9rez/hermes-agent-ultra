//! `/plan` slash command — planner queue, capability router, and task-depth controls.

use std::fmt::Write as _;

use hermes_core::AgentError;

use crate::commands::background;
use crate::commands::model::{
    ModelCapabilityRequirements, default_client, resolve_model_capabilities, split_provider_model,
    unmet_model_requirements,
};
use crate::commands::objective;
use crate::commands::ops;
use crate::commands::{CommandResult, emit_command_output, truncate_chars};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanCapabilityMode {
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

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Advisory => "advisory",
            Self::Enforce => "enforce",
        }
    }
}

pub(crate) fn plan_capability_mode() -> PlanCapabilityMode {
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

fn plan_capability_preflight(
    host: &impl crate::app::ModelRuntime,
    task: &str,
) -> (Option<String>, bool) {
    let mode = plan_capability_mode();
    if matches!(mode, PlanCapabilityMode::Off) {
        return (None, true);
    }

    let req = infer_plan_requirements(task);
    if req.is_empty() {
        return (None, true);
    }

    let (provider, model_id) = split_provider_model(host.current_model());
    let client = default_client();
    let caps = resolve_model_capabilities(provider, model_id, client);
    let unmet = unmet_model_requirements(caps, req);
    if unmet.is_empty() {
        return (
            Some(format!(
                "planner capability preflight: PASS ({}) for `{}`",
                req.summary(),
                host.current_model()
            )),
            true,
        );
    }

    let explain_hint = format!(
        "/model explain {} --cap tools,reasoning --min-context 128000",
        host.current_model()
    );
    let message = format!(
        "planner capability preflight: {} ({}) for `{}`.\nmissing: {}\nhint: run `{}` or switch with `/model` before queuing this task.",
        if matches!(mode, PlanCapabilityMode::Enforce) {
            "BLOCKED"
        } else {
            "WARN"
        },
        req.summary(),
        host.current_model(),
        unmet.join(", "),
        explain_hint
    );

    let allowed = !matches!(mode, PlanCapabilityMode::Enforce);
    (Some(message), allowed)
}

pub(crate) fn handle_plan_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    if args.is_empty()
        || args
            .first()
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "help" | "usage"))
    {
        emit_command_output(
            host,
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
                    host,
                    format!(
                        "planner capability router mode={}\nUse `/plan caps [off|advisory|enforce]`.",
                        plan_capability_mode().as_str()
                    ),
                );
            }
            "off" | "advisory" | "enforce" => {
                if let Some(mode) = PlanCapabilityMode::parse(&next) {
                    let mode_label = mode.as_str();
                    crate::env_vars::set_var("HERMES_PLAN_CAPABILITY_ROUTER", mode_label);
                    emit_command_output(
                        host,
                        format!("planner capability router set to `{}`.", mode_label),
                    );
                }
            }
            _ => emit_command_output(host, "Usage: /plan caps [status|off|advisory|enforce]"),
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
            "status" | "show" => emit_command_output(host, ops::task_depth_runtime_summary()),
            "list" => emit_command_output(
                host,
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
                ops::apply_task_depth_profile(ops::TaskDepthProfile::Balanced);
                emit_command_output(
                    host,
                    format!(
                        "Task depth reset to defaults.\n{}",
                        ops::task_depth_runtime_summary()
                    ),
                );
            }
            _ => {
                let Some(profile) = ops::TaskDepthProfile::parse(&next) else {
                    emit_command_output(
                        host,
                        "Usage: /plan depth [status|list|shallow|balanced|deep|max|clear]",
                    );
                    return Ok(CommandResult::Handled);
                };
                ops::apply_task_depth_profile(profile);
                emit_command_output(
                    host,
                    format!(
                        "Task depth profile set to `{}`.\n{}",
                        profile.as_str(),
                        ops::task_depth_runtime_summary()
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
        if let Some(steer) = objective::current_session_steer(host) {
            let _ = writeln!(out, "  steering={}", truncate_chars(&steer, 160));
        } else {
            let _ = writeln!(out, "  steering=(none)");
        }
        if let Some(objective) = host.session_objective() {
            let _ = writeln!(out, "  objective={}", truncate_chars(objective, 160));
        } else {
            let _ = writeln!(out, "  objective=(none)");
        }
        let _ = writeln!(
            out,
            "  capability_router={}",
            plan_capability_mode().as_str()
        );
        let _ = writeln!(out, "  {}", ops::task_depth_runtime_summary());
        emit_command_output(host, out.trim_end());
        return Ok(CommandResult::Handled);
    }
    if sub == "clear" {
        return background::handle_clear_queue_command(host);
    }
    let task = args.join(" ");
    if !task.trim().is_empty() {
        let (note, allowed) = plan_capability_preflight(host, &task);
        if let Some(msg) = note {
            emit_command_output(host, msg);
        }
        if !allowed {
            return Ok(CommandResult::Handled);
        }
    }
    background::handle_background_command(host, args)
}
