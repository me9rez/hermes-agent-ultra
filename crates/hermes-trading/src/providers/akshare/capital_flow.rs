//! Capital flow: HSGT + margin + main flow + holder trend (UZI 9-path subset).

use chrono::Utc;
use serde_json::{Value, json};

use crate::error::TradingError;
use crate::http::default_client;
use crate::providers::eastmoney_capital_flow::fetch_capital_flow_dim;
use crate::settlement::is_a_share;
use crate::symbol::normalize_symbol;

use super::{client, code6, map_err, try_or_fallback};

pub async fn fetch_capital_flow_dim_akshare(
    symbol: &str,
) -> Result<(Value, &'static str), TradingError> {
    let canonical = normalize_symbol(symbol);
    if !is_a_share(&canonical) {
        return Err(TradingError::SymbolNotFound(format!(
            "Capital flow A-share only: {symbol}"
        )));
    }
    try_or_fallback(fetch_akshare_inner(&canonical), async {
        let client = default_client();
        let data = fetch_capital_flow_dim(&client, &canonical).await?;
        Ok((data, "eastmoney_fflow"))
    })
    .await
}

async fn fetch_akshare_inner(symbol: &str) -> Result<(Value, &'static str), TradingError> {
    let code = code6(symbol)?;
    let mut out = json!({});

    if let Ok(flows) = client()
        .a_share_capital_flow(&code, 20)
        .await
        .map_err(map_err)
    {
        let sum_5: f64 = flows.iter().rev().take(5).map(|p| p.main_net_inflow).sum();
        let sum_20: f64 = flows.iter().map(|p| p.main_net_inflow).sum();
        out["main_fund_5d_net_yi"] = json!(sum_5 / 1e8);
        out["main_fund_20d_net_yi"] = json!(sum_20 / 1e8);
        let daily: Vec<Value> = flows
            .iter()
            .rev()
            .take(5)
            .map(|p| {
                json!({
                    "date": p.trade_date,
                    "main_net_inflow": p.main_net_inflow,
                })
            })
            .collect();
        out["main_fund_flow_daily"] = json!(daily);
    }

    if let Ok(hsgt) = client()
        .stock_hsgt_individual_em(&code)
        .await
        .map_err(map_err)
    {
        if let Some(latest) = hsgt.last() {
            out["northbound_holding_shares"] = json!(latest.holding_shares);
            out["northbound_holding_ratio"] = json!(latest.holding_circulating_ratio);
            out["northbound_trade_date"] = json!(latest.trade_date);
        }
        if hsgt.len() >= 2 {
            let window: Vec<f64> = hsgt
                .iter()
                .rev()
                .take(20)
                .map(|r| r.holding_shares)
                .collect();
            if let (Some(&newest), Some(&oldest)) = (window.first(), window.last()) {
                out["northbound_20d_net_shares"] = json!(newest - oldest);
            }
        }
    }

    if let Some(margin) = fetch_margin_for_code(&code).await {
        out["margin_fin_balance"] = json!(margin.0);
        out["margin_loan_balance"] = json!(margin.1);
    }

    if let Ok(gdhs) = client()
        .stock_zh_a_gdhs_detail_em(&code)
        .await
        .map_err(map_err)
    {
        let history: Vec<Value> = gdhs
            .iter()
            .rev()
            .take(8)
            .map(|d| {
                json!({
                    "end_date": d.end_date,
                    "holder_count": d.holder_count,
                    "holder_change_ratio": d.holder_change_ratio,
                })
            })
            .collect();
        if !history.is_empty() {
            out["holder_count_history"] = json!(history);
            if let Some(latest) = gdhs.iter().max_by(|a, b| a.end_date.cmp(&b.end_date)) {
                out["holder_change_ratio"] = json!(latest.holder_change_ratio);
                let counts: Vec<(String, f64)> = gdhs
                    .iter()
                    .map(|d| (d.end_date.clone(), d.holder_count))
                    .collect();
                out["holders_trend"] = json!(holders_trend_label(&counts));
            }
        }
    }

    if out.as_object().is_some_and(|o| !o.is_empty()) {
        Ok((out, "akshare"))
    } else {
        Err(TradingError::NoData)
    }
}

fn holders_trend_label(rows: &[(String, f64)]) -> &'static str {
    if rows.len() < 2 {
        return "—";
    }
    let latest = rows.iter().max_by(|a, b| a.0.cmp(&b.0));
    let oldest = rows.iter().min_by(|a, b| a.0.cmp(&b.0));
    let (Some(l), Some(p)) = (latest, oldest) else {
        return "—";
    };
    if l.1 < p.1 * 0.95 {
        "户数连降"
    } else if l.1 > p.1 * 1.05 {
        "户数连升"
    } else {
        "基本持平"
    }
}

async fn fetch_margin_for_code(code: &str) -> Option<(f64, f64)> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    if code.starts_with('6') {
        let rows = client().stock_margin_detail_sse(&today).await.ok()?;
        let row = rows.into_iter().find(|r| r.code == code)?;
        return Some((row.fin_balance, row.loan_volume));
    }
    let rows = client().stock_margin_detail_szse(&today).await.ok()?;
    let row = rows.into_iter().find(|r| r.code == code)?;
    Some((row.fin_balance, row.loan_balance))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holders_trend_detects_decline() {
        let rows = vec![
            ("2024-06-30".into(), 100_000.0),
            ("2024-03-31".into(), 120_000.0),
        ];
        assert_eq!(holders_trend_label(&rows), "户数连降");
    }
}
