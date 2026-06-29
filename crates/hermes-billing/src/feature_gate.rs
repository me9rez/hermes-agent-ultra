use std::collections::HashMap;

use hermes_accounts::Tier;
use serde::{Deserialize, Serialize};

use crate::tier_mapping::ProviderTier;
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
}
