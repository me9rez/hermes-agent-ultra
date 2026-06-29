use std::collections::HashMap;

use chrono::{DateTime, Utc};
use hermes_accounts::Tier;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolId {
    WebSearch,
    VisionAnalyze,
    ComputerUse,
    ExecuteCode,
    DataSourceQuery,
    MemoryWrite,
    ArtifactWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBudget {
    pub tool_id: ToolId,
    pub monthly_limit: u32,
    pub used: u32,
    pub period_start: DateTime<Utc>,
}

pub fn default_tool_budgets(tier: Tier) -> HashMap<ToolId, ToolBudget> {
    let now = Utc::now();
    let mk = |tool_id: ToolId, monthly_limit: u32| ToolBudget {
        tool_id,
        monthly_limit,
        used: 0,
        period_start: now,
    };
    match tier {
        Tier::Free => HashMap::from([
            (ToolId::WebSearch, mk(ToolId::WebSearch, 50)),
            (ToolId::VisionAnalyze, mk(ToolId::VisionAnalyze, 20)),
            (ToolId::ComputerUse, mk(ToolId::ComputerUse, 0)),
            (ToolId::ExecuteCode, mk(ToolId::ExecuteCode, 200)),
        ]),
        Tier::Pro => HashMap::from([
            (ToolId::WebSearch, mk(ToolId::WebSearch, 500)),
            (ToolId::VisionAnalyze, mk(ToolId::VisionAnalyze, 300)),
            (ToolId::ComputerUse, mk(ToolId::ComputerUse, 50)),
            (ToolId::ExecuteCode, mk(ToolId::ExecuteCode, u32::MAX)),
        ]),
        Tier::Max => HashMap::from([
            (ToolId::WebSearch, mk(ToolId::WebSearch, u32::MAX)),
            (ToolId::VisionAnalyze, mk(ToolId::VisionAnalyze, u32::MAX)),
            (ToolId::ComputerUse, mk(ToolId::ComputerUse, u32::MAX)),
            (ToolId::ExecuteCode, mk(ToolId::ExecuteCode, u32::MAX)),
        ]),
    }
}
