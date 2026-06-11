//! Interest CLI handler.

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
