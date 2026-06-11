//! Contribute CLI handler.

fn hermes_home_from_config(config: &hermes_config::GatewayConfig) -> std::path::PathBuf {
    config
        .home_dir
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(hermes_config::hermes_home)
}
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
