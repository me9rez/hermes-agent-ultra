//! Core data types for market data and backtesting.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// OHLCV data request parameters.
#[derive(Debug, Clone)]
pub struct OhlcvRequest {
    /// Symbol identifier, e.g. "BTC-USDT", "000001.SZ", "600519.SH"
    pub symbol: String,
    /// Start date (inclusive)
    pub start: NaiveDate,
    /// End date (inclusive)
    pub end: NaiveDate,
    /// Data interval
    pub interval: Interval,
}

/// Data interval for market data queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interval {
    Daily,
    Weekly,
}

/// A single OHLCV row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OhlcvRow {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Collection of OHLCV data returned by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OhlcvData {
    pub symbol: String,
    pub interval: Interval,
    pub rows: Vec<OhlcvRow>,
    /// True when returned rows do not fully cover the requested date range.
    #[serde(default)]
    pub partial: bool,
}

/// Mark `partial` when first/last row dates do not cover the request window.
pub fn mark_partial(data: &mut OhlcvData, req: &OhlcvRequest) {
    if data.rows.is_empty() {
        return;
    }
    let first = data.rows.first().unwrap().date;
    let last = data.rows.last().unwrap().date;
    data.partial = first > req.start || last < req.end;
}

impl OhlcvData {
    /// Returns true if the data contains no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Returns the number of data rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn mark_partial_when_range_incomplete() {
        let req = OhlcvRequest {
            symbol: "BTC-USDT".into(),
            start: NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 5, 10).unwrap(),
            interval: Interval::Daily,
        };
        let mut data = OhlcvData {
            symbol: "BTC-USDT".into(),
            interval: Interval::Daily,
            rows: vec![OhlcvRow {
                date: NaiveDate::from_ymd_opt(2026, 5, 3).unwrap(),
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 1.0,
                volume: 1.0,
            }],
            partial: false,
        };
        mark_partial(&mut data, &req);
        assert!(data.partial);
    }
}
