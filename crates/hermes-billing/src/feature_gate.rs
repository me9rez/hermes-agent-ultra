use std::collections::HashMap;

use hermes_accounts::{Account, ProviderTier, Tier};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tier_mapping::GlobalTierMapping;
use crate::tier_mapping::{mapping_requires_tier, tier_at_least};
use crate::tool_budget::{ToolBudget, ToolId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerticalCap {
    pub locked_features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureGate {
    pub allowed_provider_tiers: Vec<ProviderTier>,
    pub allowed_verticals: Vec<String>,
    pub allowed_vertical_caps: HashMap<String, VerticalCap>,
    pub max_concurrent_tasks: u32,
    pub max_tasks_per_day: Option<u32>,
    pub tool_budgets: HashMap<ToolId, ToolBudget>,
    pub byok_overrides_enabled: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FeatureGateError {
    #[error("provider tier {0:?} locked for current subscription")]
    ProviderTierLocked(ProviderTier),
    #[error("vertical {0} not available on current tier")]
    VerticalLocked(String),
    #[error("vertical feature {feature} locked on {vertical_id}")]
    VerticalFeatureLocked {
        vertical_id: String,
        feature: String,
    },
    #[error("concurrent task limit reached")]
    MaxConcurrentTasks,
    #[error("daily task limit reached")]
    MaxTasksPerDay,
    #[error("byok overrides not enabled on current tier")]
    ByokNotAllowed,
}

impl FeatureGate {
    pub fn for_tier(tier: Tier) -> Self {
        match tier {
            Tier::Free => Self {
                allowed_provider_tiers: vec![ProviderTier::Economic, ProviderTier::Local],
                allowed_verticals: vec!["trader".into(), "knowledge".into()],
                allowed_vertical_caps: HashMap::from([(
                    "trader".into(),
                    VerticalCap {
                        locked_features: vec![
                            "realtime_data".into(),
                            "tushare_pro".into(),
                            "intraday_chart".into(),
                        ],
                    },
                )]),
                max_concurrent_tasks: 1,
                max_tasks_per_day: Some(10),
                tool_budgets: crate::tool_budget::default_tool_budgets(tier),
                byok_overrides_enabled: false,
            },
            Tier::Pro => Self {
                allowed_provider_tiers: vec![
                    ProviderTier::Smart,
                    ProviderTier::Economic,
                    ProviderTier::Local,
                ],
                allowed_verticals: vec!["trader".into(), "knowledge".into(), "computer-use".into()],
                allowed_vertical_caps: HashMap::new(),
                max_concurrent_tasks: 5,
                max_tasks_per_day: None,
                tool_budgets: crate::tool_budget::default_tool_budgets(tier),
                byok_overrides_enabled: true,
            },
            Tier::Max => Self {
                allowed_provider_tiers: vec![
                    ProviderTier::Smart,
                    ProviderTier::Economic,
                    ProviderTier::Local,
                ],
                allowed_verticals: vec![],
                allowed_vertical_caps: HashMap::new(),
                max_concurrent_tasks: 20,
                max_tasks_per_day: None,
                tool_budgets: crate::tool_budget::default_tool_budgets(tier),
                byok_overrides_enabled: true,
            },
        }
    }

    pub fn allows_provider_tier(&self, tier: ProviderTier) -> bool {
        self.allowed_provider_tiers.contains(&tier)
    }

    pub fn check_provider_tier(&self, requested: ProviderTier) -> Result<(), FeatureGateError> {
        if self.allows_provider_tier(requested) {
            Ok(())
        } else {
            Err(FeatureGateError::ProviderTierLocked(requested))
        }
    }

    pub fn clamp_provider_tier(&self, requested: ProviderTier) -> ProviderTier {
        if self.allows_provider_tier(requested) {
            return requested;
        }
        if self.allows_provider_tier(ProviderTier::Economic) {
            ProviderTier::Economic
        } else if self.allows_provider_tier(ProviderTier::Local) {
            ProviderTier::Local
        } else {
            self.allowed_provider_tiers
                .first()
                .copied()
                .unwrap_or(ProviderTier::Economic)
        }
    }

    pub fn allows_vertical(&self, vertical_id: &str) -> bool {
        self.allowed_verticals.is_empty() || self.allowed_verticals.iter().any(|v| v == vertical_id)
    }

    pub fn check_vertical(&self, vertical_id: &str) -> Result<(), FeatureGateError> {
        if self.allows_vertical(vertical_id) {
            Ok(())
        } else {
            Err(FeatureGateError::VerticalLocked(vertical_id.to_string()))
        }
    }

    pub fn is_vertical_feature_locked(&self, vertical_id: &str, feature: &str) -> bool {
        self.allowed_vertical_caps
            .get(vertical_id)
            .is_some_and(|cap| cap.locked_features.iter().any(|f| f == feature))
    }

    pub fn check_vertical_feature(
        &self,
        vertical_id: &str,
        feature: &str,
    ) -> Result<(), FeatureGateError> {
        if self.is_vertical_feature_locked(vertical_id, feature) {
            Err(FeatureGateError::VerticalFeatureLocked {
                vertical_id: vertical_id.to_string(),
                feature: feature.to_string(),
            })
        } else {
            Ok(())
        }
    }
}

pub fn check_model_access(
    account_tier: Tier,
    requested_provider_tier: ProviderTier,
    global: &[GlobalTierMapping],
) -> Result<(), FeatureGateError> {
    let gate = FeatureGate::for_tier(account_tier);
    gate.check_provider_tier(requested_provider_tier)?;

    if let Some(required) = mapping_requires_tier(requested_provider_tier, global)
        && !tier_at_least(account_tier, required)
    {
        return Err(FeatureGateError::ProviderTierLocked(
            requested_provider_tier,
        ));
    }

    Ok(())
}

pub fn resolve_provider_tier(account: &Account) -> ProviderTier {
    let gate = FeatureGate::for_tier(account.tier);
    gate.clamp_provider_tier(account.provider_prefs.llm_tier)
}

pub fn effective_provider_tier(
    account: &Account,
    global: &[GlobalTierMapping],
) -> Result<ProviderTier, FeatureGateError> {
    let requested = account.provider_prefs.llm_tier;
    check_model_access(account.tier, requested, global)?;
    Ok(requested)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use hermes_accounts::{ProviderPreferences, QuotaState};
    use hermes_tasks::UserId;

    use super::*;
    use crate::tier_mapping::default_global_mappings;

    fn free_account(llm_tier: ProviderTier) -> Account {
        Account {
            user_id: UserId::new(),
            email: None,
            oauth_bindings: vec![],
            tier: Tier::Free,
            quota: QuotaState {
                period_start: Utc::now(),
                tokens_remaining_input: 100_000,
                tokens_remaining_output: 100_000,
                vertical_caps: HashMap::new(),
            },
            provider_prefs: ProviderPreferences {
                llm_tier,
                llm_raw_override: None,
                tts_voice_id: "warm".into(),
                tts_raw_override: None,
                stt_engine: "auto".into(),
                stt_raw_override: None,
                byok_overrides: vec![],
            },
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        }
    }

    #[test]
    fn free_tier_blocks_smart_provider_tier() {
        let gate = FeatureGate::for_tier(Tier::Free);
        assert_eq!(
            gate.check_provider_tier(ProviderTier::Smart),
            Err(FeatureGateError::ProviderTierLocked(ProviderTier::Smart))
        );
    }

    #[test]
    fn free_tier_clamps_smart_to_economic() {
        let account = free_account(ProviderTier::Smart);
        assert_eq!(resolve_provider_tier(&account), ProviderTier::Economic);
    }

    #[test]
    fn check_model_access_enforces_mapping_requires_tier() {
        let global = default_global_mappings();
        assert_eq!(
            check_model_access(Tier::Free, ProviderTier::Smart, &global),
            Err(FeatureGateError::ProviderTierLocked(ProviderTier::Smart))
        );
        assert!(check_model_access(Tier::Free, ProviderTier::Economic, &global).is_ok());
        assert!(check_model_access(Tier::Pro, ProviderTier::Smart, &global).is_ok());
    }

    #[test]
    fn trader_realtime_locked_on_free() {
        let gate = FeatureGate::for_tier(Tier::Free);
        assert!(gate.is_vertical_feature_locked("trader", "realtime_data"));
        assert!(
            gate.check_vertical_feature("trader", "realtime_data")
                .is_err()
        );
    }
}
