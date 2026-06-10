use std::fmt::Write as _;

use hermes_core::AgentError;

use crate::App;
use crate::commands::{CommandResult, emit_command_output};

pub(crate) async fn handle_auth_command(
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
            let provider = app.current_runtime_provider();
            let credential_present = crate::app::provider_api_key_from_env(&provider).is_some();
            let state = if credential_present {
                "present"
            } else {
                "missing"
            };
            let gate_line = super::oauth_runtime_gate_for_provider(&provider)
                .map(|(ok, detail)| {
                    format!(
                        "oauth_runtime_gate: {} ({})",
                        if ok { "PASS" } else { "FAIL" },
                        detail
                    )
                })
                .unwrap_or_else(|| "oauth_runtime_gate: n/a".to_string());
            emit_command_output(
                app,
                format!(
                    "Auth status\nprovider: {}\nmodel: {}\ncredential: {}\n{}\nnext: `/auth verify` (passive refresh check) or `/auth refresh` (forced token refresh)",
                    provider, app.current_model, state, gate_line
                ),
            );
        }
        "verify" => {
            let provider = app.current_runtime_provider();
            if let Some((ok, detail)) = super::oauth_runtime_gate_for_provider(&provider) {
                if !ok {
                    emit_command_output(
                        app,
                        format!(
                            "Auth verify blocked by OAuth runtime gate for `{}`.\n{}\nUpgrade runtime and retry.",
                            provider, detail
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
            }
            let summary = app.verify_runtime_auth(false).await?;
            emit_command_output(
                app,
                format!(
                    "{}\nnext: if provider rejects again, run `/auth refresh` then retry.",
                    summary
                ),
            );
        }
        "refresh" | "force" => {
            let provider = app.current_runtime_provider();
            if let Some((ok, detail)) = super::oauth_runtime_gate_for_provider(&provider) {
                if !ok {
                    emit_command_output(
                        app,
                        format!(
                            "Auth refresh blocked by OAuth runtime gate for `{}`.\n{}\nUpgrade runtime and retry.",
                            provider, detail
                        ),
                    );
                    return Ok(CommandResult::Handled);
                }
            }
            let summary = app.verify_runtime_auth(true).await?;
            emit_command_output(
                app,
                format!(
                    "{}\nforced refresh complete; retry your request now.",
                    summary
                ),
            );
        }
        _ => emit_command_output(
            app,
            "Usage: /auth [status|verify|refresh]\n- status: show active provider auth state\n- verify: passive credential hydration + verification\n- refresh: force OAuth/session token refresh",
        ),
    }
    Ok(CommandResult::Handled)
}

pub(crate) fn handle_telemetry_command(
    app: &mut App,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    let provider = app
        .current_model
        .split_once(':')
        .map(|(p, _)| p.to_string())
        .unwrap_or_else(|| "openai".to_string());
    let provider_health = super::provider_health_snapshot(&provider);
    let session = app.session_info();
    let mut out = String::new();
    let _ = writeln!(out, "Telemetry snapshot");
    let _ = writeln!(out, "session: {}", session.session_id);
    let _ = writeln!(out, "model: {}", app.current_model);
    let _ = writeln!(out, "messages: {}", session.message_count);
    let _ = writeln!(out, "provider health: {}", provider_health);

    if let Some(repo_root) = super::detect_repo_root_from_cwd() {
        let report_dir = repo_root.join(".sync-reports");
        let eval = super::latest_json_report(&report_dir, "eval-trend-gate-")
            .and_then(|p| super::summarize_gate_report(&p, "eval"))
            .unwrap_or_else(|| "eval=unknown".to_string());
        let autopilot = super::latest_json_report(&report_dir, "performance-autopilot-")
            .and_then(|p| super::summarize_performance_autopilot_report(&p, "autopilot"))
            .unwrap_or_else(|| "autopilot=unknown".to_string());
        let replay = super::latest_json_report(&report_dir, "deterministic-replay-")
            .and_then(|p| super::summarize_gate_report(&p, "replay"))
            .unwrap_or_else(|| "replay=unknown".to_string());
        let _ = writeln!(out, "gates: {}; {}; {}", eval, autopilot, replay);
    }

    if action == "lane" {
        let _ = writeln!(
            out,
            "lane hints:\n- Ctrl+L toggle activity lane\n- Ctrl+O switch lane mode (live/cockpit)\n- Ctrl+G force transcript refresh + jump latest"
        );
    } else if action != "status" {
        emit_command_output(
            app,
            "Usage: /telemetry [status|lane]\n- status: session/provider + gate snapshots\n- lane: status plus TUI activity-lane controls",
        );
        return Ok(CommandResult::Handled);
    }

    emit_command_output(app, out.trim_end());
    Ok(CommandResult::Handled)
}
