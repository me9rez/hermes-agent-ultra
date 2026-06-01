//! agent-browser CLI subprocess backend (Python `browser_tool._run_browser_command` parity).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;
use uuid::Uuid;

use super::browser_snapshot_util::process_snapshot_text;
use crate::tools::browser::BrowserBackend;
use hermes_core::ToolError;

const DEFAULT_TASK_ID: &str = "default";
const COMMAND_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone)]
enum BrowserCommand {
    Direct(PathBuf),
    Npx,
}

/// Returns true when agent-browser CLI is discoverable on PATH or via npx.
pub fn is_available() -> bool {
    resolve_browser_command().is_some()
}

fn resolve_browser_command() -> Option<BrowserCommand> {
    if let Ok(explicit) = std::env::var("HERMES_AGENT_BROWSER_CMD") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(BrowserCommand::Direct(PathBuf::from(trimmed)));
        }
    }
    if which_agent_browser().is_some() {
        return which_agent_browser();
    }
    if which_npx().is_some() {
        return Some(BrowserCommand::Npx);
    }
    None
}

fn which_agent_browser() -> Option<BrowserCommand> {
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(if cfg!(windows) {
                "agent-browser.cmd"
            } else {
                "agent-browser"
            });
            if candidate.is_file() {
                return Some(BrowserCommand::Direct(candidate));
            }
            let plain = dir.join("agent-browser");
            if plain.is_file() {
                return Some(BrowserCommand::Direct(plain));
            }
        }
        None
    })
}

fn which_npx() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(if cfg!(windows) { "npx.cmd" } else { "npx" });
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

fn effective_task_id(task_id: Option<&str>) -> String {
    task_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TASK_ID)
        .to_string()
}

fn normalize_ref(ref_id: &str) -> String {
    let trimmed = ref_id.trim();
    if trimmed.starts_with('@') {
        trimmed.to_string()
    } else {
        format!("@{trimmed}")
    }
}

pub struct AgentBrowserBackend {
    cmd: BrowserCommand,
    sessions: Mutex<HashMap<String, String>>,
}

impl AgentBrowserBackend {
    pub fn new() -> Result<Self, ToolError> {
        let cmd = resolve_browser_command().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "agent-browser CLI not found. Install with: npm install -g agent-browser \
                 && agent-browser install --with-deps"
                    .into(),
            )
        })?;
        Ok(Self {
            cmd,
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn try_new() -> Option<Self> {
        Self::new().ok()
    }

    fn session_name_for(&self, task_id: &str) -> String {
        let mut guard = self.sessions.lock().expect("browser sessions lock");
        guard
            .entry(task_id.to_string())
            .or_insert_with(|| format!("h_{}", &Uuid::new_v4().simple().to_string()[..10]))
            .clone()
    }

    fn socket_dir(&self, session_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("agent-browser-{session_name}"))
    }

    async fn run_command(
        &self,
        task_id: &str,
        command: &str,
        args: &[String],
        timeout_secs: u64,
    ) -> Result<Value, ToolError> {
        let session_name = self.session_name_for(task_id);
        let socket_dir = self.socket_dir(&session_name);
        std::fs::create_dir_all(&socket_dir).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to create browser socket dir: {e}"))
        })?;

        let mut cmd = match &self.cmd {
            BrowserCommand::Direct(path) => Command::new(path),
            BrowserCommand::Npx => {
                let npx = which_npx().unwrap_or_else(|| PathBuf::from("npx"));
                let mut c = Command::new(npx);
                c.arg("agent-browser");
                c
            }
        };

        cmd.arg("--session")
            .arg(&session_name)
            .arg("--json")
            .arg(command)
            .args(args)
            .env("AGENT_BROWSER_SOCKET_DIR", &socket_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            cmd.as_std_mut().creation_flags(0x08000000);
        }

        let timeout = Duration::from_secs(timeout_secs.max(5));
        let child = cmd.spawn().map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to spawn agent-browser: {e}"))
        })?;

        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(format!(
                    "agent-browser '{command}' timed out after {timeout_secs}s"
                ))
            })?
            .map_err(|e| ToolError::ExecutionFailed(format!("agent-browser wait failed: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolError::ExecutionFailed(format!(
                "agent-browser '{command}' produced no output (status={:?}): {stderr}",
                output.status.code()
            )));
        }

        serde_json::from_str(stdout.trim()).map_err(|e| {
            ToolError::ExecutionFailed(format!(
                "agent-browser JSON parse error: {e}; stdout={}",
                stdout.chars().take(400).collect::<String>()
            ))
        })
    }

    fn command_timeout_secs() -> u64 {
        std::env::var("HERMES_BROWSER_COMMAND_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(COMMAND_TIMEOUT_SECS)
    }
}

#[async_trait]
impl BrowserBackend for AgentBrowserBackend {
    async fn navigate(&self, url: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let started = Instant::now();
        let result = self
            .run_command(
                &task,
                "open",
                &[url.to_string()],
                Self::command_timeout_secs().max(60),
            )
            .await?;
        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "status": "navigated",
            "url": url,
            "task_id": task,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "backend": "agent-browser",
            "data": result.get("data").cloned().unwrap_or(result),
        })
        .to_string())
    }

    async fn snapshot(
        &self,
        full: bool,
        user_task: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let started = Instant::now();
        let mut args = Vec::new();
        if !full {
            args.push("-c".to_string());
        }
        let result = self
            .run_command(&task, "snapshot", &args, Self::command_timeout_secs())
            .await?;

        let mut snapshot_text = result
            .get("data")
            .and_then(|d| d.get("snapshot"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        if !snapshot_text.is_empty() {
            snapshot_text = process_snapshot_text(&snapshot_text, user_task).await;
        }

        let refs = result
            .get("data")
            .and_then(|d| d.get("refs"))
            .cloned()
            .unwrap_or(json!({}));

        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "snapshot": snapshot_text,
            "element_count": refs.as_object().map(|m| m.len()).unwrap_or(0),
            "full": full,
            "user_task": user_task,
            "task_id": task,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "backend": "agent-browser",
        })
        .to_string())
    }

    async fn click(&self, ref_id: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let reference = normalize_ref(ref_id);
        let result = self
            .run_command(
                &task,
                "click",
                &[reference.clone()],
                Self::command_timeout_secs(),
            )
            .await?;
        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "clicked": reference,
            "task_id": task,
            "backend": "agent-browser",
        })
        .to_string())
    }

    async fn r#type(
        &self,
        ref_id: &str,
        text: &str,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let reference = normalize_ref(ref_id);
        let result = self
            .run_command(
                &task,
                "fill",
                &[reference.clone(), text.to_string()],
                Self::command_timeout_secs(),
            )
            .await?;
        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "typed": reference,
            "text": text,
            "task_id": task,
            "backend": "agent-browser",
        })
        .to_string())
    }

    async fn scroll(
        &self,
        direction: &str,
        amount: Option<u32>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let px = amount.unwrap_or(500);
        let result = self
            .run_command(
                &task,
                "scroll",
                &[direction.to_string(), px.to_string()],
                Self::command_timeout_secs(),
            )
            .await?;
        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "direction": direction,
            "amount": px,
            "task_id": task,
            "backend": "agent-browser",
            "data": result.get("data").cloned().unwrap_or(result),
        })
        .to_string())
    }

    async fn go_back(&self, task_id: Option<&str>) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let result = self
            .run_command(&task, "back", &[], Self::command_timeout_secs())
            .await?;
        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "status": "navigated_back",
            "task_id": task,
            "backend": "agent-browser",
        })
        .to_string())
    }

    async fn press(&self, key: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let result = self
            .run_command(
                &task,
                "press",
                &[key.to_string()],
                Self::command_timeout_secs(),
            )
            .await?;
        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "key": key,
            "task_id": task,
            "backend": "agent-browser",
        })
        .to_string())
    }

    async fn get_images(
        &self,
        selector: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let sel = selector.unwrap_or("img");
        let js = format!(
            "JSON.stringify(Array.from(document.querySelectorAll('{sel}')).map(img => ({{src: img.src, alt: img.alt}})))"
        );
        let result = self
            .run_command(&task, "eval", &[js], Self::command_timeout_secs())
            .await?;
        Ok(json!({
            "success": true,
            "selector": sel,
            "task_id": task,
            "backend": "agent-browser",
            "data": result.get("data").cloned().unwrap_or(result),
        })
        .to_string())
    }

    async fn vision(
        &self,
        instruction: &str,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        Ok(json!({
            "success": false,
            "error": "browser_vision requires screenshot pipeline; use browser_snapshot + vision_analyze for now",
            "instruction": instruction,
            "task_id": task,
            "backend": "agent-browser",
        })
        .to_string())
    }

    async fn console(
        &self,
        action: &str,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let task = effective_task_id(task_id);
        let (cmd, args): (&str, Vec<String>) = match action {
            "clear" => ("console", vec!["--clear".to_string()]),
            "read" | _ => ("console", vec![]),
        };
        let result = self
            .run_command(&task, cmd, &args, Self::command_timeout_secs())
            .await?;
        Ok(json!({
            "success": result.get("success").and_then(|v| v.as_bool()).unwrap_or(true),
            "action": action,
            "task_id": task,
            "backend": "agent-browser",
            "data": result.get("data").cloned().unwrap_or(result),
        })
        .to_string())
    }
}

/// Select browser backend: agent-browser when available unless forced to CDP.
pub fn create_browser_backend() -> std::sync::Arc<dyn BrowserBackend> {
    use super::browser::CdpBrowserBackend;
    let forced = std::env::var("HERMES_BROWSER_BACKEND")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase());
    match forced.as_deref() {
        Some("cdp") => std::sync::Arc::new(CdpBrowserBackend::from_env()),
        Some("agent-browser") => {
            if let Ok(backend) = AgentBrowserBackend::new() {
                std::sync::Arc::new(backend)
            } else {
                std::sync::Arc::new(CdpBrowserBackend::from_env())
            }
        }
        _ => {
            if let Some(backend) = AgentBrowserBackend::try_new() {
                std::sync::Arc::new(backend)
            } else {
                std::sync::Arc::new(CdpBrowserBackend::from_env())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ref_adds_at_prefix() {
        assert_eq!(normalize_ref("e5"), "@e5");
        assert_eq!(normalize_ref("@e5"), "@e5");
    }

    #[test]
    fn effective_task_id_defaults() {
        assert_eq!(effective_task_id(None), "default");
        assert_eq!(effective_task_id(Some("abc")), "abc");
    }
}
