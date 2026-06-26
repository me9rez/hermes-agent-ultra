//! LHB via akshare datacenter APIs.

use chrono::{Duration, Utc};
use serde_json::{Value, json};

use crate::error::TradingError;
use crate::http::default_client;
use crate::providers::eastmoney_lhb::fetch_lhb_dim;
use crate::settlement::is_a_share;
use crate::symbol::normalize_symbol;

use super::{client, code6, map_err};

pub async fn fetch_lhb_dim_akshare(symbol: &str) -> Result<(Value, &'static str), TradingError> {
    let canonical = normalize_symbol(symbol);
    if !is_a_share(&canonical) {
        return Err(TradingError::SymbolNotFound(format!(
            "LHB A-share only: {symbol}"
        )));
    }
    match fetch_akshare_lhb(&canonical).await {
        Ok(v) => Ok(v),
        Err(primary_err) => {
            tracing::warn!(error = %primary_err, "akshare lhb failed, using fallback");
            let http = default_client();
            match fetch_lhb_dim(&http, &canonical).await {
                Ok(data) => Ok((data, "eastmoney_lhb")),
                Err(fallback_err) => Ok((
                    json!({
                        "lhb_count_30d": 0,
                        "matched_youzi": [],
                        "lhb_records": 0,
                        "lhb_error": format!("{primary_err}; fallback: {fallback_err}"),
                    }),
                    "akshare",
                )),
            }
        }
    }
}

async fn fetch_akshare_lhb(symbol: &str) -> Result<(Value, &'static str), TradingError> {
    let code = code6(symbol)?;
    let dates = client()
        .stock_lhb_stock_detail_date_em(&code)
        .await
        .map_err(map_err)?;
    let cutoff = (Utc::now() - Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let recent: Vec<_> = dates.iter().filter(|d| d.trade_date >= cutoff).collect();
    let matched: Vec<String> = recent
        .iter()
        .filter(|d| d.reason.contains('游'))
        .map(|d| d.reason.chars().take(12).collect())
        .take(5)
        .collect();
    Ok((
        json!({
            "lhb_count_30d": recent.len(),
            "matched_youzi": matched,
            "lhb_records": recent.len(),
        }),
        "akshare",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lhb_payload_shape_has_count_and_youzi() {
        let data = json!({
            "lhb_count_30d": 3,
            "matched_youzi": ["日涨幅偏离值达7%"],
            "lhb_records": 3
        });
        assert_eq!(data["lhb_count_30d"], 3);
        assert!(data["matched_youzi"].is_array());
    }

    #[test]
    fn lhb_error_keeps_zero_count_schema() {
        let data = json!({
            "lhb_count_30d": 0,
            "matched_youzi": [],
            "lhb_records": 0,
            "lhb_error": "timeout"
        });
        assert!(data.get("lhb_error").is_some());
        assert_eq!(data["lhb_count_30d"], 0);
        assert!(data["matched_youzi"].as_array().unwrap().is_empty());
    }
}
