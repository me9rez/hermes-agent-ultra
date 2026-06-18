//! LHB via akshare datacenter APIs.

use chrono::{Duration, Utc};
use serde_json::{Value, json};

use crate::error::TradingError;
use crate::http::default_client;
use crate::providers::eastmoney_lhb::fetch_lhb_dim;
use crate::settlement::is_a_share;
use crate::symbol::normalize_symbol;

use super::{client, code6, map_err, try_or_fallback};

pub async fn fetch_lhb_dim_akshare(symbol: &str) -> Result<(Value, &'static str), TradingError> {
    let canonical = normalize_symbol(symbol);
    if !is_a_share(&canonical) {
        return Err(TradingError::SymbolNotFound(format!(
            "LHB A-share only: {symbol}"
        )));
    }
    try_or_fallback(fetch_akshare_lhb(&canonical), async {
        let http = default_client();
        let data = fetch_lhb_dim(&http, &canonical).await?;
        Ok((data, "eastmoney_lhb"))
    })
    .await
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
