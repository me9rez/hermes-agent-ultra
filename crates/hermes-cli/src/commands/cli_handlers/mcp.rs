//! MCP CLI handler.

use std::sync::Arc;

use super::super::skills_infra::{SENTRUX_MCP_COMMAND, SENTRUX_MCP_SERVER_NAME};
use super::super::yes_no;
use super::mcp_profile::{
    remove_sentrux_mcp_profile, sentrux_mcp_status, update_yaml_mcp_server,
    upsert_sentrux_mcp_profile,
};

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
