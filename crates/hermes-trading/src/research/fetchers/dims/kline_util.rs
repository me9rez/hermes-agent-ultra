//! K-line technical stats for dimension 2.

use crate::error::TradingError;
use crate::indicators::{rsi, sma};
use crate::providers::akshare::fetch_a_share_closes;

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

/// Compute UZI-shaped kline dim from OHLCV (akshare primary, eastmoney fallback).
pub async fn compute_kline_stats(symbol: &str) -> Result<(KlineStats, &'static str), TradingError> {
    let (closes, source) = fetch_a_share_closes(symbol).await?;

    if closes.len() < 20 {
        return Err(TradingError::NoData);
    }

    let ma5 = sma(&closes, 5).last().and_then(|x| *x);
    let ma20 = sma(&closes, 20).last().and_then(|x| *x);
    let ma60 = sma(&closes, 60.min(closes.len())).last().and_then(|x| *x);
    let price = *closes.last().unwrap_or(&0.0);
    let rsi14 = rsi(&closes, 14).last().and_then(|x| *x);

    Ok((
        KlineStats {
            stage: classify_stage(price, ma20, ma60),
            ma_align: classify_ma_align(ma5, ma20, ma60),
            max_drawdown: max_drawdown_pct(&closes),
            ma5,
            ma20,
            ma60,
            rsi14,
        },
        source,
    ))
}

fn classify_stage(price: f64, ma20: Option<f64>, ma60: Option<f64>) -> String {
    match (ma20, ma60) {
        (Some(m20), Some(m60)) if price > m20 && m20 > m60 => "Stage 2 uptrend".into(),
        (Some(m20), Some(m60)) if price < m20 && m20 < m60 => "Stage 4 downtrend".into(),
        (Some(_m20), Some(m60)) if price > m60 => "Stage 1 base".into(),
        (Some(_), Some(_)) => "Stage 3 distribution".into(),
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
}
