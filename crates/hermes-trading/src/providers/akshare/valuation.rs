//! Valuation percentiles via Baidu historical series (akshare) + EM datacenter fallback.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::error::TradingError;
use crate::symbol::normalize_symbol;

use super::{client, code6, map_err};

/// Compute historical percentile rank (0–100) for `current` within `series`.
#[must_use]
pub fn percentile_rank(series: &[f64], current: f64) -> Option<f64> {
    if series.is_empty() || !current.is_finite() {
        return None;
    }
    let valid: Vec<f64> = series
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if valid.is_empty() {
        return None;
    }
    let below = valid.iter().filter(|&&v| v <= current).count();
    Some(below as f64 / valid.len() as f64 * 100.0)
}

pub async fn fetch_valuation_percentiles(
    symbol: &str,
    pe_current: Option<f64>,
    pb_current: Option<f64>,
) -> Result<Value, TradingError> {
    let code = code6(symbol)?;
    let (pe_percentile, pb_percentile) =
        match fetch_baidu_percentiles(&code, pe_current, pb_current).await {
            Ok(v) => (
                v.get("pe_percentile").and_then(|x| x.as_f64()),
                v.get("pb_percentile").and_then(|x| x.as_f64()),
            ),
            Err(e) => {
                tracing::debug!(symbol = %symbol, error = %e, "baidu valuation percentile failed");
                (None, None)
            }
        };

    let pe_percentile = if pe_percentile.is_some() {
        pe_percentile
    } else {
        fetch_pe_percentile_em_fallback(symbol, pe_current).await
    };

    let pb_percentile = if pb_percentile.is_some() {
        pb_percentile
    } else {
        fetch_pb_percentile_em_fallback(symbol, pb_current).await
    };

    Ok(json!({
        "pe_percentile": pe_percentile,
        "pb_percentile": pb_percentile,
    }))
}

async fn fetch_baidu_percentiles(
    code: &str,
    pe_current: Option<f64>,
    pb_current: Option<f64>,
) -> Result<Value, TradingError> {
    let pe_series = client()
        .stock_zh_valuation_baidu(code, "市盈率(TTM)", "近五年")
        .await
        .map_err(map_err)?;
    let pb_series = client()
        .stock_zh_valuation_baidu(code, "市净率", "近五年")
        .await
        .map_err(map_err)?;

    let pe_vals: Vec<f64> = pe_series.iter().map(|p| p.value).collect();
    let pb_vals: Vec<f64> = pb_series.iter().map(|p| p.value).collect();

    Ok(json!({
        "pe_percentile": pe_current.and_then(|c| percentile_rank(&pe_vals, c)),
        "pb_percentile": pb_current.and_then(|c| percentile_rank(&pb_vals, c)),
    }))
}

/// Fallback PE percentile from Eastmoney main-fin historical rows (same datacenter as financials).
async fn fetch_pe_percentile_em_fallback(symbol: &str, pe_current: Option<f64>) -> Option<f64> {
    let pe_current = pe_current?;
    let secucode = normalize_symbol(symbol);
    let rows = client()
        .stock_financial_analysis_indicator_em(&secucode, "按报告期")
        .await
        .ok()?;
    let pe_vals = em_historical_pe_series(&rows);
    percentile_rank(&pe_vals, pe_current)
}

async fn fetch_pb_percentile_em_fallback(symbol: &str, pb_current: Option<f64>) -> Option<f64> {
    let pb_current = pb_current?;
    let secucode = normalize_symbol(symbol);
    let rows = client()
        .stock_financial_analysis_indicator_em(&secucode, "按报告期")
        .await
        .ok()?;
    let pb_vals = em_historical_metric_series(&rows, &["PB", "PB_MRQ", "SJPB"]);
    percentile_rank(&pb_vals, pb_current)
}

fn em_historical_pe_series(rows: &[HashMap<String, Value>]) -> Vec<f64> {
    em_historical_metric_series(rows, &["PE_TTM", "PE_MRQ", "SJLTTM", "DYNAMIC_PE", "PE"])
}

fn em_historical_metric_series(rows: &[HashMap<String, Value>], keys: &[&str]) -> Vec<f64> {
    let mut out = Vec::new();
    for row in rows {
        for key in keys {
            if let Some(v) = row.get(*key).and_then(parse_f64)
                && v > 0.0
                && v < 500.0
            {
                out.push(v);
                break;
            }
        }
    }
    out
}

fn parse_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.replace(',', "").parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_rank_known_series() {
        let series = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(percentile_rank(&series, 30.0), Some(60.0));
        assert_eq!(percentile_rank(&series, 5.0), Some(0.0));
        assert_eq!(percentile_rank(&series, 50.0), Some(100.0));
        assert_eq!(percentile_rank(&[], 10.0), None);
    }

    #[test]
    fn em_historical_pe_series_parses_rows() {
        let mut row = HashMap::new();
        row.insert("PE_TTM".into(), json!(25.0));
        let vals = em_historical_pe_series(&[row]);
        assert_eq!(vals, vec![25.0]);
    }
}
