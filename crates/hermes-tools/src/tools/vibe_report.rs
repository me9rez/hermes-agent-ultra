//! get_backtest_report tool: Retrieve a previously saved backtest run card.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{Value, json};

use hermes_core::{JsonSchema, ToolError, ToolHandler, ToolSchema, tool_schema};

use crate::backends::vibe::RunCardStore;

pub struct GetBacktestReportHandler {
    store: Arc<dyn RunCardStore>,
}

impl GetBacktestReportHandler {
    pub fn new(store: Arc<dyn RunCardStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ToolHandler for GetBacktestReportHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'id' parameter".into()))?;

        let card = self.store.load(id).await?;

        serde_json::to_string_pretty(&card)
            .map_err(|e| ToolError::ExecutionFailed(format!("Serialization error: {e}")))
    }

    fn schema(&self) -> ToolSchema {
        let mut props = IndexMap::new();
        props.insert(
            "id".into(),
            json!({
                "type": "string",
                "description": "Run ID returned by run_backtest, e.g. 'BTC-USDT-sma_cross-20260616T143022Z'"
            }),
        );

        tool_schema(
            "get_backtest_report",
            "Retrieve a previously saved backtest report by its run ID. \
             Returns the full RunCard JSON including metrics, strategy, and period.",
            JsonSchema::object(props, vec!["id".into()]),
        )
    }
}
