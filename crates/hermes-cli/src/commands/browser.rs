use hermes_core::AgentError;

use crate::commands::{CommandResult, emit_command_output};

pub(crate) fn persist_browser_cdp_url(url: Option<&str>) -> Result<(), AgentError> {
    let env_path = hermes_config::hermes_home().join(".env");
    let mut lines: Vec<String> = std::fs::read_to_string(&env_path)
        .unwrap_or_default()
        .lines()
        .map(|line| line.to_string())
        .collect();
    let key = "CHROME_CDP_URL=";
    lines.retain(|line| !line.starts_with(key));
    if let Some(value) = url.map(str::trim).filter(|v| !v.is_empty()) {
        lines.push(format!("CHROME_CDP_URL={}", value));
    }
    if let Some(parent) = env_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AgentError::Io(format!("Failed to create {}: {}", parent.display(), e)))?;
    }
    let mut payload = lines.join("\n");
    if !payload.is_empty() {
        payload.push('\n');
    }
    std::fs::write(&env_path, payload)
        .map_err(|e| AgentError::Io(format!("Failed to write {}: {}", env_path.display(), e)))?;
    Ok(())
}

pub(crate) fn browser_http_probe_base(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    if let Some(rest) = trimmed.strip_prefix("ws://") {
        format!("http://{}", rest)
    } else if let Some(rest) = trimmed.strip_prefix("wss://") {
        format!("https://{}", rest)
    } else {
        trimmed.to_string()
    }
}

pub(crate) async fn browser_probe(endpoint: &str) -> Result<String, AgentError> {
    let base = browser_http_probe_base(endpoint)
        .trim_end_matches('/')
        .to_string();
    let url = format!("{}/json/version", base);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .map_err(|e| AgentError::Io(format!("Failed to create browser probe client: {}", e)))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AgentError::Io(format!("Browser probe failed at {}: {}", url, e)))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .unwrap_or_else(|_| String::from("<unavailable>"));
    if !status.is_success() {
        return Err(AgentError::Io(format!(
            "Browser probe failed at {} with status {}",
            url, status
        )));
    }
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AgentError::Config(format!("Browser probe parse failed: {}", e)))?;
    let browser = payload
        .get("Browser")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let ws_url = payload
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>");
    Ok(format!(
        "Connected CDP endpoint: {}\nBrowser: {}\nWebSocket target: {}",
        endpoint.trim(),
        browser,
        ws_url
    ))
}

pub(crate) async fn handle_browser_command(
    host: &mut impl crate::app::SlashCommandHost,
    args: &[&str],
) -> Result<CommandResult, AgentError> {
    let action = args
        .first()
        .copied()
        .unwrap_or("status")
        .to_ascii_lowercase();
    match action.as_str() {
        "status" | "show" => {
            let endpoint = std::env::var("CHROME_CDP_URL")
                .unwrap_or_else(|_| "http://localhost:9222".to_string());
            match browser_probe(&endpoint).await {
                Ok(summary) => emit_command_output(host, summary),
                Err(err) => emit_command_output(
                    host,
                    format!(
                        "Browser status (configured endpoint: {})\nProbe error: {}\nTip: `/browser connect [ws://host:port or http://host:port]`",
                        endpoint, err
                    ),
                ),
            }
            Ok(CommandResult::Handled)
        }
        "connect" => {
            let endpoint = args.get(1).copied().unwrap_or("http://localhost:9222");
            crate::env_vars::set_var("CHROME_CDP_URL", endpoint);
            persist_browser_cdp_url(Some(endpoint))?;
            match browser_probe(endpoint).await {
                Ok(summary) => emit_command_output(
                    host,
                    format!(
                        "{}\n\nSaved CHROME_CDP_URL to {}/.env",
                        summary,
                        hermes_config::hermes_home().display()
                    ),
                ),
                Err(err) => emit_command_output(
                    host,
                    format!(
                        "Saved CHROME_CDP_URL={}, but probe failed: {}\nStart Chrome with --remote-debugging-port=9222 and retry `/browser status`.",
                        endpoint, err
                    ),
                ),
            }
            Ok(CommandResult::Handled)
        }
        "disconnect" => {
            crate::env_vars::remove_var("CHROME_CDP_URL");
            persist_browser_cdp_url(None)?;
            emit_command_output(
                host,
                "Browser CDP override removed. Runtime will fall back to default local endpoint (http://localhost:9222) unless configured elsewhere.",
            );
            Ok(CommandResult::Handled)
        }
        _ => {
            emit_command_output(
                host,
                "Usage: /browser [status|connect [ws://host:port|http://host:port]|disconnect]",
            );
            Ok(CommandResult::Handled)
        }
    }
}
