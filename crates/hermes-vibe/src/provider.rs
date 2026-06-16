//! Market data provider trait and factory.

use async_trait::async_trait;
use std::fmt::Debug;

use crate::error::VibeError;
use crate::types::{OhlcvData, OhlcvRequest};

/// Trait for market data providers (akshare, Binance, HTTP fallback, etc.)
#[async_trait]
pub trait MarketDataProvider: Send + Sync + Debug {
    /// Fetch OHLCV data for the given request.
    async fn fetch_ohlcv(&self, req: &OhlcvRequest) -> Result<OhlcvData, VibeError>;

    /// Returns the provider name (for logging/diagnostics).
    fn name(&self) -> &str;
}

#[async_trait]
impl MarketDataProvider for Box<dyn MarketDataProvider> {
    async fn fetch_ohlcv(&self, req: &OhlcvRequest) -> Result<OhlcvData, VibeError> {
        (**self).fetch_ohlcv(req).await
    }

    fn name(&self) -> &str {
        (**self).name()
    }
}
