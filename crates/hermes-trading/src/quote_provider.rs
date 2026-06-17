//! Trait for spot quote providers.

use std::fmt::Debug;

use async_trait::async_trait;

use crate::error::TradingError;
use crate::quote_data::QuoteData;

/// Fetch live spot quotes (distinct from OHLCV [`MarketDataProvider`](crate::provider::MarketDataProvider)).
#[async_trait]
pub trait QuoteProvider: Send + Sync + Debug {
    async fn fetch_quote(&self, symbol: &str) -> Result<QuoteData, TradingError>;
    fn name(&self) -> &str;
}
