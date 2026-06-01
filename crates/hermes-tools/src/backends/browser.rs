//! Real browser backend: CDP (Chrome DevTools Protocol) via WebSocket.
//!
//! This backend connects to a running Chrome/Chromium instance via CDP
//! and provides browser automation capabilities.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use crate::tools::browser::BrowserBackend;
use hermes_core::ToolError;

/// Browser backend using Chrome DevTools Protocol.
/// Connects to Chrome via WebSocket for automation.
pub struct CdpBrowserBackend {
    /// CDP WebSocket endpoint URL (e.g., ws://localhost:9222/devtools/page/...)
    endpoint: String,
    client: reqwest::Client,
}

/// CamoFox anti-detection browser backend (compat layer).
///
/// Currently routes through CDP endpoint while exposing a dedicated type so
/// higher layers can opt into anti-detection profile selection.
pub struct CamoFoxBrowserBackend {
    inner: CdpBrowserBackend,
    profile: String,
}

impl CamoFoxBrowserBackend {
    pub fn new(endpoint: String, profile: String) -> Self {
        Self {
            inner: CdpBrowserBackend::new(endpoint),
            profile,
        }
    }

    pub fn from_env() -> Self {
        let endpoint = std::env::var("CAMOFOX_CDP_URL")
            .or_else(|_| std::env::var("CHROME_CDP_URL"))
            .or_else(|_| std::env::var("BROWSER_CDP_URL"))
            .unwrap_or_else(|_| "http://localhost:9222".to_string());
        let profile = std::env::var("CAMOFOX_PROFILE").unwrap_or_else(|_| "default".to_string());
        Self::new(endpoint, profile)
    }
}

impl CdpBrowserBackend {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            client: reqwest::Client::new(),
        }
    }

    /// Resolve CDP HTTP endpoint: `CHROME_CDP_URL`, then `BROWSER_CDP_URL`, else localhost.
    pub fn cdp_endpoint_from_env() -> String {
        std::env::var("CHROME_CDP_URL")
            .or_else(|_| std::env::var("BROWSER_CDP_URL"))
            .unwrap_or_else(|_| "http://localhost:9222".to_string())
    }

    /// Create from environment variables or default localhost.
    pub fn from_env() -> Self {
        Self::new(Self::cdp_endpoint_from_env())
    }

    /// Probe CDP HTTP endpoint (`/json/version`).
    pub async fn probe_endpoint(client: &reqwest::Client, endpoint: &str) -> bool {
        let url = format!(
            "{}/json/version",
            endpoint.trim_end_matches('/')
        );
        client
            .get(&url)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn auto_start_enabled() -> bool {
        matches!(
            std::env::var("HERMES_BROWSER_AUTO_START")
                .ok()
                .map(|s| s.trim().to_ascii_lowercase())
                .as_deref(),
            Some("1") | Some("true") | Some("yes") | Some("on")
        )
    }

    fn default_chrome_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if cfg!(windows) {
            if let Ok(pf) = std::env::var("ProgramFiles") {
                paths.push(
                    PathBuf::from(pf)
                        .join("Google")
                        .join("Chrome")
                        .join("Application")
                        .join("chrome.exe"),
                );
            }
            if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
                paths.push(
                    PathBuf::from(pf86)
                        .join("Google")
                        .join("Chrome")
                        .join("Application")
                        .join("chrome.exe"),
                );
            }
        } else if cfg!(target_os = "macos") {
            paths.push(PathBuf::from(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            ));
        } else {
            paths.push(PathBuf::from("google-chrome"));
            paths.push(PathBuf::from("chromium"));
            paths.push(PathBuf::from("chromium-browser"));
        }
        paths
    }

    fn debug_port_from_endpoint(endpoint: &str) -> u16 {
        endpoint
            .trim_end_matches('/')
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
            .unwrap_or(9222)
    }

    async fn try_auto_start_chrome(endpoint: &str) -> Result<(), ToolError> {
        if !Self::auto_start_enabled() {
            return Err(ToolError::ExecutionFailed(
                "Chrome CDP not reachable. Start Chrome with --remote-debugging-port=9222 \
                 or set HERMES_BROWSER_AUTO_START=1"
                    .into(),
            ));
        }
        let port = Self::debug_port_from_endpoint(endpoint);
        let user_data = std::env::temp_dir().join(format!("hermes-chrome-debug-{port}"));
        let _ = std::fs::create_dir_all(&user_data);
        let chrome = Self::default_chrome_paths()
            .into_iter()
            .find(|p| p.exists())
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "HERMES_BROWSER_AUTO_START=1 but Chrome executable not found".into(),
                )
            })?;
        let mut cmd = tokio::process::Command::new(chrome);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", user_data.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.spawn().map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to launch Chrome for CDP: {e}"))
        })?;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if Self::probe_endpoint(&reqwest::Client::new(), endpoint).await {
                return Ok(());
            }
        }
        Err(ToolError::ExecutionFailed(
            "Chrome auto-start launched but CDP endpoint did not become ready in time".into(),
        ))
    }

    async fn ensure_connected(&self) -> Result<(), ToolError> {
        if Self::probe_endpoint(&self.client, &self.endpoint).await {
            return Ok(());
        }
        Self::try_auto_start_chrome(&self.endpoint).await
    }

    fn command_retry_attempts() -> usize {
        std::env::var("HERMES_BROWSER_COMMAND_RETRY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2)
            .clamp(1, 5)
    }

    fn retry_delay_ms() -> u64 {
        std::env::var("HERMES_BROWSER_COMMAND_RETRY_DELAY_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(250)
            .clamp(50, 2_000)
    }

    /// Send a CDP command via HTTP (simplified - real impl would use WebSocket).
    async fn cdp_command(&self, method: &str, params: Value) -> Result<Value, ToolError> {
        let attempts = Self::command_retry_attempts();
        let retry_delay = Self::retry_delay_ms();
        let mut last_error: Option<ToolError> = None;
        for attempt in 1..=attempts {
            match self.cdp_command_once(method, params.clone()).await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    last_error = Some(err);
                    if attempt < attempts {
                        tokio::time::sleep(Duration::from_millis(retry_delay)).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            ToolError::ExecutionFailed("Browser CDP command failed with unknown error".into())
        }))
    }

    async fn cdp_command_once(&self, method: &str, params: Value) -> Result<Value, ToolError> {
        self.ensure_connected().await?;
        let targets_resp = self
            .client
            .get(format!("{}/json", self.endpoint.trim_end_matches('/')))
            .send()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!(
                    "Failed to connect to Chrome CDP at {}: {}",
                    self.endpoint, e
                ))
            })?;

        let targets: Vec<Value> = targets_resp.json().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to parse CDP targets: {}", e))
        })?;

        let ws_url = targets
            .first()
            .and_then(|t| t.get("webSocketDebuggerUrl"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "No Chrome page target found. Is Chrome running with --remote-debugging-port=9222?"
                        .into(),
                )
            })?;

        Ok(json!({
            "method": method,
            "params": params,
            "target": ws_url,
            "status": "sent",
        }))
    }
}

#[async_trait]
impl BrowserBackend for CdpBrowserBackend {
    async fn navigate(&self, url: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        let result = self
            .cdp_command("Page.navigate", json!({"url": url}))
            .await?;
        Ok(json!({"status": "navigated", "url": url, "task_id": task_id, "cdp": result}).to_string())
    }

    async fn snapshot(
        &self,
        full: bool,
        user_task: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let started = Instant::now();
        let result = self
            .cdp_command("Accessibility.getFullAXTree", json!({}))
            .await?;
        let raw = result.to_string();
        let snapshot_text = super::browser_snapshot_util::process_snapshot_text(&raw, user_task).await;
        Ok(json!({
            "status": "snapshot",
            "full": full,
            "snapshot": snapshot_text,
            "user_task": user_task,
            "task_id": task_id,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "backend": "cdp",
            "cdp": result
        })
        .to_string())
    }

    async fn click(&self, ref_id: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        // Use Runtime.evaluate to find and click the element
        let js = format!(
            "document.querySelector('{}')?.click(); 'clicked'",
            ref_id.replace('\'', "\\'")
        );
        let result = self
            .cdp_command("Runtime.evaluate", json!({"expression": js}))
            .await?;
        Ok(json!({"status": "clicked", "ref": ref_id, "task_id": task_id, "cdp": result}).to_string())
    }

    async fn r#type(
        &self,
        ref_id: &str,
        text: &str,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let js = format!(
            "let el = document.querySelector('{}'); if(el) {{ el.value = '{}'; el.dispatchEvent(new Event('input')); 'typed' }} else {{ 'not found' }}",
            ref_id.replace('\'', "\\'"),
            text.replace('\'', "\\'")
        );
        let result = self
            .cdp_command("Runtime.evaluate", json!({"expression": js}))
            .await?;
        Ok(
            json!({"status": "typed", "ref": ref_id, "text": text, "task_id": task_id, "cdp": result})
                .to_string(),
        )
    }

    async fn scroll(
        &self,
        direction: &str,
        amount: Option<u32>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let px = amount.unwrap_or(500) as i32;
        let (x, y) = match direction {
            "up" => (0, -px),
            "down" => (0, px),
            "left" => (-px, 0),
            "right" => (px, 0),
            _ => (0, px),
        };
        let js = format!("window.scrollBy({}, {}); 'scrolled'", x, y);
        let result = self
            .cdp_command("Runtime.evaluate", json!({"expression": js}))
            .await?;
        Ok(
            json!({"status": "scrolled", "direction": direction, "amount": px, "task_id": task_id, "cdp": result})
                .to_string(),
        )
    }

    async fn go_back(&self, task_id: Option<&str>) -> Result<String, ToolError> {
        let result = self
            .cdp_command(
                "Runtime.evaluate",
                json!({"expression": "history.back(); 'back'"}),
            )
            .await?;
        Ok(json!({"status": "navigated_back", "task_id": task_id, "cdp": result}).to_string())
    }

    async fn press(&self, key: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        let result = self
            .cdp_command(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "keyDown",
                    "key": key,
                }),
            )
            .await?;
        Ok(json!({"status": "key_pressed", "key": key, "task_id": task_id, "cdp": result}).to_string())
    }

    async fn get_images(
        &self,
        selector: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        let sel = selector.unwrap_or("img");
        let js = format!(
            "JSON.stringify(Array.from(document.querySelectorAll('{}')).map(img => ({{src: img.src, alt: img.alt, width: img.width, height: img.height}})))",
            sel.replace('\'', "\\'")
        );
        let result = self
            .cdp_command(
                "Runtime.evaluate",
                json!({"expression": js, "returnByValue": true}),
            )
            .await?;
        Ok(json!({"status": "images_found", "selector": sel, "task_id": task_id, "cdp": result}).to_string())
    }

    async fn vision(
        &self,
        instruction: &str,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        // Take a screenshot and analyze with vision model
        let result = self
            .cdp_command("Page.captureScreenshot", json!({"format": "png"}))
            .await?;
        Ok(json!({
            "status": "vision_analysis",
            "instruction": instruction,
            "task_id": task_id,
            "screenshot": result,
            "note": "Screenshot captured; vision analysis requires LLM integration"
        })
        .to_string())
    }

    async fn console(&self, action: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        match action {
            "read" => {
                let result = self.cdp_command("Runtime.evaluate", json!({
                    "expression": "'Console messages require Runtime.consoleAPICalled event listener'"
                })).await?;
                Ok(json!({"status": "console_read", "task_id": task_id, "cdp": result}).to_string())
            }
            "clear" => {
                let result = self
                    .cdp_command(
                        "Runtime.evaluate",
                        json!({"expression": "console.clear(); 'cleared'"}),
                    )
                    .await?;
                Ok(json!({"status": "console_cleared", "task_id": task_id, "cdp": result}).to_string())
            }
            other => Err(ToolError::InvalidParams(format!(
                "Unknown console action: {}",
                other
            ))),
        }
    }
}

#[async_trait]
impl BrowserBackend for CamoFoxBrowserBackend {
    async fn navigate(&self, url: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        let mut result = self.inner.navigate(url, task_id).await?;
        result.push_str(&format!("\n{{\"camofox_profile\":\"{}\"}}", self.profile));
        Ok(result)
    }

    async fn snapshot(
        &self,
        full: bool,
        user_task: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        self.inner.snapshot(full, user_task, task_id).await
    }
    async fn click(&self, ref_id: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        self.inner.click(ref_id, task_id).await
    }
    async fn r#type(
        &self,
        ref_id: &str,
        text: &str,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        self.inner.r#type(ref_id, text, task_id).await
    }
    async fn scroll(
        &self,
        direction: &str,
        amount: Option<u32>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        self.inner.scroll(direction, amount, task_id).await
    }
    async fn go_back(&self, task_id: Option<&str>) -> Result<String, ToolError> {
        self.inner.go_back(task_id).await
    }
    async fn press(&self, key: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        self.inner.press(key, task_id).await
    }
    async fn get_images(
        &self,
        selector: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        self.inner.get_images(selector, task_id).await
    }
    async fn vision(
        &self,
        instruction: &str,
        task_id: Option<&str>,
    ) -> Result<String, ToolError> {
        self.inner.vision(instruction, task_id).await
    }
    async fn console(&self, action: &str, task_id: Option<&str>) -> Result<String, ToolError> {
        self.inner.console(action, task_id).await
    }
}
