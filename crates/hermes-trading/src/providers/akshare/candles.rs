//! A-share OHLCV: akshare candles → eastmoney push2his.

use chrono::{Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::error::TradingError;
use crate::provider::MarketDataProvider;
use crate::providers::EastmoneyProvider;
use crate::types::{Interval, OhlcvRequest};

use super::{client, code6, map_err, try_or_fallback};

const CANDLE_LIMIT: usize = 260;
pub const CHART_CANDLE_COUNT: usize = 45;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct OhlcBar {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// Daily closes for kline stats (akshare primary).
pub async fn fetch_a_share_closes(symbol: &str) -> Result<(Vec<f64>, &'static str), TradingError> {
    let (bars, source) = fetch_a_share_ohlc_bars(symbol, CANDLE_LIMIT).await?;
    Ok((bars.into_iter().map(|b| b.close).collect(), source))
}

/// OHLC bars for kline stats + DEEP SCAN candlestick chart.
pub async fn fetch_a_share_ohlc_bars(
    symbol: &str,
    limit: usize,
) -> Result<(Vec<OhlcBar>, &'static str), TradingError> {
    let take = limit.max(20);
    try_or_fallback(
        async {
            let code = code6(symbol)?;
            let candles = client()
                .a_share_candles(&code, "qfq", take)
                .await
                .map_err(map_err)?;
            if candles.len() < 20 {
                return Err(TradingError::NoData);
            }
            Ok((
                candles
                    .into_iter()
                    .map(|c| OhlcBar {
                        open: c.open,
                        high: c.high,
                        low: c.low,
                        close: c.close,
                    })
                    .collect(),
                "akshare",
            ))
        },
        async {
            let bars = fetch_ohlc_eastmoney(symbol, take).await?;
            Ok((bars, "eastmoney_push2his"))
        },
    )
    .await
}

async fn fetch_ohlc_eastmoney(symbol: &str, limit: usize) -> Result<Vec<OhlcBar>, TradingError> {
    let provider = EastmoneyProvider::new();
    let end = Utc::now().date_naive();
    let start = end - Duration::days(400);
    let req = OhlcvRequest {
        symbol: symbol.to_string(),
        start,
        end,
        interval: Interval::Daily,
    };
    let data = provider.fetch_ohlcv(&req).await?;
    let bars: Vec<OhlcBar> = data
        .rows
        .iter()
        .map(|r| OhlcBar {
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
        })
        .collect();
    if bars.len() < 20 {
        return Err(TradingError::NoData);
    }
    let start_idx = bars.len().saturating_sub(limit);
    Ok(bars[start_idx..].to_vec())
}

/// Convert akshare candles to OhlcvRow-compatible closes with dates (for tests).
#[allow(dead_code)]
pub(crate) fn parse_candle_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}
