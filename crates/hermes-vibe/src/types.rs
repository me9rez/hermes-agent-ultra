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
