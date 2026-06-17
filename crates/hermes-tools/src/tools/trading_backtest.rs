//! run_backtest tool: Run a template backtest strategy on market data.

use std::sync::Arc;

use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use hermes_core::{JsonSchema, ToolError, ToolHandler, ToolSchema, tool_schema};

use crate::backends::trading::RunCardStore;

pub struct RunBacktestHandler {
    store: Arc<dyn RunCardStore>,
    strategy_registry: Arc<Mutex<hermes_strategies::StrategyRegistry>>,
}

impl RunBacktestHandler {
    pub fn new(
        store: Arc<dyn RunCardStore>,
        strategy_registry: Arc<Mutex<hermes_strategies::StrategyRegistry>>,
    ) -> Self {
        Self {
            store,
            strategy_registry,
        }
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
        let mut strategy_params = strategy_params;
        if let Some(rf) = params.get("risk_free_rate").and_then(|v| v.as_f64()) {
            if let Some(obj) = strategy_params.as_object_mut() {
                obj.insert("risk_free_rate".into(), json!(rf));
            }
        }

        let source = params
            .get("source")
            .and_then(|v| v.as_str())
            .map(hermes_trading::DataSource::parse)
            .transpose()
            .map_err(|e| ToolError::InvalidParams(e.to_string()))?
            .unwrap_or(hermes_trading::DataSource::Auto);

        let refresh = params
            .get("refresh")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

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
        let req = hermes_trading::OhlcvRequest {
            symbol: symbol.to_string(),
            start: start_date,
            end: end_date,
            interval: hermes_trading::Interval::Daily,
        };

        let router = hermes_trading::AutoRouter::new();
        let data = router
            .fetch_ohlcv_with_source(&req, source, refresh)
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
                    hermes_trading::BacktestEngine::run(&data, strategy_name, &strategy_params)
                        .map_err(|e| ToolError::ExecutionFailed(format!("Backtest failed: {e}")))?
                } else {
                    // Declarative strategy path.
                    let decisions = strategy.run(&data).map_err(|e| {
                        ToolError::ExecutionFailed(format!("Strategy execution failed: {e}"))
                    })?;
                    // Convert Decision → SignalKind.
                    let signals: Vec<hermes_trading::SignalKind> = decisions
                        .iter()
                        .map(|d| match d.signal {
                            hermes_strategies::Signal::Buy => hermes_trading::SignalKind::Buy,
                            hermes_strategies::Signal::Sell => hermes_trading::SignalKind::Sell,
                            hermes_strategies::Signal::Hold => hermes_trading::SignalKind::Hold,
                        })
                        .collect();
                    hermes_trading::BacktestEngine::run_from_signals(
                        &data,
                        strategy_name,
                        &strategy_params,
                        &signals,
                    )
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
                return Err(ToolError::ExecutionFailed(format!(
                    "Unsupported strategy '{}'.{}",
                    strategy_name, hint
                )));
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
                "description": "Symbol to backtest. Examples: 'BTC-USDT', '000001.SZ'"
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
                    "overbought": {"type": "number", "description": "RSI overbought threshold (rsi_revert, default: 70)"},
                    "risk_free_rate": {"type": "number", "description": "Annual risk-free rate for Sharpe (default: 0.0)"}
                }
            }),
        );
        props.insert(
            "source".into(),
            json!({
                "type": "string",
                "description": "Data source: 'auto' (default), 'binance', or 'eastmoney'",
                "enum": ["auto", "binance", "eastmoney"]
            }),
        );
        props.insert(
            "risk_free_rate".into(),
            json!({
                "type": "number",
                "description": "Annual risk-free rate for Sharpe ratio (default: 0.0). Also accepted inside params."
            }),
        );
        props.insert(
            "refresh".into(),
            json!({
                "type": "boolean",
                "description": "Bypass disk cache and force network fetch (default: false)"
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
