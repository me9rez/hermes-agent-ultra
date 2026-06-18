//! Capital flow: HSGT + margin + akshare main flow.

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
    }

    if let Ok(hsgt) = client()
        .stock_hsgt_individual_em(&code)
        .await
        .map_err(map_err)
        && let Some(latest) = hsgt.last()
    {
        out["northbound_holding_shares"] = json!(latest.holding_shares);
        out["northbound_holding_ratio"] = json!(latest.holding_circulating_ratio);
        out["northbound_trade_date"] = json!(latest.trade_date);
    }

    if let Some(margin) = fetch_margin_for_code(&code).await {
        out["margin_fin_balance"] = json!(margin.0);
        out["margin_loan_balance"] = json!(margin.1);
    }

    if out.as_object().is_some_and(|o| !o.is_empty()) {
        Ok((out, "akshare"))
    } else {
        Err(TradingError::NoData)
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
