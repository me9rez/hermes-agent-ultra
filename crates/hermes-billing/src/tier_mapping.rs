use std::collections::HashMap;

use hermes_accounts::Tier;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTier {
    Smart,
    Economic,
    Local,
}

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
