//! Eastmoney realtime quote API for A-shares.

use async_trait::async_trait;
use serde::Deserialize;
use tracing::debug;

use crate::error::TradingError;
use crate::http::{default_client, send_with_retry};
use crate::providers::eastmoney::EastmoneyProvider;
use crate::quote_data::QuoteData;
use crate::quote_provider::QuoteProvider;
use crate::settlement::is_a_share;
use crate::symbol::normalize_symbol;

const EASTMONEY_QUOTE_URL: &str = "https://push2.eastmoney.com/api/qt/stock/get";
const FIELDS: &str = "f57,f58,f43,f169,f170,f47,f48,f60,f84,f116,f117,f162";

/// Realtime A-share quote via Eastmoney `push2` (not historical `push2his`).
#[derive(Debug, Clone)]
pub struct EastmoneyQuoteProvider {
    client: reqwest::Client,
}

impl EastmoneyQuoteProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: default_client(),
        }
    }

    fn scaled_price(raw: Option<i64>) -> Option<f64> {
        raw.map(|v| v as f64 / 100.0)
    }

    fn scaled_change(raw: Option<i64>) -> Option<f64> {
        raw.map(|v| v as f64 / 100.0)
    }

    fn scaled_pct(raw: Option<i64>) -> Option<f64> {
        raw.map(|v| v as f64 / 100.0)
    }
}

impl Default for EastmoneyQuoteProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct EastmoneyQuoteResponse {
    data: Option<EastmoneyQuoteData>,
}

#[derive(Debug, Deserialize)]
struct EastmoneyQuoteData {
    #[serde(rename = "f57")]
    code: Option<String>,
    #[serde(rename = "f58")]
    name: Option<String>,
    #[serde(rename = "f43")]
    price_raw: Option<i64>,
    #[serde(rename = "f169")]
    change_raw: Option<i64>,
    #[serde(rename = "f170")]
    change_pct_raw: Option<i64>,
    #[serde(rename = "f47")]
    volume: Option<i64>,
    #[serde(rename = "f116")]
    pe_raw: Option<i64>,
    #[serde(rename = "f162")]
    pe_alt_raw: Option<i64>,
}

#[async_trait]
impl QuoteProvider for EastmoneyQuoteProvider {
    async fn fetch_quote(&self, symbol: &str) -> Result<QuoteData, TradingError> {
        let canonical = normalize_symbol(symbol);
        if !is_a_share(&canonical) {
            return Err(TradingError::SymbolNotFound(format!(
                "Eastmoney quote only supports A-shares: {symbol}"
            )));
        }
        let secid = EastmoneyProvider::to_secid(&canonical)?;
        debug!(%secid, "eastmoney quote fetch");

        let client = self.client.clone();
        let resp = send_with_retry(|| {
            client
                .get(EASTMONEY_QUOTE_URL)
                .query(&[("secid", secid.as_str()), ("fields", FIELDS)])
        })
        .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TradingError::InvalidResponse(format!(
                "Eastmoney quote HTTP {status}: {body}"
            )));
        }

        let parsed: EastmoneyQuoteResponse = resp.json().await?;
        let Some(data) = parsed.data else {
            return Err(TradingError::NoData);
        };

        let mut out = QuoteData::new(&canonical, "eastmoney");
        out.short_name = data.name;
        out.price = Self::scaled_price(data.price_raw);
        out.change = Self::scaled_change(data.change_raw);
        out.change_pct = Self::scaled_pct(data.change_pct_raw);
        out.volume = data.volume.map(|v| v as f64);
        out.pe_ratio = Self::scaled_price(data.pe_raw.or(data.pe_alt_raw));
        out.currency = Some("CNY".to_string());
        out.exchange = data.code.map(|c| format!("{c}.CN"));
        out.finalize_partial();
        if out.price.is_none() {
            return Err(TradingError::NoData);
        }
        Ok(out)
    }

    fn name(&self) -> &str {
        "eastmoney"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_fields() {
        assert_eq!(EastmoneyQuoteProvider::scaled_price(Some(1050)), Some(10.5));
    }
}
