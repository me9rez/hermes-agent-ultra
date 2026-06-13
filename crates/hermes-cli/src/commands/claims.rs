//! `/claims` slash command — claim verifier policy controls.

use hermes_core::AgentError;

use crate::alpha_runtime::{load_claim_verifier_policy, set_claim_verifier_enabled};
use crate::commands::{CommandResult, emit_command_output};

pub(crate) fn handle_claims_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
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
                host,
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
                host,
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
                host,
                format!(
                    "Claim verifier disabled.\nrequired={}\nmax_retries={}",
                    policy.required, policy.max_retries
                ),
            );
        }
        _ => emit_command_output(host, "Usage: /claims [status|on|off]"),
    }
    Ok(CommandResult::Handled)
}
