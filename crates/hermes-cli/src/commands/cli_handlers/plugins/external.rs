//! External Python plugin CLI subcommands.

use std::process::Stdio;

use hermes_core::AgentError;

use super::surface::discover_python_plugin_cli_commands;
pub async fn handle_cli_external_plugin_subcommand(raw: Vec<String>) -> Result<(), AgentError> {
    if raw.is_empty() {
        return Err(AgentError::Config(
            "Unknown command. Run `hermes --help` for available commands.".to_string(),
        ));
    }
    let command_name = raw[0].trim().to_string();
    let command_args: Vec<String> = raw[1..].to_vec();
    let available = discover_python_plugin_cli_commands();
    if !available.iter().any(|row| row.name == command_name) {
        let catalog = if available.is_empty() {
            "none discovered".to_string()
        } else {
            available
                .iter()
                .map(|row| {
                    if row.help.trim().is_empty() {
                        format!("  - {}", row.name)
                    } else {
                        format!("  - {}: {}", row.name, row.help.trim())
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        return Err(AgentError::Config(format!(
            "Unknown command '{}'. Run `hermes --help` for core commands.\nDiscovered plugin commands:\n{}",
            command_name, catalog
        )));
    }

    let args_json = serde_json::to_string(&command_args)
        .map_err(|e| AgentError::Config(format!("Failed to serialize plugin CLI args: {}", e)))?;
    let script = r#"
import argparse
import json
import sys

try:
    from plugins.memory import discover_plugin_cli_commands
except Exception as exc:
    print(f"Plugin CLI bridge unavailable: {exc}", file=sys.stderr)
    sys.exit(2)

name = sys.argv[1]
argv = json.loads(sys.argv[2])

for item in (discover_plugin_cli_commands() or []):
    if str(item.get("name", "")).strip() != name:
        continue
    setup = item.get("setup_fn")
    if not callable(setup):
        print(f"Plugin command '{name}' is missing setup_fn", file=sys.stderr)
        sys.exit(2)
    parser = argparse.ArgumentParser(prog=name)
    setup(parser)
    ns = parser.parse_args(argv)
    handler = item.get("handler_fn")
    if callable(handler):
        handler(ns)
        sys.exit(0)
    if hasattr(ns, "func") and callable(getattr(ns, "func")):
        ns.func(ns)
        sys.exit(0)
    parser.print_help()
    sys.exit(0)

print(f"Unknown plugin command: {name}", file=sys.stderr)
sys.exit(3)
"#;

    let output = tokio::process::Command::new("python3")
        .args(["-c", script, &command_name, &args_json])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|e| AgentError::Io(format!("Failed to execute plugin command: {}", e)))?;
    if !output.success() {
        return Err(AgentError::Config(format!(
            "Plugin command '{}' failed with exit code {:?}.",
            command_name,
            output.code()
        )));
    }
    Ok(())
}
