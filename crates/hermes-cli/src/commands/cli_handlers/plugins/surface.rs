//! Plugin discovery and surface rendering.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use hermes_agent::plugins::PluginManifest;
use hermes_core::AgentError;
use serde::Deserialize;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PluginSurfaceSource {
    User,
    Project,
    Entrypoint,
}

impl PluginSurfaceSource {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            PluginSurfaceSource::User => "user",
            PluginSurfaceSource::Project => "project",
            PluginSurfaceSource::Entrypoint => "entrypoint",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PluginSurfaceEntry {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) description: String,
    pub(crate) kind: Option<String>,
    pub(crate) source: PluginSurfaceSource,
    pub(crate) path: Option<PathBuf>,
    pub(crate) enabled: bool,
    pub(crate) entrypoint_value: Option<String>,
    pub(crate) entrypoint_dist: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PythonEntrypointPayload {
    #[serde(default)]
    entries: Vec<PythonEntrypointItem>,
}

#[derive(Debug, Deserialize)]
struct PythonEntrypointItem {
    name: String,
    value: String,
    #[serde(default)]
    dist: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PythonPluginCommandPayload {
    #[serde(default)]
    commands: Vec<PythonPluginCommandItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct PythonPluginCommandItem {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) help: String,
}

fn coerce_memory_provider_kind(path: &Path, kind: Option<String>) -> Option<String> {
    let explicit_kind = kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    if explicit_kind.is_some() {
        return explicit_kind;
    }
    let init_file = path.join("__init__.py");
    let Ok(source) = std::fs::read_to_string(&init_file) else {
        return None;
    };
    let probe = if source.len() > 8192 {
        &source[..8192]
    } else {
        source.as_str()
    };
    if probe.contains("register_memory_provider") || probe.contains("MemoryProvider") {
        Some("exclusive".to_string())
    } else {
        None
    }
}

fn scan_plugin_manifest_root(root: &Path, source: PluginSurfaceSource) -> Vec<PluginSurfaceEntry> {
    let mut out = Vec::new();
    if !root.exists() {
        return out;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("plugin.yaml");
        if !manifest_path.exists() {
            continue;
        }
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match serde_yaml::from_str(&content) {
            Ok(manifest) => manifest,
            Err(_) => continue,
        };
        let disabled_marker = path.join(".disabled");
        out.push(PluginSurfaceEntry {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            kind: coerce_memory_provider_kind(&path, manifest.kind.clone()),
            source,
            path: Some(path),
            enabled: !disabled_marker.exists(),
            entrypoint_value: None,
            entrypoint_dist: None,
        });
    }
    out
}

fn discover_python_entrypoint_plugins() -> Vec<PluginSurfaceEntry> {
    let script = r#"
import json
from importlib import metadata

def _entry_points():
    eps = metadata.entry_points()
    if hasattr(eps, "select"):
        return list(eps.select(group="hermes_agent.plugins"))
    if isinstance(eps, dict):
        return list(eps.get("hermes_agent.plugins", []))
    return [ep for ep in eps if getattr(ep, "group", "") == "hermes_agent.plugins"]

rows = []
try:
    for ep in _entry_points():
        dist = None
        try:
            if getattr(ep, "dist", None):
                dist = ep.dist.name
        except Exception:
            dist = None
        rows.append({
            "name": str(getattr(ep, "name", "") or ""),
            "value": str(getattr(ep, "value", "") or ""),
            "dist": dist,
        })
except Exception:
    rows = []
print(json.dumps({"entries": rows}))
"#;

    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let payload: PythonEntrypointPayload = match serde_json::from_slice(&output.stdout) {
        Ok(payload) => payload,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in payload.entries {
        let name = item.name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.push(PluginSurfaceEntry {
            name,
            version: "entrypoint".to_string(),
            description: String::new(),
            kind: None,
            source: PluginSurfaceSource::Entrypoint,
            path: None,
            enabled: true,
            entrypoint_value: Some(item.value),
            entrypoint_dist: item.dist,
        });
    }
    out
}

pub(crate) fn discover_plugin_surface(include_entrypoints: bool) -> Vec<PluginSurfaceEntry> {
    let mut rows = Vec::new();
    let user_root = hermes_config::hermes_home().join("plugins");
    rows.extend(scan_plugin_manifest_root(
        &user_root,
        PluginSurfaceSource::User,
    ));

    if hermes_config::env_var_enabled("HERMES_ENABLE_PROJECT_PLUGINS") {
        if let Ok(cwd) = std::env::current_dir() {
            let project_root = hermes_config::project_hermes_dir(&cwd).join("plugins");
            rows.extend(scan_plugin_manifest_root(
                &project_root,
                PluginSurfaceSource::Project,
            ));
        }
    }

    if include_entrypoints {
        rows.extend(discover_python_entrypoint_plugins());
    }

    rows.sort_by(|a, b| {
        a.source.cmp(&b.source).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    rows
}

pub(super) fn resolve_local_plugin_path_by_name(name: &str) -> Option<PathBuf> {
    discover_plugin_surface(false)
        .into_iter()
        .filter_map(|row| {
            if row.name.eq_ignore_ascii_case(name) {
                row.path
            } else {
                None
            }
        })
        .next()
}

pub(crate) fn render_plugin_surface_table(rows: &[PluginSurfaceEntry]) -> String {
    if rows.is_empty() {
        return "  (no plugins discovered)".to_string();
    }
    let mut out = String::new();
    for row in rows {
        let status = if row.enabled { "enabled" } else { "disabled" };
        let mut meta_parts = vec![format!("source={}", row.source.label())];
        if let Some(kind) = row.kind.as_deref().filter(|k| !k.trim().is_empty()) {
            meta_parts.push(format!("kind={}", kind));
        }
        if let Some(dist) = row
            .entrypoint_dist
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            meta_parts.push(format!("dist={}", dist));
        }
        if let Some(value) = row
            .entrypoint_value
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            meta_parts.push(format!("entry={}", value));
        }
        let path = row
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        let version = if row.version.trim().is_empty() {
            "unknown".to_string()
        } else {
            row.version.clone()
        };
        let description = row.description.trim();
        let _ = writeln!(
            out,
            "  • {} v{} [{}; {}; path={}]",
            row.name,
            version,
            status,
            meta_parts.join(", "),
            path
        );
        if !description.is_empty() {
            let _ = writeln!(out, "    {}", description);
        }
    }
    out.trim_end().to_string()
}

fn set_plugin_enabled(path: &Path, enable: bool) -> Result<(), AgentError> {
    let marker = path.join(".disabled");
    if enable {
        if marker.exists() {
            std::fs::remove_file(&marker)
                .map_err(|e| AgentError::Io(format!("Failed to enable plugin: {}", e)))?;
        }
    } else {
        std::fs::write(&marker, "")
            .map_err(|e| AgentError::Io(format!("Failed to disable plugin: {}", e)))?;
    }
    Ok(())
}

fn parse_selection_indices(raw: &str, max: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for token in raw.split(|c: char| c == ',' || c.is_ascii_whitespace()) {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(idx) = trimmed.parse::<usize>() else {
            continue;
        };
        if idx == 0 || idx > max {
            continue;
        }
        out.push(idx - 1);
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub(super) fn run_plugins_interactive_toggle() -> Result<(), AgentError> {
    let mut rows: Vec<PluginSurfaceEntry> = discover_plugin_surface(false)
        .into_iter()
        .filter(|row| row.path.is_some())
        .collect();
    if rows.is_empty() {
        println!("No plugin bundles discovered.");
        println!("Install one with: hermes plugins install <owner/repo>  (or a trusted git URL)");
        return Ok(());
    }

    rows.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });

    println!("Plugin toggle UI (interactive)");
    println!("------------------------------");
    println!("Source roots:");
    println!(
        "  - user:    {}",
        hermes_config::hermes_home().join("plugins").display()
    );
    if hermes_config::env_var_enabled("HERMES_ENABLE_PROJECT_PLUGINS") {
        if let Ok(cwd) = std::env::current_dir() {
            println!(
                "  - project: {}",
                hermes_config::project_hermes_dir(&cwd)
                    .join("plugins")
                    .display()
            );
        }
    } else {
        println!("  - project: disabled (set HERMES_ENABLE_PROJECT_PLUGINS=true)");
    }
    println!();

    let mut provider_indices = Vec::new();
    println!("General Plugins");
    for (idx, row) in rows.iter().enumerate() {
        let is_provider = row.kind.as_deref() == Some("exclusive");
        if is_provider {
            provider_indices.push(idx);
            continue;
        }
        let mark = if row.enabled { "✓" } else { " " };
        println!(
            "  {:>2}. [{}] {} (source={})",
            idx + 1,
            mark,
            row.name,
            row.source.label()
        );
    }

    if !provider_indices.is_empty() {
        println!();
        println!("Provider Plugins (single-select recommended)");
        for idx in &provider_indices {
            let row = &rows[*idx];
            let mark = if row.enabled { "✓" } else { " " };
            println!(
                "  {:>2}. [{}] {} (source={}, kind={})",
                idx + 1,
                mark,
                row.name,
                row.source.label(),
                row.kind.clone().unwrap_or_else(|| "provider".to_string())
            );
        }
    }

    use std::io::Write as _;
    print!("\nToggle plugin numbers (comma/space separated, Enter to skip): ");
    let _ = std::io::stdout().flush();
    let mut toggle_buf = String::new();
    std::io::stdin()
        .read_line(&mut toggle_buf)
        .map_err(|e| AgentError::Io(format!("Failed to read selection: {}", e)))?;
    let toggle_indices = parse_selection_indices(&toggle_buf, rows.len());
    for idx in toggle_indices {
        if let Some(path) = rows[idx].path.as_deref() {
            let target = !rows[idx].enabled;
            set_plugin_enabled(path, target)?;
            rows[idx].enabled = target;
        }
    }

    if !provider_indices.is_empty() {
        print!("Activate exactly one provider plugin number (Enter to keep current): ");
        let _ = std::io::stdout().flush();
        let mut provider_buf = String::new();
        std::io::stdin()
            .read_line(&mut provider_buf)
            .map_err(|e| AgentError::Io(format!("Failed to read provider selection: {}", e)))?;
        let selected = parse_selection_indices(&provider_buf, rows.len());
        if let Some(selected_idx) = selected.first().copied() {
            if provider_indices.contains(&selected_idx) {
                for idx in provider_indices {
                    if let Some(path) = rows[idx].path.as_deref() {
                        let should_enable = idx == selected_idx;
                        set_plugin_enabled(path, should_enable)?;
                        rows[idx].enabled = should_enable;
                    }
                }
            } else {
                println!(
                    "Selection {} is not a provider plugin row; keeping provider state unchanged.",
                    selected_idx + 1
                );
            }
        }
    }

    println!("\nUpdated plugin state:");
    println!("{}", render_plugin_surface_table(&rows));
    Ok(())
}

pub(super) fn discover_python_plugin_cli_commands() -> Vec<PythonPluginCommandItem> {
    let script = r#"
import json
rows = []
try:
    from plugins.memory import discover_plugin_cli_commands
    for cmd in (discover_plugin_cli_commands() or []):
        name = str(cmd.get("name", "") or "").strip()
        if not name:
            continue
        help_text = str(cmd.get("help") or cmd.get("description") or "")
        rows.append({"name": name, "help": help_text})
except Exception:
    rows = []
print(json.dumps({"commands": rows}))
"#;
    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let payload: PythonPluginCommandPayload = match serde_json::from_slice(&output.stdout) {
        Ok(payload) => payload,
        Err(_) => return Vec::new(),
    };
    let mut rows = payload.commands;
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows.dedup_by(|a, b| a.name == b.name);
    rows
}
