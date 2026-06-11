//! MCP profile helpers (Sentrux + YAML sync).

use std::path::Path;

use super::super::skills_infra::{SENTRUX_MCP_ARG, SENTRUX_MCP_COMMAND, SENTRUX_MCP_SERVER_NAME};

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

pub(super) fn update_yaml_mcp_server(
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

pub(crate) fn upsert_sentrux_mcp_profile(
    config_dir: &Path,
) -> Result<bool, hermes_core::AgentError> {
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

pub(crate) fn remove_sentrux_mcp_profile(config_dir: &Path) -> Result<(), hermes_core::AgentError> {
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

pub(super) fn sentrux_mcp_status(config_dir: &Path) -> (bool, bool, bool) {
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
