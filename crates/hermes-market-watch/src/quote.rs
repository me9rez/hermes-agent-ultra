//! Quote provider trait and quote type.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::WatchError;

/// A single real-time (or delayed) quote snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub change: f64,
    pub change_pct: f64,
    pub volume: f64,
    pub high: f64,
    pub low: f64,
    pub timestamp: DateTime<Utc>,
}

/// Trait for providers that can fetch real-time / delayed quotes.
#[async_trait]
pub trait QuoteProvider: Send + Sync {
    /// Fetch the latest quote for a single symbol.
    async fn fetch_quote(&self, symbol: &str) -> Result<Quote, WatchError>;

    /// Fetch quotes for multiple symbols in one call.
    async fn fetch_quotes(&self, symbols: &[String]) -> Result<Vec<Quote>, WatchError> {
        let mut results = Vec::with_capacity(symbols.len());
        for sym in symbols {
            results.push(self.fetch_quote(sym).await?);
        }
        Ok(results)
    }

    /// Provider name (for logging / diagnostics).
    fn name(&self) -> &str;
}
