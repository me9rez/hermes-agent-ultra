//! Interactive dependency install orchestration
//! mirrors python `hermes_cli/dep_ensure.py`.
//!
//! Uses [`hermes_config::dep_check`] for availability detection. FFmpeg installs
//! run in-process via [`crate::runtime_dep_install`]; other deps may delegate to
//! `scripts/install.ps1` / `scripts/install.sh` when interactive.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use hermes_config::dep_check::{RuntimeDep, description, is_available};
use hermes_config::hermes_home;
use tokio::process::Command;
use tracing::{debug, warn};

use crate::runtime_dep_install::{auto_ensure_enabled, ensure_runtime_dep};

/// Shell type used to invoke the install script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    PowerShell,
    Bash,
}

/// Parse a runtime dependency name (`ffmpeg`, `node`, ...).
pub fn parse_runtime_dep_name(name: &str) -> Option<RuntimeDep> {
    match name.trim().to_ascii_lowercase().as_str() {
        "node" => Some(RuntimeDep::Node),
        "browser" => Some(RuntimeDep::Browser),
        "ripgrep" | "rg" => Some(RuntimeDep::Ripgrep),
        "ffmpeg" => Some(RuntimeDep::Ffmpeg),
        _ => None,
    }
}

/// Locate the install script (`install.ps1` or `install.sh`).
pub fn find_install_script() -> Option<(PathBuf, ShellKind)> {
    let (preferred, preferred_kind, fallback, fallback_kind): (&str, ShellKind, &str, ShellKind) =
        if cfg!(windows) {
            (
                "install.ps1",
                ShellKind::PowerShell,
                "install.sh",
                ShellKind::Bash,
            )
        } else {
            (
                "install.sh",
                ShellKind::Bash,
                "install.ps1",
                ShellKind::PowerShell,
            )
        };

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(PathBuf::from);
        while let Some(d) = dir {
            let candidate = d.join("scripts").join(preferred);
            if candidate.is_file() {
                return Some((candidate, preferred_kind));
            }
            let candidate = d.join("scripts").join(fallback);
            if candidate.is_file() {
                return Some((candidate, fallback_kind));
            }
            dir = d.parent().map(PathBuf::from);
        }
    }

    let cwd = std::env::current_dir().ok()?;
    let candidate = cwd.join("scripts").join(preferred);
    if candidate.is_file() {
        return Some((candidate, preferred_kind));
    }
    let candidate = cwd.join("scripts").join(fallback);
    if candidate.is_file() {
        return Some((candidate, fallback_kind));
    }

    None
}

fn prompt_yes_no(prompt: &str) -> bool {
    print!("{prompt} [Y/n] ");
    let _ = io::stdout().flush();
    let stdin = io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_err() {
        return false;
    }
    let answer = line.trim().to_lowercase();
    answer.is_empty() || answer == "y" || answer == "yes"
}

/// Ensure a runtime dependency is available, optionally prompting for install.
pub async fn ensure_dependency(dep: RuntimeDep, interactive: bool) -> bool {
    if is_available(dep) {
        debug!(%dep, "dependency already available");
        return true;
    }

    if dep == RuntimeDep::Ffmpeg {
        if interactive {
            if !atty_is_tty() {
                warn!("not a TTY, skipping install prompt for {}", dep);
                return false;
            }
            if !prompt_yes_no(&format!(
                "{} is not installed. Install now?",
                description(dep)
            )) {
                return false;
            }
            return ensure_runtime_dep(dep, false).await;
        }
        if auto_ensure_enabled() {
            return ensure_runtime_dep(dep, true).await;
        }
        warn!(%dep, "{} is not installed", description(dep));
        return false;
    }

    if !interactive {
        if auto_ensure_enabled() {
            return ensure_runtime_dep(dep, true).await;
        }
        warn!(%dep, "{} is not installed", description(dep));
        return false;
    }

    let (script, shell) = match find_install_script() {
        Some(pair) => pair,
        None => {
            println!(
                "{} is not installed and no install script was found.",
                description(dep)
            );
            return false;
        }
    };

    if !atty_is_tty() {
        warn!("not a TTY, skipping install prompt for {}", dep);
        return false;
    }
    if !prompt_yes_no(&format!(
        "{} is not installed. Install now?",
        description(dep)
    )) {
        return false;
    }

    let dep_name = dep.to_string();
    let home = hermes_home();
    let mut cmd = match shell {
        ShellKind::PowerShell => {
            let ps = which::which("powershell")
                .or_else(|_| which::which("pwsh"))
                .ok();
            match ps {
                Some(ps_path) => {
                    let mut c = Command::new(ps_path);
                    c.arg("-ExecutionPolicy")
                        .arg("Bypass")
                        .arg("-File")
                        .arg(&script)
                        .arg("-Ensure")
                        .arg(&dep_name)
                        .arg("-HermesHome")
                        .arg(home);
                    c
                }
                None => {
                    println!("PowerShell not found; cannot run install script.");
                    return false;
                }
            }
        }
        ShellKind::Bash => {
            let mut c = Command::new("bash");
            c.arg(&script).arg("--ensure").arg(&dep_name);
            c
        }
    };

    cmd.env("IS_INTERACTIVE", "false");

    debug!(%dep, "running install script");
    match cmd.status().await {
        Ok(status) if status.success() => is_available(dep),
        Ok(status) => {
            warn!(%dep, code = ?status.code(), "install script failed");
            false
        }
        Err(e) => {
            warn!(%dep, "failed to run install script: {e}");
            false
        }
    }
}

/// Batch ensure: iterate over multiple deps, returning per-dep results.
pub async fn ensure_all(deps: &[RuntimeDep], interactive: bool) -> Vec<(RuntimeDep, bool)> {
    let mut results = Vec::with_capacity(deps.len());
    for &dep in deps {
        let ok = ensure_dependency(dep, interactive).await;
        results.push((dep, ok));
    }
    results
}

fn atty_is_tty() -> bool {
    if cfg!(windows) {
        return true;
    }
    std::env::var("TERM").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_runtime_dep_names() {
        assert_eq!(parse_runtime_dep_name("ffmpeg"), Some(RuntimeDep::Ffmpeg));
        assert_eq!(parse_runtime_dep_name("rg"), Some(RuntimeDep::Ripgrep));
        assert_eq!(parse_runtime_dep_name("unknown"), None);
    }

    #[tokio::test]
    async fn ensure_returns_true_when_available() {
        if is_available(RuntimeDep::Node) {
            assert!(ensure_dependency(RuntimeDep::Node, false).await);
        }
    }
}
