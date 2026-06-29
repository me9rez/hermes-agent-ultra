use std::collections::HashMap;

use hermes_accounts::{ProviderTier, Tier};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalTierMapping {
    pub tier: ProviderTier,
    pub primary_model: String,
    pub fallback_models: Vec<String>,
    pub display_name_key: String,
    pub price_label_key: String,
    pub requires_tier: Tier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerticalTierOverrides {
    pub vertical_id: String,
    pub overrides: HashMap<ProviderTier, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("no mapping for tier {0:?}")]
    MissingTier(ProviderTier),
}

pub fn default_global_mappings() -> Vec<GlobalTierMapping> {
    vec![
        GlobalTierMapping {
            tier: ProviderTier::Smart,
            primary_model: "tongyi-qwen-max".into(),
            fallback_models: vec!["kimi-k2".into(), "deepseek-r1".into()],
            display_name_key: "provider.tier.smart".into(),
            price_label_key: "provider.tier.smart.price".into(),
            requires_tier: Tier::Pro,
        },
        GlobalTierMapping {
            tier: ProviderTier::Economic,
            primary_model: "tongyi-qwen-turbo".into(),
            fallback_models: vec!["kimi-32k".into()],
            display_name_key: "provider.tier.economic".into(),
            price_label_key: "provider.tier.economic.price".into(),
            requires_tier: Tier::Free,
        },
        GlobalTierMapping {
            tier: ProviderTier::Local,
            primary_model: "ollama-qwen3-32b".into(),
            fallback_models: vec![],
            display_name_key: "provider.tier.local".into(),
            price_label_key: "provider.tier.local.price".into(),
            requires_tier: Tier::Free,
        },
    ]
}

pub fn default_vertical_overrides() -> Vec<VerticalTierOverrides> {
    vec![
        VerticalTierOverrides {
            vertical_id: "trader".into(),
            overrides: HashMap::from([
                (ProviderTier::Smart, "tongyi-qwen-max".into()),
                (ProviderTier::Economic, "tongyi-qwen-turbo".into()),
                (ProviderTier::Local, "ollama-qwen3-32b".into()),
            ]),
        },
        VerticalTierOverrides {
            vertical_id: "knowledge".into(),
            overrides: HashMap::from([
                (ProviderTier::Smart, "kimi-k2".into()),
                (ProviderTier::Economic, "kimi-32k".into()),
                (ProviderTier::Local, "ollama-qwen3-14b".into()),
            ]),
        },
        VerticalTierOverrides {
            vertical_id: "computer-use".into(),
            overrides: HashMap::from([
                (ProviderTier::Smart, "gpt-5-relay".into()),
                (ProviderTier::Economic, "qwen-vl-max".into()),
            ]),
        },
    ]
}

pub fn tier_at_least(have: Tier, need: Tier) -> bool {
    match (have, need) {
        (Tier::Max, _) => true,
        (Tier::Pro, Tier::Max) => false,
        (Tier::Pro, _) => true,
        (Tier::Free, Tier::Free) => true,
        (Tier::Free, _) => false,
    }
}

pub fn mapping_requires_tier(
    provider_tier: ProviderTier,
    global: &[GlobalTierMapping],
) -> Option<Tier> {
    global
        .iter()
        .find(|m| m.tier == provider_tier)
        .map(|m| m.requires_tier)
}

pub fn resolve_model(
    vertical_id: &str,
    tier: ProviderTier,
    overrides: &[VerticalTierOverrides],
    global: &[GlobalTierMapping],
) -> Result<String, ResolveError> {
    if let Some(v) = overrides.iter().find(|o| o.vertical_id == vertical_id)
        && let Some(model) = v.overrides.get(&tier)
    {
        return Ok(model.clone());
    }
    global
        .iter()
        .find(|m| m.tier == tier)
        .map(|m| m.primary_model.clone())
        .ok_or(ResolveError::MissingTier(tier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_vertical_override() {
        let global = default_global_mappings();
        let overrides = default_vertical_overrides();
        let model = resolve_model("trader", ProviderTier::Smart, &overrides, &global).unwrap();
        assert_eq!(model, "tongyi-qwen-max");
    }

    #[test]
    fn resolve_falls_back_to_global() {
        let global = default_global_mappings();
        let model = resolve_model("unknown", ProviderTier::Economic, &[], &global).unwrap();
        assert_eq!(model, "tongyi-qwen-turbo");
    }

    #[test]
    fn tier_at_least_orders_correctly() {
        assert!(tier_at_least(Tier::Pro, Tier::Free));
        assert!(!tier_at_least(Tier::Free, Tier::Pro));
    }
}
