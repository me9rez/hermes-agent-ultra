use std::process::{Child, Stdio};
use std::time::Duration;

use tauri::AppHandle;

use super::DEFAULT_HERMES_HTTP_PORT;
use super::probe::probe_status;
use super::resolve::resolve_hermes_http_bin;

pub struct EnsureResult {
    pub base_url: String,
    pub child: Option<Child>,
}

pub async fn ensure_hermes_http_running(port: u16) -> Result<EnsureResult, String> {
    let base_url = format!("http://127.0.0.1:{port}");
    let probe = probe_status(Some(&base_url)).await;
    if probe.ok {
        return Ok(EnsureResult {
            base_url,
            child: None,
        });
    }

    let bin = resolve_hermes_http_bin().ok_or_else(|| {
        "hermes-http binary not found. Build with `cargo build -p hermes-http` or set HERMES_HTTP_BIN"
            .to_string()
    })?;

    let mut command = std::process::Command::new(&bin);
    command
        .env("HERMES_HTTP_ADDR", format!("127.0.0.1:{port}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let child = command
        .spawn()
        .map_err(|e| format!("failed to spawn hermes-http: {e}"))?;

    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if probe_status(Some(&base_url)).await.ok {
            return Ok(EnsureResult {
                base_url,
                child: Some(child),
            });
        }
    }

    Err(format!(
        "hermes-http failed to become ready at {base_url}. Check logs."
    ))
}

pub async fn ensure_on_demand(_app: &AppHandle, port: u16) -> Result<EnsureResult, String> {
    ensure_hermes_http_running(port).await
}

pub fn pick_port(preferred: u16) -> u16 {
    if std::net::TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return preferred;
    }
    for port in preferred..preferred.saturating_add(80) {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    DEFAULT_HERMES_HTTP_PORT
}
