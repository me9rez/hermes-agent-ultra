use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::actions::{BrowserAction, BrowserObservation};
use crate::approval::ApprovalMode;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub struct BrowserDriver {
    binary_path: PathBuf,
    approval_mode: ApprovalMode,
}

impl BrowserDriver {
    pub fn new(binary_path: PathBuf, approval_mode: ApprovalMode) -> Self {
        Self {
            binary_path,
            approval_mode,
        }
    }

    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    pub fn approval_mode(&self) -> ApprovalMode {
        self.approval_mode
    }

    pub async fn execute(&self, action: BrowserAction) -> Result<BrowserObservation, BrowserError> {
        let _ = action;
        Err(BrowserError::Other(
            "browser driver not yet connected to CDP".into(),
        ))
    }
}

pub fn detect_system_browser() -> Result<PathBuf, BrowserError> {
    if let Ok(path) = std::env::var("BROWSER") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
    }
    for candidate in default_browser_candidates() {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(BrowserError::Other(
        "no system Chrome/Edge/Brave browser found".into(),
    ))
}

fn default_browser_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        vec![
            PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"),
            PathBuf::from(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"),
        ]
    }
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
            PathBuf::from("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"),
        ]
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        vec![
            PathBuf::from("/usr/bin/google-chrome"),
            PathBuf::from("/usr/bin/chromium"),
        ]
    }
}
