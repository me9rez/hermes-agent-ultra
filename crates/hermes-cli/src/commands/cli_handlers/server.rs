//! `hermes server` — remote LLM server account commands.

use hermes_config::{ServerConfig, hermes_home, load_config};
use hermes_core::AgentError;
use hermes_server_client::{AuthManager, LoginMethod, ServerClientError, run_doctor};

pub async fn handle_cli_server(
    action: Option<String>,
    method: Option<String>,
    config_dir: Option<&str>,
) -> Result<(), AgentError> {
    let config = load_config(config_dir).map_err(|e| AgentError::Config(e.to_string()))?;
    let action = action
        .as_deref()
        .unwrap_or("help")
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "login" => server_login(&config.server, method.as_deref()).await,
        "logout" => server_logout(&config.server).await,
        "whoami" => server_whoami(&config.server).await,
        "doctor" => server_doctor(&config.server, config_dir).await,
        "help" | "--help" | "-h" => {
            print_server_help();
            Ok(())
        }
        other => Err(AgentError::Config(format!(
            "unknown server subcommand '{other}'. Try: hermes server help"
        ))),
    }
}

fn print_server_help() {
    println!("Remote LLM server account (login + OpenAI-compatible gateway)");
    println!();
    println!("Usage:");
    println!("  hermes server login [--method wechat|email]");
    println!("  hermes server logout");
    println!("  hermes server whoami");
    println!("  hermes server doctor");
    println!();
    println!("Configure server.base_url and server.enabled in config.yaml, or use:");
    println!("  HERMES_SERVER_URL, HERMES_SERVER_ENABLED, HERMES_SERVER_TOKEN");
}

async fn server_login(config: &ServerConfig, method: Option<&str>) -> Result<(), AgentError> {
    let login_method =
        parse_method_arg(method).unwrap_or_else(|| config.auth.preferred_method.into());

    let manager = AuthManager::new(config.clone(), hermes_home()).map_err(server_client_err)?;

    println!("Login method: {}", login_method.label());
    if config.base_url.trim().is_empty() {
        println!("Warning: server.base_url is empty — set it before API integration.");
    }

    match manager.start_login(login_method).await {
        Ok(pending) => {
            println!("{}", pending.message);
            if let Some(qr) = pending.qr_content.as_deref() {
                println!("QR: {qr}");
            }
            Ok(())
        }
        Err(ServerClientError::NotConfigured(msg)) => {
            println!("Login scaffold ready; waiting for server API documentation.");
            println!("Detail: {msg}");
            Ok(())
        }
        Err(err) => Err(server_client_err(err)),
    }
}

async fn server_logout(config: &ServerConfig) -> Result<(), AgentError> {
    if std::env::var("HERMES_SERVER_TOKEN")
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        println!("HERMES_SERVER_TOKEN is set in the environment; unset it to fully logout.");
        return Ok(());
    }

    let manager = AuthManager::new(config.clone(), hermes_home()).map_err(server_client_err)?;
    let removed = manager.logout().await.map_err(server_client_err)?;
    if removed {
        println!("Logged out from remote LLM server.");
    } else {
        println!("No stored remote server credentials.");
    }
    Ok(())
}

async fn server_whoami(config: &ServerConfig) -> Result<(), AgentError> {
    let manager = AuthManager::new(config.clone(), hermes_home()).map_err(server_client_err)?;
    let status = manager.whoami().await.map_err(server_client_err)?;

    println!("Remote LLM server");
    println!("  enabled: {}", status.server_enabled);
    println!(
        "  base_url: {}",
        if status.base_url.is_empty() {
            "(not set)"
        } else {
            status.base_url.as_str()
        }
    );
    println!("  token source: {}", status.source);

    if status.is_logged_in() {
        let expired = status.token_expired();
        println!(
            "  status: logged in{}",
            if expired { " (token expired)" } else { "" }
        );
    } else {
        println!("  status: not logged in — run `hermes server login`");
    }
    Ok(())
}

async fn server_doctor(config: &ServerConfig, config_dir: Option<&str>) -> Result<(), AgentError> {
    let _ = config_dir;
    let home = hermes_home();
    let report = run_doctor(config, &home).await;
    println!("Remote LLM server diagnostics");
    for line in report.print_lines() {
        println!("  {line}");
    }
    if !report.all_ok() && config.enabled {
        return Err(AgentError::Config(
            "one or more server checks failed".to_string(),
        ));
    }
    Ok(())
}

fn parse_method_arg(method: Option<&str>) -> Option<LoginMethod> {
    method.and_then(LoginMethod::parse)
}

fn server_client_err(err: ServerClientError) -> AgentError {
    match err {
        ServerClientError::Agent(e) => e,
        ServerClientError::Disabled => {
            AgentError::Config("server integration disabled (set server.enabled=true)".into())
        }
        other => AgentError::Config(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_method_aliases() {
        assert_eq!(
            parse_method_arg(Some("wechat")),
            Some(LoginMethod::WechatQr)
        );
        assert_eq!(parse_method_arg(Some("email")), Some(LoginMethod::EmailOtp));
    }
}
