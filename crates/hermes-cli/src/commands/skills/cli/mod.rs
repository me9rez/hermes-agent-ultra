mod admin;
mod install;
mod lifecycle;
mod list;
mod search;

use hermes_core::AgentError;

use super::tier::{
    skills_action_blocked_by_tier, skills_execution_tier, skills_tier_bypass_enabled,
};

pub async fn handle_cli_skills(
    action: Option<String>,
    name: Option<String>,
    extra: Option<String>,
) -> Result<(), AgentError> {
    let requested_action = action.as_deref().unwrap_or("list");
    if !skills_tier_bypass_enabled() {
        let tier = skills_execution_tier();
        let denied = skills_action_blocked_by_tier(tier, requested_action, name.as_deref());

        if denied {
            return Err(AgentError::Config(format!(
                "skills action '{}' is blocked by skills tier '{}'. Use `/ops skills-tier open` or set HERMES_SKILLS_TIER_BYPASS=1 to override intentionally.",
                requested_action,
                tier.as_str()
            )));
        }
    }

    let skills_dir = hermes_config::hermes_home().join("skills");

    match action.as_deref().unwrap_or("list") {
        "list" => list::run_list(&skills_dir),
        "browse" => list::run_browse(&skills_dir),
        "search" => search::run_search(name, &skills_dir).await,
        "install" => install::run_install(name, extra, &skills_dir).await,
        "reset" => lifecycle::run_reset(name, &skills_dir),
        "subscribe" => lifecycle::run_subscribe(name, extra, &skills_dir),
        "inspect" => lifecycle::run_inspect(name, &skills_dir),
        "uninstall" => lifecycle::run_uninstall(name, &skills_dir),
        "check" => lifecycle::run_check(name, &skills_dir),
        "update" => lifecycle::run_update(extra, &skills_dir).await,
        "publish" => admin::run_publish(name, &skills_dir).await,
        "snapshot" => admin::run_snapshot(name, extra, &skills_dir),
        "tap" => admin::run_tap(name, extra, &skills_dir),
        "config" => admin::run_config(name, extra, &skills_dir),
        "quality" => admin::run_quality(&skills_dir),
        "audit" => admin::run_audit(name, &skills_dir),
        other => {
            println!("Skills action '{}' is not recognized.", other);
            println!(
                "Available actions: list, browse, search, install, inspect, uninstall, check, update, publish, snapshot, tap, config, quality, audit"
            );
            Ok(())
        }
    }
}
