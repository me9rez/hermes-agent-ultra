use crate::update::github::{ReleaseInfo, ReleaseSource};
use crate::update::manifest::ReleaseManifest;
use crate::update::platform::Platform;
use crate::update::version::Channel;
use async_trait::async_trait;
use hermes_core::errors::AgentError;
use semver::Version;
use std::process::Command;

/// ModelScope Release 源
pub struct ModelScopeSource {
    pub repo: String,   // "flowy2025/agent"
    pub prefix: String, // "hermes-agent-ultra"
}

impl Default for ModelScopeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelScopeSource {
    pub fn new() -> Self {
        let repo = std::env::var("HERMES_MODELSCOPE_REPO")
            .unwrap_or_else(|_| "flowy2025/agent".to_string());
        Self {
            repo,
            prefix: "hermes-agent-ultra".to_string(),
        }
    }

    /// URL to fetch latest.json
    fn latest_json_url(&self) -> String {
        format!(
            "https://modelscope.cn/api/v1/models/{}/repo?Revision=master&FilePath={}/latest.json",
            self.repo, self.prefix
        )
    }

    /// URL to download a specific file
    fn file_download_url(&self, version_tag: &str, filename: &str) -> String {
        format!(
            "https://modelscope.cn/api/v1/models/{}/repo?Revision=master&FilePath={}/{}/{}",
            self.repo, self.prefix, version_tag, filename
        )
    }
}

/// Use system curl (schannel on Windows) to fetch content from ModelScope.
/// Unlike the github variant, this does NOT send a GITHUB_TOKEN header.
fn curl_get(url: &str) -> Result<String, AgentError> {
    let mut cmd = Command::new("curl");
    cmd.args(["-sSfL", "-H", "User-Agent: hermes-agent-ultra"]);
    cmd.arg(url);

    let output = cmd
        .output()
        .map_err(|e| AgentError::Io(format!("Failed to run curl: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AgentError::Io(format!("curl failed: {stderr}")));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| AgentError::Io(format!("Invalid UTF-8 in response: {e}")))
}

#[async_trait]
impl ReleaseSource for ModelScopeSource {
    fn name(&self) -> &str {
        "ModelScope"
    }

    async fn fetch_latest(&self, platform: &Platform) -> Result<ReleaseInfo, AgentError> {
        let body = curl_get(&self.latest_json_url()).map_err(|e| {
            AgentError::Io(format!("Failed to fetch latest.json from ModelScope: {e}"))
        })?;

        let manifest: ReleaseManifest = serde_json::from_str(&body)
            .map_err(|e| AgentError::Io(format!("Failed to parse manifest: {e}")))?;

        let version = Version::parse(&manifest.version).unwrap_or_else(|_| Version::new(0, 0, 0));
        let channel = Channel::from_str(&manifest.channel);

        // 获取 artifact URL
        let platform_key = format!("{}-{}", platform.os, platform.arch);
        let artifact_url = if let Some(plat) = manifest.get_platform(&platform_key) {
            plat.url.clone()
        } else {
            // Fallback: 旧格式用 artifacts 列表
            let artifact_name = platform.artifact_name();
            if manifest.artifacts.contains(&artifact_name) {
                self.file_download_url(&manifest.version_tag(), &artifact_name)
            } else {
                return Err(AgentError::Io(format!(
                    "No artifact for platform {} in manifest",
                    platform_key
                )));
            }
        };

        // Checksum URL
        let checksum_url = if manifest.has_platform_info() {
            // 新格式: sha256 已内联，不需要额外下载
            None
        } else {
            // 旧格式: 下载 checksums.sha256
            Some(self.file_download_url(&manifest.version_tag(), "checksums.sha256"))
        };

        Ok(ReleaseInfo {
            version,
            tag: manifest.version_tag(),
            channel,
            artifact_url,
            checksum_url,
            release_notes: manifest.notes.clone(),
            forced: manifest.forced,
            min_version: manifest
                .min_version
                .as_ref()
                .and_then(|v| Version::parse(v).ok()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modelscope_source_default_repo() {
        // SAFETY: test-only env manipulation; tests run single-threaded for env vars
        unsafe { std::env::remove_var("HERMES_MODELSCOPE_REPO") };
        let source = ModelScopeSource::new();
        assert_eq!(source.repo, "flowy2025/agent");
        assert_eq!(source.prefix, "hermes-agent-ultra");
    }

    #[test]
    fn test_modelscope_source_custom_repo() {
        // SAFETY: test-only env manipulation; tests run single-threaded for env vars
        unsafe { std::env::set_var("HERMES_MODELSCOPE_REPO", "myorg/myrepo") };
        let source = ModelScopeSource::new();
        assert_eq!(source.repo, "myorg/myrepo");
        assert_eq!(source.prefix, "hermes-agent-ultra");
        // Cleanup
        unsafe { std::env::remove_var("HERMES_MODELSCOPE_REPO") };
    }

    #[test]
    fn test_latest_json_url() {
        let source = ModelScopeSource {
            repo: "flowy2025/agent".to_string(),
            prefix: "hermes-agent-ultra".to_string(),
        };
        assert_eq!(
            source.latest_json_url(),
            "https://modelscope.cn/api/v1/models/flowy2025/agent/repo?Revision=master&FilePath=hermes-agent-ultra/latest.json"
        );
    }

    #[test]
    fn test_file_download_url() {
        let source = ModelScopeSource {
            repo: "flowy2025/agent".to_string(),
            prefix: "hermes-agent-ultra".to_string(),
        };
        assert_eq!(
            source.file_download_url("v0.1.0", "hermes-linux-x86_64.tar.gz"),
            "https://modelscope.cn/api/v1/models/flowy2025/agent/repo?Revision=master&FilePath=hermes-agent-ultra/v0.1.0/hermes-linux-x86_64.tar.gz"
        );
    }
}
