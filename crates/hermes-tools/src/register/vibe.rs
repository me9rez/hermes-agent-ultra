//! Vibe Research tool registrations (get_market_data, run_backtest, get_backtest_report, list_strategies).
//!
//! Requires the `vibe-research` Cargo feature.

use std::sync::Arc;
use super::{RegistryContext, reg};

pub fn register(ctx: &RegistryContext<'_>) {
    #[cfg(feature = "vibe-research")]
    {
        let store: Arc<dyn crate::backends::vibe::RunCardStore> =
            Arc::new(crate::backends::vibe::FileRunCardStore::default_path());

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
            Arc::new(crate::tools::vibe_backtest::RunBacktestHandler::new(store.clone())),
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
            Arc::new(crate::tools::vibe_strategies::ListStrategiesHandler::new()),
            "📝",
            vec![],
        );
    }
}
