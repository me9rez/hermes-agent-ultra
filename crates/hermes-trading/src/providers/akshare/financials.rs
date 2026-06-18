//! Financials via Sina abstract + analysis indicator (akshare).

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::error::TradingError;
use crate::providers::eastmoney_financials::EastmoneyFinancialsProvider;
use crate::providers::fundamentals::FundamentalsProvider;
use crate::symbol::normalize_symbol;

use super::{client, code6, map_err, sina_paper_code, try_or_fallback};

pub async fn fetch_financials_dim_akshare(
    symbol: &str,
) -> Result<(Value, &'static str), TradingError> {
    try_or_fallback(
        async {
            let code = code6(symbol)?;
            let paper = sina_paper_code(symbol)?;
            let abstract_rows = client()
                .stock_financial_abstract(&paper)
                .await
                .map_err(map_err)?;
            let indicator_rows = client()
                .stock_financial_analysis_indicator(&code, "2020")
                .await
                .map_err(map_err)?;
            Ok((
                map_financials_json(&abstract_rows, &indicator_rows),
                "akshare",
            ))
        },
        async {
            let provider = EastmoneyFinancialsProvider::new();
            let snap = provider.fetch(symbol).await?;
            Ok((snap_to_json(&snap, symbol), "eastmoney_f10"))
        },
    )
    .await
}

fn snap_to_json(snap: &crate::research::types::FundamentalsSnapshot, symbol: &str) -> Value {
    json!({
        "roe": snap.roe_latest,
        "net_margin": snap.net_margin,
        "gross_margin": snap.gross_margin,
        "revenue_growth": snap.revenue_growth_latest,
        "revenue_latest_yi": snap.revenue_latest_yi,
        "fcf_yi": snap.fcf_latest_yi,
        "fcf_positive": snap.fcf_positive,
        "equity_yi": snap.equity_yi,
        "total_debt_yi": snap.total_debt_yi,
        "cash_yi": snap.cash_yi,
        "ebitda_yi": snap.ebitda_yi,
        "roe_history": snap.roe_history,
        "revenue_history": snap.revenue_history,
        "financial_health": {
            "debt_ratio": snap.debt_ratio,
            "current_ratio": snap.current_ratio,
            "fcf_margin": snap.fcf_margin,
        },
        "symbol": normalize_symbol(symbol),
    })
}

fn map_financials_json(
    abstract_rows: &[HashMap<String, Value>],
    indicator_rows: &[HashMap<String, Value>],
) -> Value {
    let roe = metric_latest(abstract_rows, "净资产收益率")
        .or_else(|| metric_latest(indicator_rows, "净资产收益率(%)"));
    let net_margin = metric_latest(abstract_rows, "销售净利率")
        .or_else(|| metric_latest(indicator_rows, "销售净利率(%)"));
    let gross_margin = metric_latest(abstract_rows, "销售毛利率");
    let revenue_latest = metric_latest(abstract_rows, "营业总收入");
    let roe_history = metric_series(abstract_rows, "净资产收益率")
        .or_else(|| metric_series(indicator_rows, "净资产收益率(%)"));
    let revenue_history = metric_series(abstract_rows, "营业总收入");

    json!({
        "roe": roe,
        "net_margin": net_margin,
        "gross_margin": gross_margin,
        "revenue_latest_yi": revenue_latest.map(|v| v / 1e8),
        "roe_history": roe_history,
        "revenue_history": revenue_history.map(|v| v.into_iter().map(|x| x / 1e8).collect::<Vec<_>>()),
        "financial_health": {
            "debt_ratio": metric_latest(indicator_rows, "资产负债率(%)"),
            "current_ratio": metric_latest(indicator_rows, "流动比率"),
        },
    })
}

fn metric_latest(rows: &[HashMap<String, Value>], name: &str) -> Option<f64> {
    let row = rows.iter().find(|r| row_name(r) == Some(name))?;
    latest_numeric_in_row(row)
}

fn metric_series(rows: &[HashMap<String, Value>], name: &str) -> Option<Vec<f64>> {
    let row = rows.iter().find(|r| row_name(r) == Some(name))?;
    Some(numeric_series_in_row(row))
}

fn row_name(row: &HashMap<String, Value>) -> Option<&str> {
    row.get("指标")
        .or_else(|| row.get("选项"))
        .and_then(|v| v.as_str())
}

fn latest_numeric_in_row(row: &HashMap<String, Value>) -> Option<f64> {
    numeric_series_in_row(row).into_iter().next_back()
}

fn numeric_series_in_row(row: &HashMap<String, Value>) -> Vec<f64> {
    let mut pairs: Vec<(String, f64)> = row
        .iter()
        .filter(|(k, _)| *k != "指标" && *k != "选项")
        .filter_map(|(k, v)| parse_f64(v).map(|n| (k.clone(), n)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs.into_iter().map(|(_, v)| v).collect()
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
    fn map_financials_from_fixture_rows() {
        let mut row = HashMap::new();
        row.insert("指标".into(), json!("净资产收益率"));
        row.insert("2023-12-31".into(), json!(25.5));
        row.insert("2022-12-31".into(), json!(22.1));
        let out = map_financials_json(&[row], &[]);
        assert_eq!(out.get("roe").and_then(|v| v.as_f64()), Some(25.5));
        assert_eq!(
            out.get("roe_history")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(2)
        );
    }
}
