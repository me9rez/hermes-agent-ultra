//! Discord `allowed_mentions` safe defaults (P2-2).

use hermes_config::PlatformConfig;
use serde_json::{json, Value};

/// What mention types the bot may generate in outbound messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordAllowedMentions {
    pub everyone: bool,
    pub roles: bool,
    pub users: bool,
    pub replied_user: bool,
}

impl Default for DiscordAllowedMentions {
    fn default() -> Self {
        Self {
            everyone: false,
            roles: false,
            users: true,
            replied_user: true,
        }
    }
}

impl DiscordAllowedMentions {
    /// Build JSON for Discord REST `allowed_mentions` field.
    pub fn to_api_value(&self) -> Value {
        let mut parse = Vec::new();
        if self.everyone {
            parse.push("everyone");
        }
        if self.roles {
            parse.push("roles");
        }
        if self.users {
            parse.push("users");
        }
        json!({
            "parse": parse,
            "replied_user": self.replied_user,
        })
    }

    pub fn from_platform(platform_cfg: &PlatformConfig) -> Self {
        let mut cfg = Self::default();
        if let Some(obj) = platform_cfg.extra.get("allow_mentions").and_then(|v| v.as_object())
        {
            if let Some(v) = obj.get("everyone").and_then(|v| v.as_bool()) {
                cfg.everyone = v;
            }
            if let Some(v) = obj.get("roles").and_then(|v| v.as_bool()) {
                cfg.roles = v;
            }
            if let Some(v) = obj.get("users").and_then(|v| v.as_bool()) {
                cfg.users = v;
            }
            if let Some(v) = obj.get("replied_user").and_then(|v| v.as_bool()) {
                cfg.replied_user = v;
            }
        }
        cfg.everyone = env_bool_override("DISCORD_ALLOW_MENTION_EVERYONE").unwrap_or(cfg.everyone);
        cfg.roles = env_bool_override("DISCORD_ALLOW_MENTION_ROLES").unwrap_or(cfg.roles);
        cfg.users = env_bool_override("DISCORD_ALLOW_MENTION_USERS").unwrap_or(cfg.users);
        cfg.replied_user =
            env_bool_override("DISCORD_ALLOW_MENTION_REPLIED_USER").unwrap_or(cfg.replied_user);
        cfg
    }
}

fn env_bool_override(name: &str) -> Option<bool> {
    std::env::var(name)
        .ok()
        .map(|v| parse_bool_like(&v))
}

pub fn parse_bool_like(raw: &str) -> bool {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" | "" => false,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_defaults_block_everyone_and_roles() {
        let am = DiscordAllowedMentions::default();
        assert!(!am.everyone);
        assert!(!am.roles);
        assert!(am.users);
        assert!(am.replied_user);
        let v = am.to_api_value();
        let parse = v["parse"].as_array().unwrap();
        assert_eq!(parse.len(), 1);
        assert_eq!(parse[0], "users");
    }

    #[test]
    fn env_opts_into_everyone() {
        crate::test_env::set_var("DISCORD_ALLOW_MENTION_EVERYONE", "true");
        let cfg = PlatformConfig::default();
        let am = DiscordAllowedMentions::from_platform(&cfg);
        assert!(am.everyone);
        crate::test_env::remove_var("DISCORD_ALLOW_MENTION_EVERYONE");
    }
}
