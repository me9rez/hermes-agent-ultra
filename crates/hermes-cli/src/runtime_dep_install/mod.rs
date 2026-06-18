//! Silent runtime dependency installation (ffmpeg and future deps).

mod ffmpeg;
mod probe;

use hermes_config::dep_check::{RuntimeDep, is_available};
use tracing::{debug, info, warn};

pub use ffmpeg::ensure_ffmpeg;

const AUTO_ENSURE_ENV: &str = "HERMES_AUTO_ENSURE_DEPS";

/// Whether gateway/CLI should attempt silent dependency installation.
pub fn auto_ensure_enabled() -> bool {
    std::env::var(AUTO_ENSURE_ENV)
        .ok()
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

/// Install a single runtime dependency when missing (`quiet` suppresses stdout).
pub async fn ensure_runtime_dep(dep: RuntimeDep, quiet: bool) -> bool {
    if is_available(dep) {
        debug!(%dep, "runtime dependency already available");
        return true;
    }

    let ok = match dep {
        RuntimeDep::Ffmpeg => ensure_ffmpeg(quiet).await.is_ok(),
        RuntimeDep::Node | RuntimeDep::Browser | RuntimeDep::Ripgrep => {
            if !quiet {
                eprintln!(
                    "Automatic install for {dep} is not implemented yet; run `hermes gateway setup`."
                );
            } else {
                warn!(
                    %dep,
                    "automatic install not implemented; run `hermes gateway setup`"
                );
            }
            false
        }
    };

    if ok && is_available(dep) {
        if !quiet {
            info!(%dep, "runtime dependency installed");
        }
        true
    } else {
        false
    }
}

/// Ensure all missing deps when [`auto_ensure_enabled`] is true.
pub async fn ensure_missing_runtime_deps(
    deps: &[RuntimeDep],
    quiet: bool,
) -> Vec<(RuntimeDep, bool)> {
    let mut results = Vec::new();
    for &dep in deps {
        if is_available(dep) {
            results.push((dep, true));
            continue;
        }
        if !auto_ensure_enabled() {
            debug!(%dep, "HERMES_AUTO_ENSURE_DEPS disabled; skipping auto install");
            results.push((dep, false));
            continue;
        }
        let ok = ensure_runtime_dep(dep, quiet).await;
        results.push((dep, ok));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_ensure_defaults_on() {
        let prior = std::env::var(AUTO_ENSURE_ENV).ok();
        unsafe { std::env::remove_var(AUTO_ENSURE_ENV) };
        assert!(auto_ensure_enabled());
        unsafe { std::env::remove_var(AUTO_ENSURE_ENV) };
        if let Some(v) = prior {
            unsafe { std::env::set_var(AUTO_ENSURE_ENV, v) };
        }
    }
}
