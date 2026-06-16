//! Mock market data provider for tests and parity fixtures.
//!
//! Returns deterministic synthetic OHLCV data without network calls.

use async_trait::async_trait;
use chrono::{Duration, NaiveDate};

use crate::error::VibeError;
use crate::provider::MarketDataProvider;
use crate::types::{Interval, OhlcvData, OhlcvRequest, OhlcvRow};

/// Mock provider that synthesizes OHLCV data for known symbols.
///
/// Use this in tests and parity fixtures to avoid hitting external APIs
/// (Binance, Eastmoney). It produces enough price oscillation to generate
/// SMA crossovers for backtest scenarios.
#[derive(Debug, Clone, Default)]
pub struct MockProvider;

impl MockProvider {
    /// Create a new mock provider.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Generate deterministic rows for the requested date range.
    fn generate_rows(symbol: &str, start: NaiveDate, end: NaiveDate, _interval: Interval) -> Vec<OhlcvRow> {
        let base_price = match symbol {
            "BTC-USDT" => 50_000.0,
            "ETH-BTC" => 0.05,
            "000001.SZ" | "600519.SH" => 100.0,
            _ => 100.0,
        };

        let mut rows = Vec::new();
        let mut date = start;
        let mut i = 0;
        while date <= end {
            let t = i as f64;
            // Smooth sine overlay on a slight upward drift produces both
            // uptrends and downtrends, triggering golden/death crosses.
            let close = base_price + 0.1 * t + base_price * 0.1 * (t * std::f64::consts::PI / 30.0).sin();
            let open = close * (1.0 + 0.005 * ((i + 1) as f64 * 0.3).sin());
            let high = close.max(open) * 1.02;
            let low = close.min(open) * 0.98;
            let volume = 1_000_000.0 + 500_000.0 * (t * 0.2).sin();

            rows.push(OhlcvRow {
                date,
                open,
                high,
                low,
                close,
                volume,
            });

            date += Duration::days(1);
            i += 1;
        }

        rows
    }
}

#[async_trait]
impl MarketDataProvider for MockProvider {
    async fn fetch_ohlcv(&self, req: &OhlcvRequest) -> Result<OhlcvData, VibeError> {
        if req.symbol == "INVALID_XYZ" {
            return Err(VibeError::SymbolNotFound(format!(
                "Symbol '{}' not found",
                req.symbol
            )));
        }

        let rows = Self::generate_rows(&req.symbol, req.start, req.end, req.interval);

        Ok(OhlcvData {
            symbol: req.symbol.clone(),
            interval: req.interval,
            rows,
        })
    }

    fn name(&self) -> &str {
        "mock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_rows_for_btc() {
        let provider = MockProvider::new();
        let req = OhlcvRequest {
            symbol: "BTC-USDT".to_string(),
            start: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 5, 10).unwrap(),
            interval: Interval::Daily,
        };
        let data = provider.fetch_ohlcv(&req).await.unwrap();
        assert_eq!(data.symbol, "BTC-USDT");
        assert_eq!(data.interval, Interval::Daily);
        assert_eq!(data.len(), 10);
    }

    #[tokio::test]
    async fn mock_returns_error_for_invalid_symbol() {
        let provider = MockProvider::new();
        let req = OhlcvRequest {
            symbol: "INVALID_XYZ".to_string(),
            start: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 5, 10).unwrap(),
            interval: Interval::Daily,
        };
        let result = provider.fetch_ohlcv(&req).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "expected 'not found' in error: {err}");
    }
}
