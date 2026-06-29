use std::collections::HashMap;

use chrono::{DateTime, Utc};
use hermes_tasks::UserId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OauthProvider {
    Wechat,
    PhoneOtp,
    Email,
    Google,
    Microsoft,
    Apple,
    Github,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OauthBinding {
    pub provider: OauthProvider,
    pub subject: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Free,
    Pro,
    Max,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaState {
    pub period_start: DateTime<Utc>,
    pub tokens_remaining_input: u64,
    pub tokens_remaining_output: u64,
    pub vertical_caps: HashMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTier {
    Smart,
    Economic,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByokOverride {
    pub provider_id: String,
    pub credentials_keychain_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPreferences {
    pub llm_tier: ProviderTier,
    pub llm_raw_override: Option<String>,
    pub tts_voice_id: String,
    pub tts_raw_override: Option<String>,
    pub stt_engine: String,
    pub stt_raw_override: Option<String>,
    pub byok_overrides: Vec<ByokOverride>,
}

impl Default for ProviderPreferences {
    fn default() -> Self {
        Self {
            llm_tier: ProviderTier::Economic,
            llm_raw_override: None,
            tts_voice_id: "warm".into(),
            tts_raw_override: None,
            stt_engine: "auto".into(),
            stt_raw_override: None,
            byok_overrides: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub user_id: UserId,
    pub email: Option<String>,
    pub oauth_bindings: Vec<OauthBinding>,
    pub tier: Tier,
    pub quota: QuotaState,
    pub provider_prefs: ProviderPreferences,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentRecord {
    pub user_id: UserId,
    pub vertical_id: String,
    pub provider_ids: Vec<String>,
    pub granted_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub disclosure_text_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    pub user_id: UserId,
    pub avatar_url: Option<String>,
    pub display_name: String,
    pub access_token_keychain_id: String,
    pub refresh_token_keychain_id: String,
    pub last_active_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_preferences_default_economic() {
        let prefs = ProviderPreferences::default();
        assert_eq!(prefs.llm_tier, ProviderTier::Economic);
        assert_eq!(prefs.stt_engine, "auto");
    }

    #[test]
    fn tier_serializes_snake_case() {
        let json = serde_json::to_string(&Tier::Free).unwrap();
        assert_eq!(json, "\"free\"");
    }
}
