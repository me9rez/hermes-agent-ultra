//! Copilot orchestrator: ties together data, strategies, and alerts.

use std::sync::Arc;

use tracing::info;

use hermes_market_watch::{AlertEngine, AlertTrigger, Quote, Watchlist};
use hermes_strategies::{Decision, Strategy};
use hermes_vibe::{MarketDataProvider, OhlcvData, OhlcvRequest};

use crate::error::CopilotError;
use crate::report::AnalysisReport;

/// Lightweight copilot that orchestrates market data fetching, strategy
/// execution, and alert evaluation.
pub struct CopilotLite {
    provider: Option<Arc<dyn MarketDataProvider>>,
    strategies: Vec<Arc<dyn Strategy>>,
    watchlist: Watchlist,
    alert_engine: AlertEngine,
}

impl Default for CopilotLite {
    fn default() -> Self {
        Self::new()
    }
}

impl CopilotLite {
    /// Create a new copilot with no provider and no strategies.
    pub fn new() -> Self {
        Self {
            provider: None,
            strategies: Vec::new(),
            watchlist: Watchlist::new(),
            alert_engine: AlertEngine::new(),
        }
    }

    /// Set the market data provider.
    pub fn set_provider(&mut self, provider: Arc<dyn MarketDataProvider>) {
        self.provider = Some(provider);
    }

    /// Register a strategy.
    pub fn add_strategy(&mut self, strategy: Arc<dyn Strategy>) {
        self.strategies.push(strategy);
    }

    /// Access the watchlist mutably.
    pub fn watchlist_mut(&mut self) -> &mut Watchlist {
        &mut self.watchlist
    }

    /// Access the alert engine mutably.
    pub fn alert_engine_mut(&mut self) -> &mut AlertEngine {
        &mut self.alert_engine
    }

    /// Fetch OHLCV data and run all registered strategies, producing a
    /// combined analysis report.
    pub async fn analyze(&self, req: &OhlcvRequest) -> Result<AnalysisReport, CopilotError> {
        let provider = self
            .provider
            .as_ref()
            .ok_or(CopilotError::NoProvider)?;

        if self.strategies.is_empty() {
            return Err(CopilotError::NoStrategy);
        }

        info!(symbol = %req.symbol, "fetching OHLCV data");
        let data: OhlcvData = provider.fetch_ohlcv(req).await?;

        let mut all_decisions: Vec<(String, Vec<Decision>)> = Vec::new();
        for strategy in &self.strategies {
            let decisions = strategy.run(&data)?;
            info!(
                strategy = %strategy.name(),
                bars = decisions.len(),
                "strategy executed"
            );
            all_decisions.push((strategy.name().to_string(), decisions));
        }

        Ok(AnalysisReport {
            symbol: req.symbol.clone(),
            data_points: data.len(),
            strategy_decisions: all_decisions,
        })
    }

    /// Evaluate alerts against a quote snapshot.
    pub fn check_alerts(&self, quote: &Quote) -> Vec<AlertTrigger> {
        self.alert_engine.evaluate(quote)
    }
}
