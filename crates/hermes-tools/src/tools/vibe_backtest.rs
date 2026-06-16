//! run_backtest tool: Run a template backtest strategy on market data.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use hermes_core::{JsonSchema, ToolError, ToolHandler, ToolSchema, tool_schema};
use hermes_vibe::MarketDataProvider;

use crate::backends::vibe::RunCardStore;

pub struct RunBacktestHandler {
    store: Arc<dyn RunCardStore>,
    strategy_registry: Arc<Mutex<hermes_strategies::StrategyRegistry>>,
}

impl RunBacktestHandler {
    pub fn new(
        store: Arc<dyn RunCardStore>,
        strategy_registry: Arc<Mutex<hermes_strategies::StrategyRegistry>>,
    ) -> Self {
        Self { store, strategy_registry }
    }
}

#[async_trait]
impl ToolHandler for RunBacktestHandler {
    async fn execute(&self, params: Value) -> Result<String, ToolError> {
        let symbol = params
            .get("symbol")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'symbol' parameter".into()))?;

        let strategy = params
            .get("strategy")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("Missing 'strategy' parameter".into()))?;

        let strategy_params = params.get("params").cloned().unwrap_or_else(|| json!({}));

        let end_date = params
            .get("end_date")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| chrono::Utc::now().date_naive());

        let start_date = params
            .get("start_date")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            .unwrap_or_else(|| end_date - chrono::Duration::days(180));

        // Fetch market data
        let req = hermes_vibe::OhlcvRequest {
            symbol: symbol.to_string(),
            start: start_date,
            end: end_date,
            interval: hermes_vibe::Interval::Daily,
        };

        let router = hermes_vibe::AutoRouter::new();
        let data = router
            .fetch_ohlcv(&req)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch market data: {e}")))?;

        // Run backtest — try declarative strategy first, then fallback to hardcoded.
        let strategy_name = strategy; // save before we shadow it
        let card = {
            let reg = self.strategy_registry.lock().await;
            
            // Fix 1: If user provided params, use hardcoded path for param override support.
            let has_params = strategy_params.as_object().map_or(false, |o| !o.is_empty());
            
            // Fix 9: Get strategy clone, then release lock before expensive computation.
            let strategy_opt = reg.get(strategy_name);
            drop(reg);
            
            if let Some(strategy) = strategy_opt {
                if has_params {
                    // User passed params → use hardcoded path for backward compatibility.
                    hermes_vibe::BacktestEngine::run(&data, strategy_name, &strategy_params)
                        .map_err(|e| ToolError::ExecutionFailed(format!("Backtest failed: {e}")))?
                } else {
                    // Declarative strategy path.
                    let decisions = strategy.run(&data).map_err(|e| {
                        ToolError::ExecutionFailed(format!("Strategy execution failed: {e}"))
                    })?;
                    // Convert Decision → SignalKind.
                    let signals: Vec<hermes_vibe::SignalKind> = decisions
                        .iter()
                        .map(|d| match d.signal {
                            hermes_strategies::Signal::Buy => hermes_vibe::SignalKind::Buy,
                            hermes_strategies::Signal::Sell => hermes_vibe::SignalKind::Sell,
                            hermes_strategies::Signal::Hold => hermes_vibe::SignalKind::Hold,
                        })
                        .collect();
                    hermes_vibe::BacktestEngine::run_from_signals(&data, strategy_name, &strategy_params, &signals)
                        .map_err(|e| ToolError::ExecutionFailed(format!("Backtest failed: {e}")))?
                }
            } else {
                // Fix 8: Fallback failed — provide helpful error with available strategies.
                let reg = self.strategy_registry.lock().await;
                let available = reg.list().into_iter().map(|s| s.name).collect::<Vec<_>>();
                drop(reg);
                let hint = if available.is_empty() {
                    String::new()
                } else {
                    format!(" Available strategies: {}.", available.join(", "))
                };
                return Err(ToolError::ExecutionFailed(
                    format!("Unsupported strategy '{}'.{}", strategy_name, hint)
                ));
            }
        };

        // Attach persistence metadata and save to disk.
        let now = chrono::Utc::now();
        let id = card.generate_id(&now);
        let card = card.with_persistence_meta(id, now.to_rfc3339());
        if let Err(e) = self.store.save(&card).await {
            tracing::warn!(error = %e, "Failed to persist run_card; returning result without saving");
        }

        serde_json::to_string_pretty(&card)
            .map_err(|e| ToolError::ExecutionFailed(format!("Serialization error: {e}")))
    }

    fn schema(&self) -> ToolSchema {
        let mut props = IndexMap::new();
        props.insert(
            "symbol".into(),
            json!({
                "type": "string",
                "description": "Symbol to backtest. Examples: 'BTC-USDT', '000001.SZ', '600519.SH'"
            }),
        );
        props.insert(
            "strategy".into(),
            json!({
                "type": "string",
                "description": "Strategy name. Use list_strategies to see all available strategies (built-in + user-created)."
            }),
        );
        props.insert(
            "params".into(),
            json!({
                "type": "object",
                "description": "Strategy parameters. Use list_strategies to see default params per strategy.",
                "properties": {
                    "short_window": {"type": "integer", "description": "Short SMA window (sma_cross, default: 20)"},
                    "long_window": {"type": "integer", "description": "Long SMA window (sma_cross, default: 50)"},
                    "rsi_period": {"type": "integer", "description": "RSI period (rsi_revert, default: 14)"},
                    "oversold": {"type": "number", "description": "RSI oversold threshold (rsi_revert, default: 30)"},
                    "overbought": {"type": "number", "description": "RSI overbought threshold (rsi_revert, default: 70)"}
                }
            }),
        );
        props.insert(
            "start_date".into(),
            json!({
                "type": "string",
                "description": "Backtest start date in YYYY-MM-DD format (default: 180 days ago)"
            }),
        );
        props.insert(
            "end_date".into(),
            json!({
                "type": "string",
                "description": "Backtest end date in YYYY-MM-DD format (default: today)"
            }),
        );

        tool_schema(
            "run_backtest",
            "Run a template backtest strategy on historical market data. \
             Returns performance metrics including return, max drawdown, Sharpe ratio, and trade count. \
             Data is fetched automatically.",
            JsonSchema::object(props, vec!["symbol".into(), "strategy".into()]),
        )
    }
}
