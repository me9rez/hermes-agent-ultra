//! Remote LLM server configuration (auth + OpenAI-compatible inference gateway).

use serde::{Deserialize, Serialize};

/// Supported remote server login methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ServerLoginMethod {
    #[default]
    WechatQr,
    EmailOtp,
}

impl ServerLoginMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WechatQr => "wechat_qr",
            Self::EmailOtp => "email_otp",
        }
    }
}

/// Top-level remote server settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// When true, Hermes uses the remote server for LLM calls (after login).
    #[serde(default)]
    pub enabled: bool,

    /// Server API origin, e.g. `https://api.example.com`.
    #[serde(default)]
    pub base_url: String,

    #[serde(default)]
    pub auth: ServerAuthConfig,

    #[serde(default)]
    pub llm: ServerLlmConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            auth: ServerAuthConfig::default(),
            llm: ServerLlmConfig::default(),
        }
    }
}

/// Login settings for the remote server account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerAuthConfig {
    /// Default login method when `hermes server login` is invoked without `--method`.
    #[serde(default)]
    pub preferred_method: ServerLoginMethod,

    /// WeChat QR scan poll interval in milliseconds.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,

    /// Hint for email OTP validity duration (seconds).
    #[serde(default = "default_otp_ttl_seconds")]
    pub otp_ttl_seconds: u64,
}

impl Default for ServerAuthConfig {
    fn default() -> Self {
        Self {
            preferred_method: ServerLoginMethod::default(),
            poll_interval_ms: default_poll_interval_ms(),
            otp_ttl_seconds: default_otp_ttl_seconds(),
        }
    }
}

/// LLM gateway path and timeout settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerLlmConfig {
    /// OpenAI-compatible path prefix, usually `/v1`.
    #[serde(default = "default_llm_path_prefix")]
    pub path_prefix: String,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_model: String,

    #[serde(default = "default_llm_request_timeout_seconds")]
    pub request_timeout_seconds: u64,
}

impl Default for ServerLlmConfig {
    fn default() -> Self {
        Self {
            path_prefix: default_llm_path_prefix(),
            default_model: String::new(),
            request_timeout_seconds: default_llm_request_timeout_seconds(),
        }
    }
}

fn default_poll_interval_ms() -> u64 {
    2000
}

fn default_otp_ttl_seconds() -> u64 {
    300
}

fn default_llm_path_prefix() -> String {
    "/v1".to_string()
}

fn default_llm_request_timeout_seconds() -> u64 {
    120
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_config_defaults_off() {
        let cfg = ServerConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.base_url.is_empty());
        assert_eq!(cfg.auth.preferred_method, ServerLoginMethod::WechatQr);
        assert_eq!(cfg.llm.path_prefix, "/v1");
    }

    #[test]
    fn server_config_yaml_roundtrip() {
        let yaml = r#"
enabled: true
base_url: https://llm.example.com
auth:
  preferred_method: email_otp
  poll_interval_ms: 1500
llm:
  path_prefix: /v1
  default_model: gpt-4o
  request_timeout_seconds: 90
"#;
        let cfg: ServerConfig = serde_yaml::from_str(yaml).expect("parse");
        assert!(cfg.enabled);
        assert_eq!(cfg.base_url, "https://llm.example.com");
        assert_eq!(cfg.auth.preferred_method, ServerLoginMethod::EmailOtp);
        assert_eq!(cfg.llm.default_model, "gpt-4o");
    }
}
