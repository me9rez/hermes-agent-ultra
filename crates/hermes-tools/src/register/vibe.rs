//! Vibe Research tool registrations (get_market_data, run_backtest).
//!
//! Requires the `vibe-research` Cargo feature.

use std::sync::Arc;
use super::{RegistryContext, reg};

pub fn register(ctx: &RegistryContext<'_>) {
    #[cfg(feature = "vibe-research")]
    {
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
            Arc::new(crate::tools::vibe_backtest::RunBacktestHandler::new()),
            "📊",
            vec![],
        );
    }
}
