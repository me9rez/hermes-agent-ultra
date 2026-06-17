//! Mock quote provider for parity tests (deterministic, no network).

use async_trait::async_trait;

use crate::error::TradingError;
use crate::quote_data::QuoteData;
use crate::quote_provider::QuoteProvider;
use crate::settlement::is_a_share;
use crate::symbol::{is_hk_share, is_us_share};

/// Deterministic quote provider for tests and parity fixtures.
#[derive(Debug, Clone, Default)]
pub struct MockQuoteProvider;

impl MockQuoteProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    fn base_price(symbol: &str) -> f64 {
        match symbol {
            "BTC-USDT" => 65_000.0,
            "ETH-BTC" => 0.05,
            "000001.SZ" | "600519.SH" => 10.5,
            s if s.ends_with(".HK") => 350.0,
            "AAPL" => 291.58,
            _ => 100.0,
        }
    }

    fn source_name(symbol: &str) -> &'static str {
        if is_a_share(symbol) {
            "eastmoney"
        } else if symbol.contains('-') {
            "binance"
        } else if is_hk_share(symbol) || is_us_share(symbol) {
            "yahoo"
        } else {
            "mock"
        }
    }
}

#[async_trait]
impl QuoteProvider for MockQuoteProvider {
    async fn fetch_quote(&self, symbol: &str) -> Result<QuoteData, TradingError> {
        if symbol == "INVALID_XYZ" {
            return Err(TradingError::SymbolNotFound(format!(
                "Symbol '{symbol}' not found"
            )));
        }
        let price = Self::base_price(symbol);
        let mut q = QuoteData::new(symbol, Self::source_name(symbol));
        q.price = Some(price);
        q.change = Some(1.0);
        q.change_pct = Some(0.5);
        q.currency = Some(if is_a_share(symbol) {
            "CNY".to_string()
        } else {
            "USD".to_string()
        });
        q.finalize_partial();
        Ok(q)
    }

    fn name(&self) -> &str {
        "mock"
    }
}
