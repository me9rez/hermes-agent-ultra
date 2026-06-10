use hermes_core::errors::AgentError;

/// 当前运行平台信息
pub struct Platform {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Platform {
    /// 检测当前平台
    pub fn detect() -> Result<Self, AgentError> {
        let os = match std::env::consts::OS {
            "linux" => "linux",
            "windows" => "windows",
            "macos" => "macos",
            other => return Err(AgentError::Io(format!("Unsupported OS: {other}"))),
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            other => return Err(AgentError::Io(format!("Unsupported architecture: {other}"))),
        };
        Ok(Self { os, arch })
    }

    /// 对应的 release artifact 文件名
    pub fn artifact_name(&self) -> String {
        match (self.os, self.arch) {
            ("linux", "x86_64") => "hermes-linux-x86_64.tar.gz".to_string(),
            ("linux", "aarch64") => "hermes-linux-aarch64.tar.gz".to_string(),
            ("windows", "x86_64") => "hermes-windows-x86_64.zip".to_string(),
            ("macos", "aarch64") => "hermes-macos-aarch64.tar.gz".to_string(),
            ("macos", "x86_64") => "hermes-macos-x86_64.tar.gz".to_string(),
            _ => format!("hermes-{}-{}.tar.gz", self.os, self.arch),
        }
    }

    /// binary 文件名
    pub fn binary_name(&self) -> &'static str {
        if self.os == "windows" {
            "hermes.exe"
        } else {
            "hermes"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_returns_valid_platform() {
        let platform = Platform::detect().unwrap();
        assert!(["linux", "windows", "macos"].contains(&platform.os));
        assert!(["x86_64", "aarch64"].contains(&platform.arch));
    }

    #[test]
    fn test_artifact_name_linux_x86_64() {
        let p = Platform {
            os: "linux",
            arch: "x86_64",
        };
        assert_eq!(p.artifact_name(), "hermes-linux-x86_64.tar.gz");
    }

    #[test]
    fn test_artifact_name_linux_aarch64() {
        let p = Platform {
            os: "linux",
            arch: "aarch64",
        };
        assert_eq!(p.artifact_name(), "hermes-linux-aarch64.tar.gz");
    }

    #[test]
    fn test_artifact_name_windows() {
        let p = Platform {
            os: "windows",
            arch: "x86_64",
        };
        assert_eq!(p.artifact_name(), "hermes-windows-x86_64.zip");
    }

    #[test]
    fn test_artifact_name_macos_aarch64() {
        let p = Platform {
            os: "macos",
            arch: "aarch64",
        };
        assert_eq!(p.artifact_name(), "hermes-macos-aarch64.tar.gz");
    }

    #[test]
    fn test_artifact_name_macos_x86_64() {
        let p = Platform {
            os: "macos",
            arch: "x86_64",
        };
        assert_eq!(p.artifact_name(), "hermes-macos-x86_64.tar.gz");
    }

    #[test]
    fn test_binary_name_windows() {
        let p = Platform {
            os: "windows",
            arch: "x86_64",
        };
        assert_eq!(p.binary_name(), "hermes.exe");
    }

    #[test]
    fn test_binary_name_unix() {
        let p = Platform {
            os: "linux",
            arch: "x86_64",
        };
        assert_eq!(p.binary_name(), "hermes");
        let p = Platform {
            os: "macos",
            arch: "aarch64",
        };
        assert_eq!(p.binary_name(), "hermes");
    }
}
