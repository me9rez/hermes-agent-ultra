//! get_market_data tool: Fetch OHLCV market data for a symbol.

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{Value, json};

use hermes_core::{JsonSchema, ToolError, ToolHandler, ToolSchema, tool_schema};
use hermes_vibe::MarketDataProvider;

#[derive(Default)]
pub struct GetMarketDataHandler;

impl GetMarketDataHandler {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolHandler for GetMarketDataHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let symbol = params
            .get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'symbol' parameter".into()))?;

        let end_date = params
            .get("end_date")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| chrono::Utc::now().date_naive());

        let start_date = params
            .get("start_date")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| end_date - chrono::Duration::days(30));

        let interval = match params.get("interval").and_then(|v| v.as_str()) {
            Some("weekly") => hermes_vibe::Interval::Weekly,
            _ => hermes_vibe::Interval::Daily,
        };

        let req = hermes_vibe::OhlcvRequest {
            symbol: symbol.to_string(),
            start: start_date,
            end: end_date,
            interval,
        };

        let router = hermes_vibe::AutoRouter::new();
        let data = router
            .fetch_ohlcv(&req)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch market data: {e}")))?;

        serde_json::to_string_pretty(&data)
            .map_err(|e| ToolError::ExecutionFailed(format!("Serialization error: {e}")))
    }

    fn schema(&self) -> ToolSchema {
        let mut props = IndexMap::new();
        props.insert(
            "symbol".into(),
            json!({
                "type": "string",
                "description": "Symbol identifier. Examples: 'BTC-USDT' (crypto), '000001.SZ' (Shenzhen A-share), '600519.SH' (Shanghai A-share)"
            }),
        );
        props.insert(
            "start_date".into(),
            json!({
                "type": "string",
                "description": "Start date in YYYY-MM-DD format (default: 30 days ago)"
            }),
        );
        props.insert(
            "end_date".into(),
            json!({
                "type": "string",
                "description": "End date in YYYY-MM-DD format (default: today)"
            }),
        );
        props.insert(
            "interval".into(),
            json!({
                "type": "string",
                "description": "Data interval: 'daily' or 'weekly' (default: daily)",
                "enum": ["daily", "weekly"]
            }),
        );

        tool_schema(
            "get_market_data",
            "Fetch OHLCV (Open/High/Low/Close/Volume) market data for a symbol. \
             Supports A-shares (e.g. 000001.SZ, 600519.SH) and crypto (e.g. BTC-USDT). \
             No API key required.",
            JsonSchema::object(props, vec!["symbol".into()]),
        )
    }
}
