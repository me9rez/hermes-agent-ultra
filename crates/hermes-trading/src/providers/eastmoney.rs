//! 东方财富（Eastmoney）HTTP API provider for A-share (China) OHLCV data.
//!
//! Endpoint: `https://push2his.eastmoney.com/api/qt/stock/kline/get`
//! No API key required for public market data.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::error::TradingError;
use crate::http::{default_client, send_with_retry};
use crate::provider::MarketDataProvider;
use crate::types::{Interval, OhlcvData, OhlcvRequest, OhlcvRow, mark_partial};

/// Base URL for Eastmoney historical kline API.
const EASTMONEY_BASE_URL: &str = "https://push2his.eastmoney.com/api/qt/stock/kline/get";

/// Eastmoney market data provider for A-share stocks.
///
/// Symbol format: `"000001.SZ"` (深市) or `"600519.SH"` (沪市).
#[derive(Debug, Clone)]
pub struct EastmoneyProvider {
    client: reqwest::Client,
}

impl EastmoneyProvider {
    /// Create a new `EastmoneyProvider` with a default HTTP client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: default_client(),
        }
    }

    /// Create with a custom HTTP client.
    #[must_use]
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Convert user-facing symbol to Eastmoney `secid` format.
    ///
    /// - `"000001.SZ"` → `"0.000001"` (深市 / Shenzhen = market 0)
    /// - `"600519.SH"` → `"1.600519"` (沪市 / Shanghai = market 1)
    pub(crate) fn to_secid(symbol: &str) -> Result<String, TradingError> {
        let parts: Vec<&str> = symbol.split('.').collect();
        if parts.len() != 2 {
            return Err(TradingError::SymbolNotFound(format!(
                "Invalid A-share symbol format (expected XXXXXX.SZ or XXXXXX.SH): {symbol}"
            )));
        }
        let code = parts[0];
        let market = match parts[1].to_uppercase().as_str() {
            "SZ" => "0",
            "SH" => "1",
            other => {
                return Err(TradingError::SymbolNotFound(format!(
                    "Unknown market suffix '.{other}' (expected .SZ or .SH)"
                )));
            }
        };
        Ok(format!("{market}.{code}"))
    }

    /// Map [`Interval`] to Eastmoney `klt` parameter.
    fn to_klt(interval: Interval) -> &'static str {
        match interval {
            Interval::Daily => "101",
            Interval::Weekly => "102",
        }
    }
}

impl Default for EastmoneyProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Top-level JSON response from Eastmoney.
#[derive(Debug, Deserialize)]
struct EastmoneyResponse {
    data: Option<EastmoneyData>,
}

/// Nested `data` object containing the klines array.
#[derive(Debug, Deserialize)]
struct EastmoneyData {
    klines: Vec<String>,
}

/// Parse a single kline CSV string:
/// `"2025-01-02,10.50,10.80,10.90,10.30,123456,7890000"`
/// Fields: date, open, close, high, low, volume, amount
fn parse_kline(line: &str) -> Option<OhlcvRow> {
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() < 6 {
        return None;
    }
    let date = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d").ok()?;
    let open: f64 = parts[1].parse().ok()?;
    let close: f64 = parts[2].parse().ok()?;
    let high: f64 = parts[3].parse().ok()?;
    let low: f64 = parts[4].parse().ok()?;
    let volume: f64 = parts[5].parse().ok()?;
    Some(OhlcvRow {
        date,
        open,
        high,
        low,
        close,
        volume,
    })
}

#[async_trait]
impl MarketDataProvider for EastmoneyProvider {
    async fn fetch_ohlcv(&self, req: &OhlcvRequest) -> Result<OhlcvData, TradingError> {
        let secid = Self::to_secid(&req.symbol)?;
        let klt = Self::to_klt(req.interval);
        let beg = req.start.format("%Y%m%d").to_string();
        let end = req.end.format("%Y%m%d").to_string();

        debug!(
            secid = %secid,
            klt = %klt,
            beg = %beg,
            end = %end,
            "Eastmoney kline request"
        );

        let resp = send_with_retry(|| {
            self.client.get(EASTMONEY_BASE_URL).query(&[
                ("secid", secid.as_str()),
                ("fields1", "f1,f2,f3,f4,f5,f6"),
                ("fields2", "f51,f52,f53,f54,f55,f56,f57"),
                ("klt", klt),
                ("fqt", "1"),
                ("beg", beg.as_str()),
                ("end", end.as_str()),
            ])
        })
        .await?;

        if resp.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(TradingError::InvalidResponse(format!(
                "Eastmoney returned HTTP 403 for symbol {}",
                req.symbol
            )));
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(%status, body = %body, "Eastmoney API error");
            return Err(TradingError::InvalidResponse(format!(
                "Eastmoney returned HTTP {status}: {body}"
            )));
        }

        let parsed: EastmoneyResponse = resp.json().await?;

        let data = parsed.data.ok_or_else(|| {
            TradingError::InvalidResponse(format!(
                "Eastmoney returned empty data for symbol {}",
                req.symbol
            ))
        })?;

        let rows: Vec<OhlcvRow> = data
            .klines
            .iter()
            .filter_map(|line| parse_kline(line))
            .collect();

        if rows.is_empty() {
            return Err(TradingError::InvalidResponse(format!(
                "Eastmoney returned no kline rows for symbol {}",
                req.symbol
            )));
        }

        debug!(rows = rows.len(), "Eastmoney klines parsed");

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
        "eastmoney"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secid_conversion() {
        assert_eq!(
            EastmoneyProvider::to_secid("000001.SZ").unwrap(),
            "0.000001"
        );
        assert_eq!(
            EastmoneyProvider::to_secid("600519.SH").unwrap(),
            "1.600519"
        );
    }

    #[test]
    fn test_secid_invalid() {
        assert!(EastmoneyProvider::to_secid("BTCUSDT").is_err());
        assert!(EastmoneyProvider::to_secid("000001.XX").is_err());
    }

    #[test]
    fn test_klt_mapping() {
        assert_eq!(EastmoneyProvider::to_klt(Interval::Daily), "101");
        assert_eq!(EastmoneyProvider::to_klt(Interval::Weekly), "102");
    }

    #[test]
    fn test_parse_kline() {
        let line = "2025-01-02,10.50,10.80,10.90,10.30,123456,7890000";
        let row = parse_kline(line).unwrap();
        assert_eq!(row.date, NaiveDate::from_ymd_opt(2025, 1, 2).unwrap());
        assert!((row.open - 10.50).abs() < f64::EPSILON);
        assert!((row.close - 10.80).abs() < f64::EPSILON);
        assert!((row.high - 10.90).abs() < f64::EPSILON);
        assert!((row.low - 10.30).abs() < f64::EPSILON);
        assert!((row.volume - 123_456.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_kline_short() {
        assert!(parse_kline("2025-01-02,10.50").is_none());
    }

    #[tokio::test]
    #[ignore] // requires network
    async fn test_eastmoney_pingan_bank() {
        let provider = EastmoneyProvider::new();
        let req = OhlcvRequest {
            symbol: "000001.SZ".into(),
            start: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
            interval: Interval::Daily,
        };
        let data = provider.fetch_ohlcv(&req).await.unwrap();
        assert!(!data.is_empty());
        assert_eq!(data.symbol, "000001.SZ");
    }
}
