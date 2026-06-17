//! Spot quote response types.

use chrono::{Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Live spot quote for a single symbol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuoteData {
    pub symbol: String,
    /// Market session date (`YYYY-MM-DD`) for the quoted price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_date: Option<String>,
    /// As-of timestamp for the quoted price (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pe_ratio: Option<f64>,
    #[serde(rename = "52w_high", skip_serializing_if = "Option::is_none")]
    pub high_52w: Option<f64>,
    #[serde(rename = "52w_low", skip_serializing_if = "Option::is_none")]
    pub low_52w: Option<f64>,
    pub source: String,
    #[serde(default)]
    pub partial: bool,
}

impl QuoteData {
    #[must_use]
    pub fn new(symbol: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            market_date: None,
            as_of: None,
            price: None,
            change: None,
            change_pct: None,
            volume: None,
            currency: None,
            exchange: None,
            short_name: None,
            pe_ratio: None,
            high_52w: None,
            low_52w: None,
            source: source.into(),
            partial: true,
        }
    }

    pub fn finalize_partial(&mut self) {
        self.partial = self.price.is_none();
        if !self.partial && self.as_of.is_none() {
            self.stamp_fetch_time();
        }
    }

    /// Set `as_of` / `market_date` from exchange unix seconds.
    pub fn set_market_timestamp_secs(&mut self, secs: i64) {
        let Some(dt) = Utc.timestamp_opt(secs, 0).single() else {
            self.stamp_fetch_time();
            return;
        };
        let local = dt.with_timezone(&Local);
        self.as_of = Some(local.to_rfc3339());
        self.market_date = Some(local.format("%Y-%m-%d").to_string());
    }

    /// Set `as_of` / `market_date` from exchange unix milliseconds.
    pub fn set_market_timestamp_millis(&mut self, millis: i64) {
        let secs = millis.div_euclid(1_000);
        let nanos = (millis.rem_euclid(1_000) * 1_000_000) as u32;
        let Some(dt) = Utc.timestamp_opt(secs, nanos).single() else {
            self.stamp_fetch_time();
            return;
        };
        let local = dt.with_timezone(&Local);
        self.as_of = Some(local.to_rfc3339());
        self.market_date = Some(local.format("%Y-%m-%d").to_string());
    }

    /// Fallback when the provider does not expose an exchange timestamp.
    pub fn stamp_fetch_time(&mut self) {
        let now = Local::now();
        self.as_of = Some(now.to_rfc3339());
        self.market_date = Some(now.format("%Y-%m-%d").to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_fetch_time_sets_date_fields() {
        let mut q = QuoteData::new("AAPL", "yahoo");
        q.price = Some(1.0);
        q.finalize_partial();
        assert!(q.market_date.is_some());
        assert!(q.as_of.is_some());
    }
}
