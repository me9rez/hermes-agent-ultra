//! Yahoo Finance chart API for US/HK spot quotes (unofficial, no API key).

use async_trait::async_trait;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::debug;

use crate::error::TradingError;
use crate::http::{default_client, send_with_retry};
use crate::quote_data::QuoteData;
use crate::quote_provider::QuoteProvider;
use crate::symbol::{is_hk_share, is_us_share, normalize_symbol};

const YF_CHART: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const YF_CHART_ALT: &str = "https://query2.finance.yahoo.com/v8/finance/chart";
const YF_CRUMB: &str = "https://query1.finance.yahoo.com/v1/test/getcrumb";
const YF_HOME: &str = "https://finance.yahoo.com/";
const USER_AGENT_STR: &str = "Mozilla/5.0 (compatible; HermesAgent/1.0)";

/// Yahoo chart-based quote provider for US and HK symbols.
#[derive(Debug)]
pub struct YahooProvider {
    client: Client,
    crumb: Mutex<Option<String>>,
}

impl YahooProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: yahoo_client(),
            crumb: Mutex::new(None),
        }
    }

    fn yahoo_symbol(symbol: &str) -> String {
        let canonical = normalize_symbol(symbol);
        if is_us_share(&canonical) {
            return canonical
                .strip_suffix(".US")
                .unwrap_or(&canonical)
                .to_string();
        }
        canonical
    }

    fn headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STR));
        h.insert(ACCEPT, HeaderValue::from_static("application/json, */*"));
        h.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        h
    }

    async fn get_crumb(&self, force_refresh: bool) -> Option<String> {
        if !force_refresh {
            if let Some(cached) = self.crumb.lock().await.clone() {
                return Some(cached);
            }
        } else {
            *self.crumb.lock().await = None;
        }

        let headers = Self::headers();
        let _ = self
            .client
            .get(YF_HOME)
            .headers(headers.clone())
            .send()
            .await;

        let resp = self
            .client
            .get(YF_CRUMB)
            .headers(headers)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let text = resp.text().await.ok()?;
        let crumb = text.trim();
        if crumb.is_empty() {
            return None;
        }
        let crumb = crumb.to_string();
        *self.crumb.lock().await = Some(crumb.clone());
        Some(crumb)
    }

    async fn fetch_chart_at(
        &self,
        base: &str,
        yahoo_sym: &str,
        crumb: Option<&str>,
    ) -> Result<reqwest::Response, TradingError> {
        let path = format!("{base}/{yahoo_sym}");
        let client = self.client.clone();
        let headers = Self::headers();
        let crumb_owned = crumb.map(str::to_string);
        send_with_retry(|| {
            let mut req = client
                .get(&path)
                .headers(headers.clone())
                .query(&[("interval", "1d"), ("range", "1d")]);
            if let Some(ref c) = crumb_owned {
                req = req.query(&[("crumb", c.as_str())]);
            }
            req
        })
        .await
    }

    async fn fetch_chart(&self, yahoo_sym: &str) -> Result<Value, TradingError> {
        let mut last_err: Option<TradingError> = None;
        for refresh_crumb in [false, true] {
            let crumb = self.get_crumb(refresh_crumb).await;
            for base in [YF_CHART, YF_CHART_ALT] {
                let resp = match self.fetch_chart_at(base, yahoo_sym, crumb.as_deref()).await {
                    Ok(r) => r,
                    Err(e) => {
                        last_err = Some(e);
                        continue;
                    }
                };
                if resp.status().is_success() {
                    return resp.json().await.map_err(TradingError::Http);
                }
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if status == StatusCode::FORBIDDEN {
                    debug!(%yahoo_sym, %base, "yahoo chart 403, retrying with fresh crumb/host");
                    last_err = Some(TradingError::InvalidResponse(format!(
                        "Yahoo chart HTTP {status}: {body}"
                    )));
                    continue;
                }
                return Err(TradingError::InvalidResponse(format!(
                    "Yahoo chart HTTP {status}: {body}"
                )));
            }
        }
        Err(last_err
            .unwrap_or_else(|| TradingError::InvalidResponse("Yahoo chart request failed".into())))
    }

    pub(crate) fn parse_chart(symbol: &str, chart: &Value) -> QuoteData {
        let mut out = QuoteData::new(symbol, "yahoo");
        let Some(result) = chart
            .get("chart")
            .and_then(|c| c.get("result"))
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
        else {
            out.finalize_partial();
            return out;
        };

        let meta = result.get("meta").unwrap_or(&Value::Null);
        out.currency = meta
            .get("currency")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        out.exchange = meta
            .get("exchangeName")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        out.short_name = meta
            .get("shortName")
            .or_else(|| meta.get("longName"))
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let price = meta
            .get("regularMarketPrice")
            .or_else(|| meta.get("chartPreviousClose"))
            .and_then(json_f64);
        let prev = meta
            .get("previousClose")
            .or_else(|| meta.get("chartPreviousClose"))
            .and_then(json_f64);

        out.price = price;
        if let (Some(p), Some(pc)) = (price, prev) {
            let chg = p - pc;
            out.change = Some(chg);
            if pc != 0.0 {
                out.change_pct = Some(chg / pc * 100.0);
            }
        }
        out.volume = meta.get("regularMarketVolume").and_then(json_f64);
        out.high_52w = meta.get("fiftyTwoWeekHigh").and_then(json_f64);
        out.low_52w = meta.get("fiftyTwoWeekLow").and_then(json_f64);
        if let Some(ts) = meta
            .get("regularMarketTime")
            .and_then(json_i64)
            .or_else(|| {
                meta.get("regularMarketTime")
                    .and_then(json_f64)
                    .map(|v| v as i64)
            })
        {
            out.set_market_timestamp_secs(ts);
        }
        out.finalize_partial();
        out
    }
}

impl Default for YahooProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn yahoo_client() -> Client {
    Client::builder()
        .cookie_store(true)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| default_client())
}

fn json_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn json_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().map(|v| v as i64))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

#[async_trait]
impl QuoteProvider for YahooProvider {
    async fn fetch_quote(&self, symbol: &str) -> Result<QuoteData, TradingError> {
        let canonical = normalize_symbol(symbol);
        if !is_us_share(&canonical) && !is_hk_share(&canonical) {
            return Err(TradingError::SymbolNotFound(format!(
                "Yahoo quote does not support symbol '{symbol}'"
            )));
        }
        let yahoo_sym = Self::yahoo_symbol(symbol);
        debug!(%yahoo_sym, "yahoo quote fetch");
        let chart = self.fetch_chart(&yahoo_sym).await?;
        let data = Self::parse_chart(&canonical, &chart);
        if data.price.is_none() {
            return Err(TradingError::NoData);
        }
        Ok(data)
    }

    fn name(&self) -> &str {
        "yahoo"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_yahoo_chart_meta() {
        let chart = json!({
            "chart": {
                "result": [{
                    "meta": {
                        "currency": "USD",
                        "exchangeName": "NMS",
                        "shortName": "Apple Inc.",
                        "regularMarketPrice": 291.58,
                        "previousClose": 290.0,
                        "regularMarketVolume": 50000000,
                        "fiftyTwoWeekHigh": 300.0,
                        "fiftyTwoWeekLow": 200.0,
                        "regularMarketTime": 1718640000
                    }
                }]
            }
        });
        let q = YahooProvider::parse_chart("AAPL", &chart);
        assert_eq!(q.price, Some(291.58));
        assert!((q.change.unwrap() - 1.58).abs() < 1e-6);
        assert!((q.change_pct.unwrap() - 0.5448275862068966).abs() < 1e-6);
        assert!(q.market_date.is_some());
        assert!(q.as_of.is_some());
        assert!(!q.partial);
    }

    #[tokio::test]
    #[ignore = "live Yahoo network"]
    async fn live_aapl_quote() {
        let q = YahooProvider::new()
            .fetch_quote("AAPL")
            .await
            .expect("AAPL quote");
        assert!(q.price.is_some(), "expected price: {q:?}");
    }
}
