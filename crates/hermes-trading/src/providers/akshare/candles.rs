//! A-share OHLCV closes: akshare candles → eastmoney push2his.

use chrono::{Duration, NaiveDate, Utc};

use crate::error::TradingError;
use crate::provider::MarketDataProvider;
use crate::providers::EastmoneyProvider;
use crate::types::{Interval, OhlcvRequest};

use super::{client, code6, map_err, try_or_fallback};

const CANDLE_LIMIT: usize = 260;

/// Daily closes for kline stats (akshare primary).
pub async fn fetch_a_share_closes(symbol: &str) -> Result<(Vec<f64>, &'static str), TradingError> {
    try_or_fallback(
        async {
            let code = code6(symbol)?;
            let candles = client()
                .a_share_candles(&code, "qfq", CANDLE_LIMIT)
                .await
                .map_err(map_err)?;
            if candles.len() < 20 {
                return Err(TradingError::NoData);
            }
            Ok((candles.into_iter().map(|c| c.close).collect(), "akshare"))
        },
        async {
            let closes = fetch_closes_eastmoney(symbol).await?;
            Ok((closes, "eastmoney_push2his"))
        },
    )
    .await
}

async fn fetch_closes_eastmoney(symbol: &str) -> Result<Vec<f64>, TradingError> {
    let provider = EastmoneyProvider::new();
    let end = Utc::now().date_naive();
    let start = end - Duration::days(365);
    let req = OhlcvRequest {
        symbol: symbol.to_string(),
        start,
        end,
        interval: Interval::Daily,
    };
    let data = provider.fetch_ohlcv(&req).await?;
    let closes: Vec<f64> = data.rows.iter().map(|r| r.close).collect();
    if closes.len() < 20 {
        return Err(TradingError::NoData);
    }
    Ok(closes)
}

/// Convert akshare candles to OhlcvRow-compatible closes with dates (for tests).
#[allow(dead_code)]
pub(crate) fn parse_candle_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}
