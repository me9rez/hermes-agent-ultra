//! Shared sherpa-onnx runtime settings (ONNX Runtime execution provider).
//!
//! Provider names are validated with **target `cfg`** (platform capability).
//! Which ONNX Runtime build is linked is selected at compile time via the
//! `SHERPA_ONNX_PACK` env var (set by `make release-talk-*` / `package-talk-*`).

use crate::error::{DemoError, Result};

/// Execution providers available on the **current compile target**.
#[cfg(target_os = "windows")]
pub const PLATFORM_PROVIDERS: &[&str] = &["cpu", "cuda", "directml"];

#[cfg(target_os = "macos")]
pub const PLATFORM_PROVIDERS: &[&str] = &["cpu", "coreml"];

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const PLATFORM_PROVIDERS: &[&str] = &["cpu", "cuda"];

#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", target_arch = "x86_64")
)))]
pub const PLATFORM_PROVIDERS: &[&str] = &["cpu"];

pub fn platform_supports(provider: &str) -> bool {
    PLATFORM_PROVIDERS.contains(&provider)
}

pub fn validate_provider(provider: &str) -> Result<()> {
    if platform_supports(provider) {
        Ok(())
    } else {
        Err(DemoError::Config(format!(
            "invalid sherpa provider '{provider}' on this platform (expected one of: {})",
            PLATFORM_PROVIDERS.join(", ")
        )))
    }
}

pub fn provider_hint(provider: &str) -> Option<&'static str> {
    match provider {
        "cuda" => Some(
            "use SHERPA_ONNX_PACK=cuda (or `make release-talk` on Windows/Linux x64); \
             requires CUDA 12.x + cuDNN 9 at runtime",
        ),
        "directml" => Some(
            "Windows only: build sherpa-onnx with DirectML, set SHERPA_ONNX_LIB_DIR, \
             SHERPA_ONNX_PACK=directml",
        ),
        "coreml" => {
            Some("macOS only: use SHERPA_ONNX_PACK=macos (or `make release-talk` on macOS)")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_always_supported() {
        validate_provider("cpu").unwrap();
    }

    #[test]
    fn rejects_gpu_alias() {
        assert!(validate_provider("gpu").is_err());
    }

    #[test]
    fn platform_list_includes_cpu() {
        assert!(PLATFORM_PROVIDERS.contains(&"cpu"));
    }
}
