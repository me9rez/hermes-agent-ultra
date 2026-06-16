//! list_strategies tool: List all available backtest strategies.

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::Value;

use hermes_core::{JsonSchema, ToolError, ToolHandler, ToolSchema, tool_schema};

pub struct ListStrategiesHandler;

impl ListStrategiesHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListStrategiesHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolHandler for ListStrategiesHandler {
    async fn execute(&self, _params: Value) -> Result<String, ToolError> {
        let strategies = hermes_vibe::StrategyRegistry::all();
        serde_json::to_string_pretty(&strategies)
            .map_err(|e| ToolError::ExecutionFailed(format!("Serialization error: {e}")))
    }

    fn schema(&self) -> ToolSchema {
        let props = IndexMap::new();

        tool_schema(
            "list_strategies",
            "List all available backtest strategy templates with their descriptions and default parameters. \
             Use this before run_backtest to discover which strategies are supported.",
            JsonSchema::object(props, vec![]),
        )
    }
}
