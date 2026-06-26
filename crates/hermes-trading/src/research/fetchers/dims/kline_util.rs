//! K-line technical stats for dimension 2.

use serde_json::json;

use crate::error::TradingError;
use crate::indicators::{rsi, sma};
use crate::providers::akshare::{CHART_CANDLE_COUNT, OhlcBar, fetch_a_share_ohlc_bars};

#[derive(Debug, Clone)]
pub struct KlineStats {
    pub stage: String,
    pub ma_align: String,
    pub max_drawdown: f64,
    pub ma5: Option<f64>,
    pub ma20: Option<f64>,
    pub ma60: Option<f64>,
    pub rsi14: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct KlineFetchResult {
    pub stats: KlineStats,
    pub recent_candles: Vec<OhlcBar>,
}

/// Compute UZI-shaped kline dim from OHLCV (akshare primary, eastmoney fallback).
pub async fn compute_kline_stats(
    symbol: &str,
) -> Result<(KlineFetchResult, &'static str), TradingError> {
    let (bars, source) = fetch_a_share_ohlc_bars(symbol, 260).await?;
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();

    if closes.len() < 20 {
        return Err(TradingError::NoData);
    }

    let ma5 = sma(&closes, 5).last().and_then(|x| *x);
    let ma20 = sma(&closes, 20).last().and_then(|x| *x);
    let ma60 = sma(&closes, 60.min(closes.len())).last().and_then(|x| *x);
    let price = *closes.last().unwrap_or(&0.0);
    let rsi14 = rsi(&closes, 14).last().and_then(|x| *x);

    let recent_start = bars.len().saturating_sub(CHART_CANDLE_COUNT);
    let recent_candles = bars[recent_start..].to_vec();

    Ok((
        KlineFetchResult {
            stats: KlineStats {
                stage: classify_stage(price, ma20, ma60),
                ma_align: classify_ma_align(ma5, ma20, ma60),
                max_drawdown: max_drawdown_pct(&closes),
                ma5,
                ma20,
                ma60,
                rsi14,
            },
            recent_candles,
        },
        source,
    ))
}

#[must_use]
pub fn recent_candles_json(candles: &[OhlcBar]) -> serde_json::Value {
    serde_json::Value::Array(
        candles
            .iter()
            .map(|c| {
                json!({
                    "o": c.open,
                    "h": c.high,
                    "l": c.low,
                    "c": c.close,
                })
            })
            .collect(),
    )
}

fn classify_stage(price: f64, ma20: Option<f64>, ma60: Option<f64>) -> String {
    match (ma20, ma60) {
        (Some(m20), Some(m60)) if price > m20 && m20 > m60 => "Stage 2 上升".into(),
        (Some(m20), Some(m60)) if price < m20 && m20 < m60 => "Stage 4 下跌".into(),
        (Some(_m20), Some(m60)) if price > m60 => "Stage 1 筑底".into(),
        (Some(_), Some(_)) => "Stage 3 整理".into(),
        _ => String::new(),
    }
}

fn classify_ma_align(ma5: Option<f64>, ma20: Option<f64>, ma60: Option<f64>) -> String {
    match (ma5, ma20, ma60) {
        (Some(a), Some(b), Some(c)) if a > b && b > c => "多头排列".into(),
        (Some(a), Some(b), Some(c)) if a < b && b < c => "空头排列".into(),
        _ => "纠缠".into(),
    }
}

fn max_drawdown_pct(closes: &[f64]) -> f64 {
    let mut peak = closes.first().copied().unwrap_or(0.0);
    let mut max_dd = 0.0_f64;
    for &p in closes {
        if p > peak {
            peak = p;
        }
        if peak > 0.0 {
            let dd = (p - peak) / peak * 100.0;
            max_dd = max_dd.min(dd);
        }
    }
    max_dd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_and_drawdown() {
        assert!(classify_stage(110.0, Some(100.0), Some(90.0)).contains("Stage 2"));
        let closes = vec![100.0, 120.0, 90.0, 95.0];
        assert!(max_drawdown_pct(&closes) <= -20.0);
    }

    #[test]
    fn recent_candles_json_shape() {
        let arr = recent_candles_json(&[OhlcBar {
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
        }]);
        assert!(arr.as_array().unwrap()[0].get("c").is_some());
    }
}
