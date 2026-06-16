//! Vibe Research tool registrations (get_market_data, run_backtest, get_backtest_report, list_strategies, create_strategy).
//!
//! Requires the `vibe-research` Cargo feature.

use std::sync::Arc;
use tokio::sync::Mutex;
use super::{RegistryContext, reg};

pub fn register(ctx: &RegistryContext<'_>) {
    #[cfg(feature = "vibe-research")]
    {
        let store: Arc<dyn crate::backends::vibe::RunCardStore> =
            Arc::new(crate::backends::vibe::FileRunCardStore::default_path());

        // Build the strategy registry: built-ins + user strategies from disk.
        let strategies_dir = hermes_config::hermes_home().join("vibe").join("strategies");
        let mut registry = hermes_strategies::StrategyRegistry::with_builtins();
        // Fix 4: Load user strategies from disk at startup for cross-session persistence.
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(registry.load_from_dir(&strategies_dir));
        });
        let strategy_registry = Arc::new(Mutex::new(registry));

        reg(
            ctx,
            "vibe",
            Arc::new(crate::tools::vibe_market_data::GetMarketDataHandler::new()),
            "📈",
            vec![],
        );
        reg(
            ctx,
            "vibe",
            Arc::new(crate::tools::vibe_backtest::RunBacktestHandler::new(store.clone(), strategy_registry.clone())),
            "📊",
            vec![],
        );
        reg(
            ctx,
            "vibe",
            Arc::new(crate::tools::vibe_report::GetBacktestReportHandler::new(store)),
            "📑",
            vec![],
        );
        reg(
            ctx,
            "vibe",
            Arc::new(crate::tools::vibe_strategies::ListStrategiesHandler::new(strategy_registry.clone())),
            "📝",
            vec![],
        );
        reg(
            ctx,
            "vibe",
            Arc::new(crate::tools::vibe_create_strategy::CreateStrategyHandler::new(strategies_dir, strategy_registry)),
            "⚡",
            vec![],
        );
    }
}
