//! WhatsApp wa-rs client configuration.

use std::path::PathBuf;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::AdapterProxyConfig;
use hermes_config::PlatformConfig;

pub const DEFAULT_BRIDGE_PORT: u16 = 3000;
pub const MAX_MESSAGE_LENGTH: usize = 4096;
pub const DEFAULT_REPLY_PREFIX: &str = "────────────\n";
pub const DEFAULT_TEXT_BATCH_DELAY_SECS: f64 = 5.0;
pub const DEFAULT_TEXT_BATCH_SPLIT_DELAY_SECS: f64 = 10.0;
pub const TEXT_BATCH_SPLIT_THRESHOLD: usize = 6000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub bridge_port: u16,
    #[serde(default)]
    pub bridge_script: Option<String>,
    #[serde(default)]
    pub session_path: Option<String>,
    #[serde(default)]
    pub reply_prefix: Option<String>,
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
    #[serde(default)]
    pub allow_from: Vec<String>,
    #[serde(default = "default_group_policy")]
    pub group_policy: String,
    #[serde(default)]
    pub group_allow_from: Vec<String>,
    #[serde(default)]
    pub require_mention: Option<bool>,
    #[serde(default)]
    pub mention_patterns: Vec<String>,
    #[serde(default)]
    pub free_response_chats: Vec<String>,
    #[serde(default)]
    pub text_batch_delay_seconds: f64,
    #[serde(default)]
    pub text_batch_split_delay_seconds: f64,
    #[serde(default)]
    pub proxy: AdapterProxyConfig,
    /// `self-chat` or `bot`, persisted from gateway setup / wizard.
    #[serde(default)]
    pub mode: Option<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_dm_policy() -> String {
    "open".into()
}

fn default_group_policy() -> String {
    "open".into()
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bridge_port: DEFAULT_BRIDGE_PORT,
            bridge_script: None,
            session_path: None,
            reply_prefix: None,
            dm_policy: default_dm_policy(),
            allow_from: Vec::new(),
            group_policy: default_group_policy(),
            group_allow_from: Vec::new(),
            require_mention: None,
            mention_patterns: Vec::new(),
            free_response_chats: Vec::new(),
            text_batch_delay_seconds: DEFAULT_TEXT_BATCH_DELAY_SECS,
            text_batch_split_delay_seconds: DEFAULT_TEXT_BATCH_SPLIT_DELAY_SECS,
            proxy: AdapterProxyConfig::default(),
            mode: None,
        }
    }
}

impl WhatsAppConfig {
    pub fn from_platform_config(p: &PlatformConfig) -> Self {
        let ex = &p.extra;
        let mut cfg = Self::default();
        cfg.enabled = p.enabled;
        cfg.bridge_port = extra_u16(ex, "bridge_port", DEFAULT_BRIDGE_PORT);
        cfg.bridge_script = extra_string(ex, "bridge_script");
        cfg.session_path = extra_string(ex, "session_path");
        cfg.reply_prefix = ex
            .get("reply_prefix")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| extra_string(ex, "reply_prefix"));
        if let Some(v) = ex.get("dm_policy").and_then(|v| v.as_str()) {
            cfg.dm_policy = v.trim().to_lowercase();
        } else if let Ok(v) = std::env::var("WHATSAPP_DM_POLICY") {
            cfg.dm_policy = v.trim().to_lowercase();
        }
        cfg.allow_from = extra_string_list(ex, "allow_from")
            .or_else(|| env_list("WHATSAPP_ALLOWED_USERS"))
            .unwrap_or_default();
        if let Some(v) = ex.get("group_policy").and_then(|v| v.as_str()) {
            cfg.group_policy = v.trim().to_lowercase();
        } else if let Ok(v) = std::env::var("WHATSAPP_GROUP_POLICY") {
            cfg.group_policy = v.trim().to_lowercase();
        }
        cfg.group_allow_from = extra_string_list(ex, "group_allow_from")
            .or_else(|| env_list("WHATSAPP_GROUP_ALLOW_FROM"))
            .unwrap_or_default();
        cfg.require_mention =
            extra_bool(ex, "require_mention").or_else(|| env_bool("WHATSAPP_REQUIRE_MENTION"));
        cfg.mention_patterns = extra_string_list(ex, "mention_patterns")
            .or_else(|| env_list("WHATSAPP_MENTION_PATTERNS"))
            .unwrap_or_default();
        cfg.free_response_chats = extra_string_list(ex, "free_response_chats")
            .or_else(|| env_list("WHATSAPP_FREE_RESPONSE_CHATS"))
            .unwrap_or_default();
        cfg.text_batch_delay_seconds = extra_f64(
            ex,
            "text_batch_delay_seconds",
            DEFAULT_TEXT_BATCH_DELAY_SECS,
        );
        cfg.text_batch_split_delay_seconds = extra_f64(
            ex,
            "text_batch_split_delay_seconds",
            DEFAULT_TEXT_BATCH_SPLIT_DELAY_SECS,
        );
        cfg.mode = extra_string(ex, "mode");
        if cfg.whatsapp_mode() == "self-chat" && ex.get("text_batch_delay_seconds").is_none() {
            cfg.text_batch_delay_seconds = 1.0;
        }
        cfg
    }

    pub fn default_bridge_script() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("scripts")
            .join("whatsapp-bridge")
            .join("bridge.js")
    }

    pub fn bridge_script_path(&self) -> PathBuf {
        self.bridge_script
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(Self::default_bridge_script)
    }

    pub fn session_path(&self) -> PathBuf {
        self.session_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                hermes_config::hermes_home()
                    .join("whatsapp")
                    .join("session")
            })
    }

    pub fn whatsapp_mode(&self) -> String {
        if let Some(ref mode) = self.mode {
            let m = mode.trim().to_lowercase();
            if !m.is_empty() {
                return m;
            }
        }
        std::env::var("WHATSAPP_MODE")
            .unwrap_or_else(|_| "self-chat".into())
            .trim()
            .to_lowercase()
    }

    /// Self-chat talks to the owner's own DM; gateway DM pairing gate must be open.
    pub fn self_chat_dm_policy_open(&self) -> bool {
        self.whatsapp_mode() == "self-chat"
    }

    pub fn effective_reply_prefix(&self) -> String {
        if self.whatsapp_mode() != "self-chat" {
            return String::new();
        }
        if let Some(ref prefix) = self.reply_prefix {
            return prefix.replace("\\n", "\n");
        }
        if let Ok(prefix) = std::env::var("WHATSAPP_REPLY_PREFIX") {
            return prefix.replace("\\n", "\n");
        }
        DEFAULT_REPLY_PREFIX.to_string()
    }

    pub fn outgoing_chunk_limit(&self) -> usize {
        let prefix_len = self.effective_reply_prefix().len();
        MAX_MESSAGE_LENGTH.saturating_sub(prefix_len).max(1024)
    }
}

fn extra_string(ex: &HashMap<String, Value>, key: &str) -> Option<String> {
    ex.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn extra_u16(ex: &HashMap<String, Value>, key: &str, default: u16) -> u16 {
    ex.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok())
        .unwrap_or(default)
}

fn extra_f64(ex: &HashMap<String, Value>, key: &str, default: f64) -> f64 {
    let value = ex.get(key).cloned();
    let parsed = match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    };
    match parsed {
        Some(v) if v.is_finite() && v >= 0.0 => v,
        _ => default,
    }
}

fn extra_bool(ex: &HashMap<String, Value>, key: &str) -> Option<bool> {
    match ex.get(key) {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => Some(matches!(
            s.to_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        )),
        _ => None,
    }
}

fn extra_string_list(ex: &HashMap<String, Value>, key: &str) -> Option<Vec<String>> {
    match ex.get(key) {
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
        ),
        Some(Value::String(raw)) => {
            let items: Vec<String> = raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if items.is_empty() { None } else { Some(items) }
        }
        _ => None,
    }
}

fn env_list(key: &str) -> Option<Vec<String>> {
    std::env::var(key).ok().map(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    })
}

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "true" | "1" | "yes" | "on"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermes_config::PlatformConfig;

    #[test]
    fn text_batch_defaults() {
        let cfg = WhatsAppConfig::default();
        assert_eq!(cfg.text_batch_delay_seconds, 5.0);
        assert_eq!(cfg.text_batch_split_delay_seconds, 10.0);
    }

    #[test]
    fn invalid_batch_delay_falls_back() {
        let mut p = PlatformConfig::default();
        p.extra.insert(
            "text_batch_delay_seconds".into(),
            Value::String("garbage".into()),
        );
        p.extra.insert(
            "text_batch_split_delay_seconds".into(),
            Value::Number((-3).into()),
        );
        let cfg = WhatsAppConfig::from_platform_config(&p);
        assert_eq!(cfg.text_batch_delay_seconds, 5.0);
        assert_eq!(cfg.text_batch_split_delay_seconds, 10.0);
    }

    #[test]
    fn reply_prefix_empty_disables() {
        let mut p = PlatformConfig::default();
        p.extra
            .insert("reply_prefix".into(), Value::String("".into()));
        let cfg = WhatsAppConfig::from_platform_config(&p);
        assert_eq!(cfg.reply_prefix.as_deref(), Some(""));
    }

    #[test]
    fn self_chat_requests_open_dm_policy() {
        let mut p = PlatformConfig::default();
        p.extra
            .insert("mode".into(), Value::String("self-chat".into()));
        let cfg = WhatsAppConfig::from_platform_config(&p);
        assert!(cfg.self_chat_dm_policy_open());
    }
}
