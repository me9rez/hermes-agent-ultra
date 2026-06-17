//! Auto-router for spot quotes by symbol format.

use tracing::debug;

use crate::error::TradingError;
use crate::quote_cache::QuoteCache;
use crate::quote_data::QuoteData;
use crate::quote_provider::QuoteProvider;
use crate::settlement::is_a_share;
use crate::symbol::{is_hk_share, is_us_share, normalize_symbol};

use super::binance_quote::BinanceQuoteProvider;
use super::eastmoney_quote::EastmoneyQuoteProvider;
use super::yahoo::YahooProvider;

/// Explicit quote data source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuoteSource {
    #[default]
    Auto,
    Yahoo,
    Eastmoney,
    Binance,
}

impl QuoteSource {
    pub fn parse(value: &str) -> Result<Self, TradingError> {
        match value.to_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "yahoo" => Ok(Self::Yahoo),
            "eastmoney" => Ok(Self::Eastmoney),
            "binance" => Ok(Self::Binance),
            other => Err(TradingError::SymbolNotFound(format!(
                "Unknown quote source '{other}'. Use auto, yahoo, eastmoney, or binance."
            ))),
        }
    }
}

/// Routes spot quote requests to Yahoo / Eastmoney / Binance (never stub).
#[derive(Debug)]
pub struct QuoteRouter {
    yahoo: Box<dyn QuoteProvider>,
    eastmoney: Box<dyn QuoteProvider>,
    binance: Box<dyn QuoteProvider>,
    cache: QuoteCache,
}

impl QuoteRouter {
    #[must_use]
    pub fn new() -> Self {
        Self::with_providers_and_cache(
            YahooProvider::new(),
            EastmoneyQuoteProvider::new(),
            BinanceQuoteProvider::new(),
            QuoteCache::default_path(),
        )
    }

    #[must_use]
    pub fn with_providers(
        yahoo: impl QuoteProvider + 'static,
        eastmoney: impl QuoteProvider + 'static,
        binance: impl QuoteProvider + 'static,
    ) -> Self {
        Self::with_providers_and_cache(yahoo, eastmoney, binance, QuoteCache::disabled())
    }

    #[must_use]
    pub fn with_providers_and_cache(
        yahoo: impl QuoteProvider + 'static,
        eastmoney: impl QuoteProvider + 'static,
        binance: impl QuoteProvider + 'static,
        cache: QuoteCache,
    ) -> Self {
        Self {
            yahoo: Box::new(yahoo),
            eastmoney: Box::new(eastmoney),
            binance: Box::new(binance),
            cache,
        }
    }

    pub async fn fetch_quote_with_source(
        &self,
        symbol: &str,
        source: QuoteSource,
        refresh: bool,
    ) -> Result<QuoteData, TradingError> {
        let canonical = normalize_symbol(symbol);
        let provider_name = match source {
            QuoteSource::Auto => self.select(&canonical)?.name(),
            QuoteSource::Yahoo => "yahoo",
            QuoteSource::Eastmoney => "eastmoney",
            QuoteSource::Binance => "binance",
        };

        let cache_key = QuoteCache::cache_key(provider_name, &canonical);
        if !refresh && let Some(cached) = self.cache.get(&cache_key).await {
            debug!(symbol = %canonical, provider = provider_name, "quote cache hit");
            return Ok(cached);
        }

        let data = match source {
            QuoteSource::Auto => self.select(&canonical)?.fetch_quote(&canonical).await?,
            QuoteSource::Yahoo => self.yahoo.fetch_quote(&canonical).await?,
            QuoteSource::Eastmoney => self.eastmoney.fetch_quote(&canonical).await?,
            QuoteSource::Binance => self.binance.fetch_quote(&canonical).await?,
        };

        if self.cache.put(&cache_key, &data).await.is_err() {
            debug!("quote cache write skipped or failed");
        }
        Ok(data)
    }

    fn select<'a>(&'a self, symbol: &str) -> Result<&'a dyn QuoteProvider, TradingError> {
        if is_a_share(symbol) {
            return Ok(self.eastmoney.as_ref());
        }
        if symbol.contains('-') {
            return Ok(self.binance.as_ref());
        }
        if is_hk_share(symbol) || is_us_share(symbol) {
            return Ok(self.yahoo.as_ref());
        }
        Err(TradingError::SymbolNotFound(format!(
            "Unsupported quote symbol '{symbol}'. Examples: AAPL, 0700.HK, 000001.SZ, BTC-USDT"
        )))
    }
}

impl Default for QuoteRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::fmt::Debug;

    use crate::quote_data::QuoteData;

    #[derive(Debug, Clone)]
    struct StubQuoteProvider {
        name: &'static str,
        price: f64,
    }

    #[async_trait]
    impl QuoteProvider for StubQuoteProvider {
        async fn fetch_quote(&self, symbol: &str) -> Result<QuoteData, TradingError> {
            let mut q = QuoteData::new(symbol, self.name);
            q.price = Some(self.price);
            q.partial = false;
            Ok(q)
        }

        fn name(&self) -> &str {
            self.name
        }
    }

    fn test_router() -> QuoteRouter {
        QuoteRouter::with_providers(
            StubQuoteProvider {
                name: "yahoo",
                price: 100.0,
            },
            StubQuoteProvider {
                name: "eastmoney",
                price: 10.0,
            },
            StubQuoteProvider {
                name: "binance",
                price: 50_000.0,
            },
        )
    }

    #[tokio::test]
    async fn routes_aapl_to_yahoo() {
        let r = test_router();
        let q = r
            .fetch_quote_with_source("AAPL", QuoteSource::Auto, true)
            .await
            .unwrap();
        assert_eq!(q.source, "yahoo");
        assert_eq!(q.price, Some(100.0));
    }

    #[tokio::test]
    async fn routes_a_share_to_eastmoney() {
        let r = test_router();
        let q = r
            .fetch_quote_with_source("000001.SZ", QuoteSource::Auto, true)
            .await
            .unwrap();
        assert_eq!(q.source, "eastmoney");
    }

    #[tokio::test]
    async fn routes_crypto_to_binance() {
        let r = test_router();
        let q = r
            .fetch_quote_with_source("BTC-USDT", QuoteSource::Auto, true)
            .await
            .unwrap();
        assert_eq!(q.source, "binance");
    }
}
