//! Shared Eastmoney push2 / Tencent qt HTTP layer (UZI `data_sources.py` subset).
//!
//! All A-share realtime quote and push2 endpoints must go through this module.

use reqwest::header::{CONTENT_ENCODING, REFERER};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;
use tracing::warn;

use crate::error::TradingError;
use crate::http::{BROWSER_USER_AGENT, EASTMONEY_UT, send_with_retry};
use crate::providers::eastmoney::EastmoneyProvider;
use crate::symbol::normalize_symbol;
use crate::types::Interval;

pub const EASTMONEY_QUOTE_URL: &str = "https://push2.eastmoney.com/api/qt/stock/get";
pub const EASTMONEY_KLINE_URL: &str = "https://push2his.eastmoney.com/api/qt/stock/kline/get";
pub const EASTMONEY_FFLOW_URL: &str = "https://push2.eastmoney.com/api/qt/stock/fflow/kline/get";
pub const EASTMONEY_REFERER: &str = "https://quote.eastmoney.com/";
pub const TENCENT_QUOTE_URL: &str = "https://qt.gtimg.cn/q=";
pub const TENCENT_REFERER: &str = "https://finance.qq.com/";

pub const QUOTE_FIELDS_MIN: &str = "f57,f58,f43,f169,f170,f47,f48,f60,f84,f116,f117,f162";
pub const QUOTE_FIELDS_EXTENDED: &str = "f57,f58,f43,f169,f170,f47,f116,f117,f162,f167,f184,f185";

/// Raw push2 quote fields (scaled integers ÷100 or ÷1e8 at call site).
#[derive(Debug, Clone, Default)]
pub struct Push2QuoteRaw {
    pub code: Option<String>,
    pub name: Option<String>,
    pub price_raw: Option<i64>,
    pub change_raw: Option<i64>,
    pub change_pct_raw: Option<i64>,
    pub volume: Option<i64>,
    pub market_cap_raw: Option<i64>,
    pub circulating_cap_raw: Option<i64>,
    pub pe_raw: Option<i64>,
    pub pb_raw: Option<i64>,
    pub pe_alt_raw: Option<i64>,
    pub total_shares_raw: Option<i64>,
    pub float_shares_raw: Option<i64>,
    pub source: &'static str,
}

/// Parsed Tencent qt.gtimg.cn payload (A-share).
#[derive(Debug, Clone, Default)]
pub struct TencentQtRaw {
    pub name: Option<String>,
    pub code: Option<String>,
    pub price: Option<f64>,
    pub prev_close: Option<f64>,
    pub change_pct: Option<f64>,
    pub pe_ttm: Option<f64>,
    pub pb: Option<f64>,
    pub market_cap_yi: Option<f64>,
    pub circulating_cap_yi: Option<f64>,
    pub source: &'static str,
}

/// Merged A-share snapshot after push2 → tencent fallback.
#[derive(Debug, Clone)]
pub struct AshareSnapshot {
    pub symbol: String,
    pub source: String,
    pub name: Option<String>,
    pub price: Option<f64>,
    pub change: Option<f64>,
    pub change_pct: Option<f64>,
    pub volume: Option<f64>,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
    pub market_cap_yi: Option<f64>,
    pub circulating_cap_yi: Option<f64>,
    pub shares_outstanding_yi: Option<f64>,
}

#[must_use]
pub fn scaled_price(raw: Option<i64>) -> Option<f64> {
    raw.map(|v| v as f64 / 100.0)
}

#[must_use]
pub fn scaled_pct(raw: Option<i64>) -> Option<f64> {
    raw.map(|v| v as f64 / 100.0)
}

#[must_use]
pub fn market_cap_yi(raw: Option<i64>) -> Option<f64> {
    raw.map(|v| v as f64 / 1e8)
}

#[must_use]
pub fn shares_yi(raw: Option<i64>) -> Option<f64> {
    raw.map(|v| v as f64 / 1e8)
}

/// Build a push2 quote GET with required headers and `ut`.
pub fn push2_quote_builder(client: &Client, secid: &str, fields: &str) -> RequestBuilder {
    client
        .get(EASTMONEY_QUOTE_URL)
        .header(REFERER, EASTMONEY_REFERER)
        .query(&[("secid", secid), ("fields", fields), ("ut", EASTMONEY_UT)])
}

pub fn push2_kline_builder(
    client: &Client,
    secid: &str,
    klt: &str,
    beg: &str,
    end: &str,
) -> RequestBuilder {
    client
        .get(EASTMONEY_KLINE_URL)
        .header(REFERER, EASTMONEY_REFERER)
        .query(&[
            ("secid", secid),
            ("fields1", "f1,f2,f3,f4,f5,f6"),
            ("fields2", "f51,f52,f53,f54,f55,f56,f57"),
            ("klt", klt),
            ("fqt", "1"),
            ("beg", beg),
            ("end", end),
            ("ut", EASTMONEY_UT),
        ])
}

pub fn push2_fflow_builder(client: &Client, secid: &str) -> RequestBuilder {
    client
        .get(EASTMONEY_FFLOW_URL)
        .header(REFERER, EASTMONEY_REFERER)
        .query(&[
            ("lmt", "20"),
            ("klt", "101"),
            ("secid", secid),
            ("fields1", "f1,f2,f3,f7"),
            (
                "fields2",
                "f51,f52,f53,f54,f55,f56,f57,f58,f59,f60,f61,f62,f63",
            ),
            ("ut", EASTMONEY_UT),
        ])
}

pub fn to_tencent_code(symbol: &str) -> Result<String, TradingError> {
    let parts: Vec<&str> = symbol.split('.').collect();
    if parts.len() != 2 {
        return Err(TradingError::SymbolNotFound(format!(
            "Invalid A-share symbol for Tencent quote: {symbol}"
        )));
    }
    let (code, market) = (parts[0], parts[1].to_uppercase());
    let prefix = match market.as_str() {
        "SH" => "sh",
        "SZ" => "sz",
        other => {
            return Err(TradingError::SymbolNotFound(format!(
                "Tencent quote unsupported market suffix: {other}"
            )));
        }
    };
    Ok(format!("{prefix}{code}"))
}

fn parse_f64_field(fields: &[&str], idx: usize) -> Option<f64> {
    fields.get(idx).and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { t.parse().ok() }
    })
}

/// Parse Tencent qt response body (UZI field indices for extended PE/PB/mcap).
pub fn parse_tencent_body(body: &str) -> Result<TencentQtRaw, TradingError> {
    let inner = body
        .split('"')
        .nth(1)
        .ok_or_else(|| TradingError::InvalidResponse("Tencent quote: missing payload".into()))?;
    let fields: Vec<&str> = inner.split('~').collect();
    if fields.len() < 5 {
        return Err(TradingError::InvalidResponse(
            "Tencent quote: too few fields".into(),
        ));
    }
    let price = parse_f64_field(&fields, 3)
        .filter(|&p| p > 0.0)
        .ok_or_else(|| TradingError::InvalidResponse("Tencent quote: bad price".into()))?;
    let prev_close = parse_f64_field(&fields, 4).unwrap_or(0.0);
    let change_pct = if prev_close > 0.0 {
        Some((price - prev_close) / prev_close * 100.0)
    } else {
        None
    };
    Ok(TencentQtRaw {
        name: fields.get(1).map(|s| (*s).to_string()),
        code: fields.get(2).map(|s| (*s).to_string()),
        price: Some(price),
        prev_close: Some(prev_close),
        change_pct,
        pe_ttm: parse_f64_field(&fields, 39),
        pb: parse_f64_field(&fields, 46),
        market_cap_yi: parse_f64_field(&fields, 45),
        circulating_cap_yi: parse_f64_field(&fields, 44),
        source: "tencent_qt",
    })
}

#[derive(Debug, Deserialize)]
struct Push2QuoteResponse {
    data: Option<Push2QuoteData>,
}

#[derive(Debug, Deserialize)]
struct Push2QuoteData {
    #[serde(rename = "f57")]
    code: Option<String>,
    #[serde(rename = "f58")]
    name: Option<String>,
    #[serde(rename = "f43")]
    price_raw: Option<Value>,
    #[serde(rename = "f169")]
    change_raw: Option<Value>,
    #[serde(rename = "f170")]
    change_pct_raw: Option<Value>,
    #[serde(rename = "f47")]
    volume: Option<Value>,
    #[serde(rename = "f116")]
    market_cap_raw: Option<Value>,
    #[serde(rename = "f117")]
    circulating_cap_raw: Option<Value>,
    #[serde(rename = "f162")]
    pe_raw: Option<Value>,
    #[serde(rename = "f167")]
    pb_raw: Option<Value>,
    #[serde(rename = "f184")]
    total_shares_raw: Option<Value>,
    #[serde(rename = "f185")]
    float_shares_raw: Option<Value>,
}

fn json_field_to_i64(value: Option<Value>) -> Option<i64> {
    let Value::Number(num) = value? else {
        return None;
    };
    if let Some(v) = num.as_i64() {
        return Some(v);
    }
    num.as_f64().map(|v| v.round() as i64)
}

impl Push2QuoteData {
    fn into_raw(self) -> Push2QuoteRaw {
        Push2QuoteRaw {
            code: self.code,
            name: self.name,
            price_raw: json_field_to_i64(self.price_raw),
            change_raw: json_field_to_i64(self.change_raw),
            change_pct_raw: json_field_to_i64(self.change_pct_raw),
            volume: json_field_to_i64(self.volume),
            market_cap_raw: json_field_to_i64(self.market_cap_raw),
            circulating_cap_raw: json_field_to_i64(self.circulating_cap_raw),
            pe_raw: json_field_to_i64(self.pe_raw),
            pb_raw: json_field_to_i64(self.pb_raw),
            pe_alt_raw: None,
            total_shares_raw: json_field_to_i64(self.total_shares_raw),
            float_shares_raw: json_field_to_i64(self.float_shares_raw),
            source: "eastmoney",
        }
    }
}

fn push2_has_price(raw: &Push2QuoteRaw) -> bool {
    raw.price_raw.is_some_and(|v| v > 0)
}

fn snapshot_from_push2(symbol: &str, raw: Push2QuoteRaw) -> AshareSnapshot {
    AshareSnapshot {
        symbol: symbol.to_string(),
        source: raw.source.to_string(),
        name: raw.name.clone(),
        price: scaled_price(raw.price_raw),
        change: scaled_price(raw.change_raw),
        change_pct: scaled_pct(raw.change_pct_raw),
        volume: raw.volume.map(|v| v as f64),
        pe: scaled_price(raw.pe_raw.or(raw.pe_alt_raw)),
        pb: scaled_price(raw.pb_raw),
        market_cap_yi: market_cap_yi(raw.market_cap_raw),
        circulating_cap_yi: market_cap_yi(raw.circulating_cap_raw),
        shares_outstanding_yi: shares_yi(raw.total_shares_raw),
    }
}

fn merge_tencent_into(mut snap: AshareSnapshot, qt: &TencentQtRaw) -> AshareSnapshot {
    if snap.source == "eastmoney" && qt.source == "tencent_qt" {
        snap.source = "eastmoney+tencent_qt".to_string();
    } else if snap.price.is_none() {
        snap.source = qt.source.to_string();
    }
    if snap.name.is_none() {
        snap.name.clone_from(&qt.name);
    }
    if snap.price.is_none() {
        snap.price = qt.price;
    }
    if snap.change_pct.is_none() {
        snap.change_pct = qt.change_pct;
    }
    if snap.pe.is_none() {
        snap.pe = qt.pe_ttm;
    }
    if snap.pb.is_none() {
        snap.pb = qt.pb;
    }
    if snap.market_cap_yi.is_none() {
        snap.market_cap_yi = qt.market_cap_yi;
    }
    if snap.circulating_cap_yi.is_none() {
        snap.circulating_cap_yi = qt.circulating_cap_yi;
    }
    if snap.change.is_none() {
        snap.change = qt.price.zip(qt.prev_close).map(|(p, pc)| p - pc);
    }
    snap
}

fn needs_tencent_fill(snap: &AshareSnapshot) -> bool {
    snap.price.is_none()
        || snap.name.is_none()
        || snap.pe.is_none()
        || snap.pb.is_none()
        || snap.market_cap_yi.is_none()
}

/// Fetch push2 quote JSON for one ticker.
pub async fn fetch_push2_quote(
    client: &Client,
    secid: &str,
    fields: &str,
) -> Result<Push2QuoteRaw, TradingError> {
    let client = client.clone();
    let secid = secid.to_string();
    let fields = fields.to_string();
    let resp = send_with_retry(|| push2_quote_builder(&client, &secid, &fields)).await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(TradingError::InvalidResponse(format!(
            "Eastmoney quote HTTP {status}: {body}"
        )));
    }

    let status = resp.status();
    let encoding = resp
        .headers()
        .get(CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .bytes()
        .await
        .map_err(|e| TradingError::InvalidResponse(e.to_string()))?;

    let parsed: Push2QuoteResponse = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let prefix = String::from_utf8_lossy(&body[..body.len().min(120)]);
            warn!(
                %status,
                content_encoding = %encoding,
                body_prefix = %prefix,
                error = %e,
                "eastmoney push2 JSON decode failed"
            );
            return Err(TradingError::InvalidResponse(format!(
                "Eastmoney quote JSON decode: {e}"
            )));
        }
    };
    let Some(data) = parsed.data else {
        return Err(TradingError::NoData);
    };
    let raw = data.into_raw();
    if !push2_has_price(&raw) {
        return Err(TradingError::NoData);
    }
    Ok(raw)
}

/// Fetch Tencent qt for one A-share symbol.
pub async fn fetch_tencent_qt(client: &Client, symbol: &str) -> Result<TencentQtRaw, TradingError> {
    let tencent_code = to_tencent_code(symbol)?;
    let url = format!("{TENCENT_QUOTE_URL}{tencent_code}");
    let client = client.clone();
    let resp = send_with_retry(|| {
        client
            .get(&url)
            .header(REFERER, TENCENT_REFERER)
            .header("User-Agent", BROWSER_USER_AGENT)
    })
    .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(TradingError::InvalidResponse(format!(
            "Tencent quote HTTP {status}: {body}"
        )));
    }
    let bytes = resp.bytes().await?;
    let body = crate::text_encoding::decode_tencent_qt_body(&bytes);
    parse_tencent_body(&body)
}

/// push2 extended quote → Tencent qt fallback (UZI `fetch_a_share_basic` subset).
pub async fn fetch_a_share_snapshot(
    client: &Client,
    symbol: &str,
) -> Result<AshareSnapshot, TradingError> {
    let canonical = normalize_symbol(symbol);
    let secid = EastmoneyProvider::to_secid(&canonical)?;

    let mut snap = match fetch_push2_quote(client, &secid, QUOTE_FIELDS_EXTENDED).await {
        Ok(raw) => snapshot_from_push2(&canonical, raw),
        Err(e) => {
            warn!(
                symbol = %canonical,
                error = %e,
                "eastmoney push2 snapshot failed, trying tencent qt"
            );
            AshareSnapshot {
                symbol: canonical.clone(),
                source: String::new(),
                name: None,
                price: None,
                change: None,
                change_pct: None,
                volume: None,
                pe: None,
                pb: None,
                market_cap_yi: None,
                circulating_cap_yi: None,
                shares_outstanding_yi: None,
            }
        }
    };

    if needs_tencent_fill(&snap) {
        match fetch_tencent_qt(client, &canonical).await {
            Ok(qt) => {
                snap = merge_tencent_into(snap, &qt);
            }
            Err(e) => {
                warn!(symbol = %canonical, error = %e, "tencent qt fallback failed");
                if snap.price.is_none() {
                    crate::network_preflight::log_domestic_diagnostic();
                    return Err(e);
                }
            }
        }
    }

    if snap.price.is_none() || snap.price == Some(0.0) {
        crate::network_preflight::log_domestic_diagnostic();
        return Err(TradingError::NoData);
    }
    Ok(snap)
}

/// Fetch push2his kline CSV lines.
pub async fn fetch_push2_klines(
    client: &Client,
    secid: &str,
    interval: Interval,
    beg: &str,
    end: &str,
) -> Result<Vec<String>, TradingError> {
    let klt = match interval {
        Interval::Daily => "101",
        Interval::Weekly => "102",
    };
    let client = client.clone();
    let secid = secid.to_string();
    let beg = beg.to_string();
    let end = end.to_string();
    let resp = send_with_retry(|| push2_kline_builder(&client, &secid, klt, &beg, &end)).await?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(TradingError::InvalidResponse(format!(
            "Eastmoney kline HTTP 403 for secid {secid}"
        )));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(TradingError::InvalidResponse(format!(
            "Eastmoney kline HTTP {status}: {body}"
        )));
    }

    #[derive(Debug, Deserialize)]
    struct KlineResponse {
        data: Option<KlineData>,
    }
    #[derive(Debug, Deserialize)]
    struct KlineData {
        klines: Vec<String>,
    }

    let parsed: KlineResponse = resp.json().await?;
    parsed
        .data
        .map(|d| d.klines)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| TradingError::InvalidResponse("Eastmoney kline empty".into()))
}

/// Fetch push2 capital-flow kline CSV lines.
pub async fn fetch_push2_fflow_klines(
    client: &Client,
    secid: &str,
) -> Result<Vec<String>, TradingError> {
    let client = client.clone();
    let secid = secid.to_string();
    let resp = send_with_retry(|| push2_fflow_builder(&client, &secid)).await?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }

    #[derive(Debug, Deserialize)]
    struct FlowResponse {
        data: Option<FlowData>,
    }
    #[derive(Debug, Deserialize)]
    struct FlowData {
        #[serde(default)]
        klines: Vec<String>,
    }

    let parsed: FlowResponse = resp.json().await?;
    Ok(parsed.data.map(|d| d.klines).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::default_client;

    #[test]
    fn scaled_fields() {
        assert_eq!(scaled_price(Some(1050)), Some(10.5));
        assert_eq!(market_cap_yi(Some(280_000_000_000)), Some(2800.0));
    }

    #[test]
    fn tencent_code_mapping() {
        assert_eq!(to_tencent_code("600519.SH").unwrap(), "sh600519");
        assert_eq!(to_tencent_code("000001.SZ").unwrap(), "sz000001");
    }

    #[test]
    fn parse_tencent_gbk_body() {
        let inner = b"1~ST\xB3\xA4\xD4\xB0~600525~4.95~4.90~";
        let mut body = Vec::from(b"v_sh600525=\"" as &[u8]);
        body.extend_from_slice(inner);
        body.extend_from_slice(b"\"");
        let decoded = crate::text_encoding::decode_tencent_qt_body(&body);
        let qt = parse_tencent_body(&decoded).unwrap();
        assert_eq!(qt.name.as_deref(), Some("ST长园"));
        assert_eq!(qt.price, Some(4.95));
    }

    #[test]
    fn parse_tencent_minimal() {
        let body = r#"v_sh600519="1~贵州茅台~600519~1407.04~1407.00~1409.54~""#;
        let qt = parse_tencent_body(body).unwrap();
        assert_eq!(qt.price, Some(1407.04));
        assert_eq!(qt.name.as_deref(), Some("贵州茅台"));
        assert!((qt.change_pct.unwrap() - 0.00284).abs() < 0.01);
    }

    #[test]
    fn parse_tencent_extended_fields() {
        let mut fields: Vec<String> = vec!["1".into(), "茅台".into(), "600519".into()];
        fields.extend(std::iter::repeat_n(String::new(), 44));
        fields[3] = "1224.45".into();
        fields[4] = "1240.00".into();
        fields[39] = "18.5".into();
        fields[44] = "1200.0".into();
        fields[45] = "1500.0".into();
        fields[46] = "8.2".into();
        let payload = fields.join("~");
        let body = format!(r#"v_sh600519="{payload}""#);
        let qt = parse_tencent_body(&body).unwrap();
        assert_eq!(qt.pe_ttm, Some(18.5));
        assert_eq!(qt.market_cap_yi, Some(1500.0));
        assert_eq!(qt.circulating_cap_yi, Some(1200.0));
        assert_eq!(qt.pb, Some(8.2));
    }

    #[test]
    fn push2_quote_builder_includes_ut_and_referer() {
        let client = default_client();
        let req = push2_quote_builder(&client, "1.600519", QUOTE_FIELDS_MIN)
            .build()
            .unwrap();
        let url = req.url().as_str();
        assert!(url.contains("secid=1.600519"));
        assert!(url.contains(&format!("ut={EASTMONEY_UT}")));
        assert!(url.contains("fields="));
        assert_eq!(
            req.headers().get(REFERER).and_then(|v| v.to_str().ok()),
            Some(EASTMONEY_REFERER)
        );
    }

    #[tokio::test]
    async fn fetch_chain_push2_fail_tencent_ok() {
        let tencent_body = r#"v_sh600519="1~贵州茅台~600519~1407.04~1407.00~1409.54~""#;
        let qt = parse_tencent_body(tencent_body).unwrap();
        let snap = merge_tencent_into(
            AshareSnapshot {
                symbol: "600519.SH".into(),
                source: String::new(),
                name: None,
                price: None,
                change: None,
                change_pct: None,
                volume: None,
                pe: None,
                pb: None,
                market_cap_yi: None,
                circulating_cap_yi: None,
                shares_outstanding_yi: None,
            },
            &qt,
        );
        assert_eq!(snap.price, Some(1407.04));
        assert_eq!(snap.source, "tencent_qt");
    }

    #[tokio::test]
    async fn fetch_push2_quote_decodes_gzip_body() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;
        use wiremock::matchers::{method, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let body = r#"{"data":{"f57":"600519","f58":"贵州茅台","f43":140704,"f116":2100000000000,"f162":1850,"f184":1256197800}}"#;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(body.as_bytes()).unwrap();
        let gzip_body = encoder.finish().unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("secid", "1.600519"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(gzip_body)
                    .insert_header("Content-Encoding", "gzip"),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::builder().gzip(true).build().unwrap();
        let url = format!(
            "{}/api/qt/stock/get?secid=1.600519&fields={}&ut={}",
            server.uri(),
            QUOTE_FIELDS_MIN,
            EASTMONEY_UT
        );
        let resp = client
            .get(&url)
            .header(REFERER, EASTMONEY_REFERER)
            .send()
            .await
            .unwrap();
        let parsed: Push2QuoteResponse =
            serde_json::from_slice(&resp.bytes().await.unwrap()).unwrap();
        let raw = parsed.data.unwrap().into_raw();
        assert_eq!(raw.name.as_deref(), Some("贵州茅台"));
        assert_eq!(scaled_price(raw.price_raw), Some(1407.04));
        assert_eq!(market_cap_yi(raw.market_cap_raw), Some(21_000.0));
    }

    #[test]
    fn push2_quote_data_tolerates_float_numeric_fields() {
        let body = r#"{"data":{"f57":"600519","f58":"贵州茅台","f43":140704.0,"f116":2.1e12,"f117":2100000000000.0,"f184":1256197800.5,"f185":"bad","f47":12345.6}}"#;
        let parsed: Push2QuoteResponse = serde_json::from_str(body).unwrap();
        let raw = parsed.data.unwrap().into_raw();
        assert_eq!(scaled_price(raw.price_raw), Some(1407.04));
        assert_eq!(market_cap_yi(raw.market_cap_raw), Some(21_000.0));
        assert_eq!(market_cap_yi(raw.circulating_cap_raw), Some(21_000.0));
        assert_eq!(shares_yi(raw.total_shares_raw), Some(12.56197801));
        assert_eq!(raw.float_shares_raw, None);
        assert_eq!(raw.volume, Some(12_346));
    }

    #[test]
    fn json_field_to_i64_handles_int_float_and_invalid() {
        assert_eq!(json_field_to_i64(Some(Value::from(140704))), Some(140704));
        assert_eq!(
            json_field_to_i64(Some(Value::from(2100000000000.0))),
            Some(2_100_000_000_000)
        );
        assert_eq!(json_field_to_i64(Some(Value::String("n/a".into()))), None);
        assert_eq!(json_field_to_i64(None), None);
    }

    #[tokio::test]
    #[ignore = "live network"]
    async fn live_push2_quote_600519() {
        let client = default_client();
        let raw = fetch_push2_quote(&client, "1.600519", QUOTE_FIELDS_MIN)
            .await
            .expect("push2 live");
        assert!(raw.price_raw.is_some_and(|v| v > 0));
    }

    #[tokio::test]
    #[ignore = "live network"]
    async fn live_tencent_qt_600519() {
        let client = default_client();
        let qt = fetch_tencent_qt(&client, "600519.SH")
            .await
            .expect("tencent live");
        assert!(qt.price.is_some_and(|p| p > 0.0));
    }
}
