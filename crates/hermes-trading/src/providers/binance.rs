//! Binance REST API provider for crypto OHLCV data.
//!
//! Endpoint: `https://api.binance.com/api/v3/klines`
//! No API key required for public market data.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::error::TradingError;
use crate::http::{default_client, send_with_retry};
use crate::provider::MarketDataProvider;
use crate::types::{Interval, OhlcvData, OhlcvRequest, OhlcvRow, mark_partial};

/// Base URL for Binance public REST API v3.
const BINANCE_BASE_URL: &str = "https://api.binance.com/api/v3/klines";

/// Binance market data provider.
///
/// Fetches crypto OHLCV data from the Binance public klines endpoint.
/// Symbol format: `"BTC-USDT"` → internally converted to `"BTCUSDT"`.
#[derive(Debug, Clone)]
pub struct BinanceProvider {
    client: reqwest::Client,
}

impl BinanceProvider {
    /// Create a new `BinanceProvider` with a default HTTP client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: default_client(),
        }
    }

    /// Create with a custom HTTP client (useful for testing / custom timeouts).
    #[must_use]
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Convert user-facing symbol (e.g. `"BTC-USDT"`) to Binance format (`"BTCUSDT"`).
    pub(crate) fn to_binance_symbol(symbol: &str) -> String {
        symbol.replace('-', "")
    }

    /// Map [`Interval`] to the Binance `interval` query parameter.
    fn to_binance_interval(interval: Interval) -> &'static str {
        match interval {
            Interval::Daily => "1d",
            Interval::Weekly => "1w",
        }
    }

    /// Convert a [`NaiveDate`] to a millisecond timestamp (UTC midnight).
    fn date_to_ms(date: NaiveDate) -> i64 {
        date.and_hms_opt(0, 0, 0)
            .expect("midnight is always valid")
            .and_utc()
            .timestamp_millis()
    }
}

impl Default for BinanceProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// A single kline entry returned by Binance.
/// Binance returns: [open_time, open, high, low, close, volume, close_time, ...]
/// We only deserialize the fields we need (indices 0–5).
#[derive(Debug, Deserialize)]
struct BinanceKline(
    i64,    // [0] open time (ms)
    String, // [1] open
    String, // [2] high
    String, // [3] low
    String, // [4] close
    String, // [5] volume
);

#[async_trait]
impl MarketDataProvider for BinanceProvider {
    async fn fetch_ohlcv(&self, req: &OhlcvRequest) -> Result<OhlcvData, TradingError> {
        let symbol = Self::to_binance_symbol(&req.symbol);
        let interval = Self::to_binance_interval(req.interval);
        let start_ms = Self::date_to_ms(req.start);
        // End date is inclusive: set to end-of-day (23:59:59.999 UTC)
        let end_ms = Self::date_to_ms(req.end) + 86_399_999;

        debug!(
            symbol = %symbol,
            interval = %interval,
            start_ms,
            end_ms,
            "Binance klines request"
        );

        let resp = send_with_retry(|| {
            self.client.get(BINANCE_BASE_URL).query(&[
                ("symbol", symbol.as_str()),
                ("interval", interval),
                ("startTime", &start_ms.to_string()),
                ("endTime", &end_ms.to_string()),
                ("limit", "1000"),
            ])
        })
        .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(%status, body = %body, "Binance API error");
            return Err(TradingError::InvalidResponse(format!(
                "Binance returned HTTP {status}: {body}"
            )));
        }

        let klines: Vec<BinanceKline> = resp.json().await?;

        let rows: Vec<OhlcvRow> = klines
            .into_iter()
            .filter_map(|k| {
                let date = chrono::DateTime::from_timestamp_millis(k.0)?.date_naive();
                let parse = |s: &str| -> Option<f64> { s.parse().ok() };
                Some(OhlcvRow {
                    date,
                    open: parse(&k.1)?,
                    high: parse(&k.2)?,
                    low: parse(&k.3)?,
                    close: parse(&k.4)?,
                    volume: parse(&k.5)?,
                })
            })
            .collect();

        if rows.is_empty() {
            return Err(TradingError::NoData);
        }

        debug!(rows = rows.len(), "Binance klines parsed");

        let mut data = OhlcvData {
            symbol: req.symbol.clone(),
            interval: req.interval,
            rows,
            partial: false,
        };
        mark_partial(&mut data, req);
        Ok(data)
    }

    fn name(&self) -> &str {
        "binance"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_conversion() {
        assert_eq!(BinanceProvider::to_binance_symbol("BTC-USDT"), "BTCUSDT");
        assert_eq!(BinanceProvider::to_binance_symbol("ETH-BTC"), "ETHBTC");
    }

    #[test]
    fn test_interval_mapping() {
        assert_eq!(BinanceProvider::to_binance_interval(Interval::Daily), "1d");
        assert_eq!(BinanceProvider::to_binance_interval(Interval::Weekly), "1w");
    }

    #[test]
    fn test_date_to_ms() {
        let d = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let ms = BinanceProvider::date_to_ms(d);
        // 2025-01-01T00:00:00Z = 1735689600000 ms
        assert_eq!(ms, 1_735_689_600_000);
    }

    #[tokio::test]
    #[ignore] // requires network
    async fn test_binance_btcusdt_daily() {
        let provider = BinanceProvider::new();
        let req = OhlcvRequest {
            symbol: "BTC-USDT".into(),
            start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2025, 1, 10).unwrap(),
            interval: Interval::Daily,
        };
        let data = provider.fetch_ohlcv(&req).await.unwrap();
        assert!(!data.is_empty());
        assert_eq!(data.symbol, "BTC-USDT");
    }
}
