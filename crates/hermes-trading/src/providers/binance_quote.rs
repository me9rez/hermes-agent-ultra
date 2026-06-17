//! Binance 24h ticker for crypto spot quotes.

use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use crate::error::TradingError;
use crate::http::{default_client, send_with_retry};
use crate::providers::binance::BinanceProvider;
use crate::quote_data::QuoteData;
use crate::quote_provider::QuoteProvider;

const BINANCE_TICKER_URL: &str = "https://api.binance.com/api/v3/ticker/24hr";

/// Crypto spot quote via Binance public ticker.
#[derive(Debug, Clone)]
pub struct BinanceQuoteProvider {
    client: reqwest::Client,
}

impl BinanceQuoteProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: default_client(),
        }
    }
}

impl Default for BinanceQuoteProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct BinanceTicker24hr {
    #[serde(rename = "lastPrice")]
    last_price: String,
    #[serde(rename = "priceChange")]
    price_change: String,
    #[serde(rename = "priceChangePercent")]
    price_change_percent: String,
    volume: String,
    #[serde(rename = "closeTime")]
    close_time: i64,
}

fn parse_f64(s: &str) -> Option<f64> {
    s.parse().ok()
}

#[async_trait]
impl QuoteProvider for BinanceQuoteProvider {
    async fn fetch_quote(&self, symbol: &str) -> Result<QuoteData, TradingError> {
        if !symbol.contains('-') {
            return Err(TradingError::SymbolNotFound(format!(
                "Binance quote expects pair format like BTC-USDT: {symbol}"
            )));
        }
        let binance_sym = BinanceProvider::to_binance_symbol(symbol);
        debug!(%binance_sym, "binance quote fetch");

        let client = self.client.clone();
        let sym = binance_sym.clone();
        let resp = send_with_retry(|| {
            client
                .get(BINANCE_TICKER_URL)
                .query(&[("symbol", sym.as_str())])
        })
        .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TradingError::InvalidResponse(format!(
                "Binance ticker HTTP {status}: {body}"
            )));
        }

        let ticker: BinanceTicker24hr = resp.json().await?;
        let price = parse_f64(&ticker.last_price);
        let mut out = QuoteData::new(symbol.to_uppercase(), "binance");
        out.price = price;
        out.change = parse_f64(&ticker.price_change);
        out.change_pct = parse_f64(&ticker.price_change_percent);
        out.volume = parse_f64(&ticker.volume);
        out.currency = Some("USDT".to_string());
        out.exchange = Some("BINANCE".to_string());
        out.set_market_timestamp_millis(ticker.close_time);
        out.finalize_partial();
        if out.price.is_none() {
            return Err(TradingError::NoData);
        }
        Ok(out)
    }

    fn name(&self) -> &str {
        "binance"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ticker_fields() {
        let raw = r#"{
            "lastPrice": "65000.12",
            "priceChange": "100.5",
            "priceChangePercent": "0.155",
            "volume": "12345.67",
            "closeTime": 1718640000123
        }"#;
        let t: BinanceTicker24hr = serde_json::from_str(raw).unwrap();
        assert_eq!(parse_f64(&t.last_price), Some(65000.12));
    }
}
