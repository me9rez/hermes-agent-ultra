//! Auto-router that selects the appropriate provider based on symbol format.
//!
//! Routing rules:
//! - Symbols containing `-` (e.g. `"BTC-USDT"`, `"ETH-BTC"`) → [`BinanceProvider`]
//! - Symbols ending in `.SZ` or `.SH` (e.g. `"000001.SZ"`) → [`EastmoneyProvider`]

use async_trait::async_trait;
use tracing::debug;

use crate::error::VibeError;
use crate::provider::MarketDataProvider;
use crate::types::{OhlcvData, OhlcvRequest};

use super::binance::BinanceProvider;
use super::eastmoney::EastmoneyProvider;

/// Automatic market data router that dispatches to the correct provider
/// based on the symbol format.
#[derive(Debug, Clone)]
pub struct AutoRouter {
    binance: BinanceProvider,
    eastmoney: EastmoneyProvider,
}

impl AutoRouter {
    /// Create a new `AutoRouter` with default providers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            binance: BinanceProvider::new(),
            eastmoney: EastmoneyProvider::new(),
        }
    }

    /// Create with pre-configured providers.
    #[must_use]
    pub fn with_providers(binance: BinanceProvider, eastmoney: EastmoneyProvider) -> Self {
        Self { binance, eastmoney }
    }

    /// Determine which provider to use based on the symbol format.
    fn select<'a>(&'a self, symbol: &str) -> Result<&'a dyn MarketDataProvider, VibeError> {
        let upper = symbol.to_uppercase();
        if upper.ends_with(".SZ") || upper.ends_with(".SH") {
            debug!(symbol = %symbol, provider = "eastmoney", "AutoRouter selected");
            Ok(&self.eastmoney)
        } else if symbol.contains('-') {
            debug!(symbol = %symbol, provider = "binance", "AutoRouter selected");
            Ok(&self.binance)
        } else {
            Err(VibeError::SymbolNotFound(format!(
                "Cannot determine provider for symbol '{symbol}'. \
                 Use 'XXX-YYY' for crypto (Binance) or 'XXXXXX.SZ/.SH' for A-shares (Eastmoney)."
            )))
        }
    }
}

impl Default for AutoRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MarketDataProvider for AutoRouter {
    async fn fetch_ohlcv(&self, req: &OhlcvRequest) -> Result<OhlcvData, VibeError> {
        let provider = self.select(&req.symbol)?;
        provider.fetch_ohlcv(req).await
    }

    fn name(&self) -> &str {
        "auto-router"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_selects_binance() {
        let router = AutoRouter::new();
        assert_eq!(router.select("BTC-USDT").unwrap().name(), "binance");
        assert_eq!(router.select("ETH-BTC").unwrap().name(), "binance");
    }

    #[test]
    fn test_router_selects_eastmoney() {
        let router = AutoRouter::new();
        assert_eq!(router.select("000001.SZ").unwrap().name(), "eastmoney");
        assert_eq!(router.select("600519.SH").unwrap().name(), "eastmoney");
    }

    #[test]
    fn test_router_unknown_symbol() {
        let router = AutoRouter::new();
        assert!(router.select("AAPL").is_err());
    }
}
