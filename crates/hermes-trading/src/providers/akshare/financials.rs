//! Financials via Sina abstract + analysis indicator (akshare).

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::error::TradingError;
use crate::providers::eastmoney_financials::EastmoneyFinancialsProvider;
use crate::providers::fundamentals::FundamentalsProvider;
use crate::symbol::normalize_symbol;

use super::{client, code6, labels, map_err, sina_paper_code, try_or_fallback};

pub async fn fetch_financials_dim_akshare(symbol: &str) -> Result<(Value, String), TradingError> {
    let (mut data, base_source) = try_or_fallback(
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
                "akshare".to_string(),
            ))
        },
        async {
            let provider = EastmoneyFinancialsProvider::new();
            let snap = provider.fetch(symbol).await?;
            Ok((snap_to_json(&snap, symbol), "eastmoney_f10".to_string()))
        },
    )
    .await?;

    let mut source = base_source;
    if needs_f10_supplement(&data) {
        let secucode = normalize_symbol(symbol);
        if let Ok(rows) = client()
            .stock_financial_analysis_indicator_em(&secucode, "按报告期")
            .await
        {
            let sup = map_em_main_finance_json(&rows);
            data = merge_financials_missing(&data, &sup);
            if source == "akshare" {
                source = "akshare+em_datacenter".into();
            } else {
                source = format!("{source}+em_datacenter");
            }
        }
        if needs_f10_supplement(&data)
            && let Ok(snap) = EastmoneyFinancialsProvider::new().fetch(symbol).await
        {
            data = merge_financials_missing(&data, &snap_to_json(&snap, symbol));
            source = match source.as_str() {
                "akshare" => "akshare+eastmoney_f10".into(),
                "eastmoney_f10" => "eastmoney_f10".into(),
                other => format!("{other}+eastmoney_f10"),
            };
        }
    }

    Ok((data, source))
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
    let roe = metric_latest(abstract_rows, labels::financials::ROE)
        .or_else(|| metric_latest(indicator_rows, labels::financials::ROE_PCT));
    let net_margin = metric_latest(abstract_rows, labels::financials::NET_MARGIN)
        .or_else(|| metric_latest(indicator_rows, labels::financials::NET_MARGIN_PCT));
    let gross_margin = metric_latest(abstract_rows, labels::financials::GROSS_MARGIN);
    let revenue_latest = metric_latest(abstract_rows, labels::financials::REVENUE);
    let fcf_yi = metric_latest(abstract_rows, labels::financials::FCF).map(|v| v / 1e8);
    let roe_history = metric_series(abstract_rows, labels::financials::ROE)
        .or_else(|| metric_series(indicator_rows, labels::financials::ROE_PCT));
    let revenue_history = metric_series(abstract_rows, labels::financials::REVENUE);
    let debt_ratio = metric_latest(indicator_rows, labels::financials::DEBT_RATIO_PCT);
    let debt_history = metric_series(indicator_rows, labels::financials::DEBT_RATIO_PCT);

    json!({
        "roe": roe,
        "net_margin": net_margin,
        "gross_margin": gross_margin,
        "revenue_latest_yi": revenue_latest.map(|v| v / 1e8),
        "fcf_yi": fcf_yi,
        "fcf_positive": fcf_yi.is_some_and(|v| v > 0.0),
        "roe_history": roe_history,
        "revenue_history": revenue_history.map(|v| v.into_iter().map(|x| x / 1e8).collect::<Vec<_>>()),
        "debt_ratio_history": debt_history,
        "financial_health": {
            "debt_ratio": debt_ratio,
            "current_ratio": metric_latest(indicator_rows, labels::financials::CURRENT_RATIO),
        },
    })
}

/// Map Eastmoney datacenter main-fin rows (`stock_financial_analysis_indicator_em`).
fn map_em_main_finance_json(rows: &[HashMap<String, Value>]) -> Value {
    let latest = rows.iter().max_by(|a, b| {
        let da = a.get("REPORT_DATE").and_then(|v| v.as_str()).unwrap_or("");
        let db = b.get("REPORT_DATE").and_then(|v| v.as_str()).unwrap_or("");
        da.cmp(db)
    });
    let Some(row) = latest else {
        return json!({});
    };

    let f = |k: &str| row.get(k).and_then(parse_f64);
    let revenue_yi = f("TOTALOPERATEREVE").map(|v| v / 1e8);
    let net_profit_yi = f("PARENTNETPROFIT").map(|v| v / 1e8);
    let roe = f("ROEJQ");
    let net_margin = f("XSJLL").or_else(|| {
        revenue_yi.and_then(|r| net_profit_yi.map(|np| if r > 0.0 { np / r * 100.0 } else { 0.0 }))
    });
    let debt_ratio = f("ZCFZL");
    let equity_yi = f("TOTAL_EQUITY").map(|v| v / 1e8);
    let total_debt_yi = f("TOTAL_LIABILITIES").map(|v| v / 1e8);
    let cash_yi = f("MONETARYFUNDS").map(|v| v / 1e8);
    let ocf_yi = f("NETCASH_OPERATE").map(|v| v / 1e8);
    let capex_yi = f("CAPITAL_EXPENDITURE").map(|v| v.abs() / 1e8);
    let fcf_yi = match (ocf_yi, capex_yi) {
        (Some(o), Some(c)) => Some(o - c),
        (Some(o), None) => Some(o),
        _ => f("FCF").map(|v| v / 1e8),
    };
    let shares_yi = f("TOTAL_SHARE").map(shares_yi_from_total_share);
    let eps = f("EPSJB").or_else(|| f("BASIC_EPS")).or_else(|| f("EPS"));
    let bvps = f("BPS").or_else(|| f("BVPS"));
    let ebitda_yi = f("EBITDA")
        .or_else(|| f("XLRLF"))
        .map(|v| if v.abs() > 1_000_000.0 { v / 1e8 } else { v });

    json!({
        "roe": roe,
        "net_margin": net_margin,
        "gross_margin": f("XSMLL"),
        "revenue_latest_yi": revenue_yi,
        "fcf_yi": fcf_yi,
        "fcf_positive": fcf_yi.is_some_and(|v| v > 0.0),
        "equity_yi": equity_yi,
        "total_debt_yi": total_debt_yi,
        "cash_yi": cash_yi,
        "ebitda_yi": ebitda_yi,
        "eps": eps,
        "bvps": bvps,
        "shares_outstanding_yi": shares_yi,
        "financial_health": {
            "debt_ratio": debt_ratio,
            "current_ratio": f("LD"),
        },
    })
}

/// Normalize TOTAL_SHARE from EM datacenter (股 / 万股 / 亿) → 亿股.
#[must_use]
pub fn shares_yi_from_total_share(raw: f64) -> f64 {
    if raw <= 0.0 {
        return raw;
    }
    if raw > 1e8 {
        raw / 1e8
    } else if raw > 100.0 {
        raw / 10_000.0
    } else {
        raw
    }
}

/// When akshare Sina rows succeed but omit key balance/cash fields, merge Eastmoney F10 / datacenter.
fn needs_f10_supplement(data: &Value) -> bool {
    fn has_f64(v: &Value, key: &str) -> bool {
        v.get(key).and_then(|x| x.as_f64()).is_some()
    }
    !has_f64(data, "roe")
        || !has_f64(data, "fcf_yi")
        || !has_f64(data, "equity_yi")
        || !has_f64(data, "cash_yi")
        || data
            .get("financial_health")
            .and_then(|h| h.get("debt_ratio"))
            .and_then(|v| v.as_f64())
            .is_none()
}

fn merge_financials_missing(primary: &Value, supplement: &Value) -> Value {
    let mut out = primary.clone();
    for key in [
        "roe",
        "net_margin",
        "gross_margin",
        "revenue_growth",
        "revenue_latest_yi",
        "fcf_yi",
        "equity_yi",
        "total_debt_yi",
        "cash_yi",
        "ebitda_yi",
        "eps",
        "bvps",
        "shares_outstanding_yi",
    ] {
        fill_missing_f64(&mut out, supplement, key);
    }
    if out.get("fcf_positive").and_then(|v| v.as_bool()).is_none()
        && let Some(v) = supplement.get("fcf_positive").and_then(|v| v.as_bool())
    {
        out["fcf_positive"] = json!(v);
    }
    for key in ["roe_history", "revenue_history"] {
        if out
            .get(key)
            .and_then(|v| v.as_array())
            .is_none_or(|a| a.is_empty())
            && let Some(v) = supplement.get(key)
        {
            out[key] = v.clone();
        }
    }
    if let Some(sup_health) = supplement.get("financial_health") {
        let health = out.as_object_mut().and_then(|o| {
            o.entry("financial_health")
                .or_insert(json!({}))
                .as_object_mut()
        });
        if let (Some(h), Some(sh)) = (health, sup_health.as_object()) {
            for (k, v) in sh {
                if h.get(k).and_then(|x| x.as_f64()).is_none() {
                    h.insert(k.clone(), v.clone());
                }
            }
        }
    }
    out
}

fn fill_missing_f64(out: &mut Value, supplement: &Value, key: &str) {
    if out.get(key).and_then(|v| v.as_f64()).is_none()
        && let Some(v) = supplement.get(key).and_then(|v| v.as_f64())
    {
        out[key] = json!(v);
    }
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

    #[test]
    fn merge_financials_fills_missing_roe_and_fcf() {
        let partial = json!({
            "net_margin": 52.0,
            "revenue_latest_yi": 1500.0,
            "financial_health": {}
        });
        let supplement = json!({
            "roe": 32.0,
            "fcf_yi": 600.0,
            "financial_health": { "debt_ratio": 18.0 }
        });
        let merged = merge_financials_missing(&partial, &supplement);
        assert_eq!(merged.get("roe").and_then(|v| v.as_f64()), Some(32.0));
        assert_eq!(merged.get("fcf_yi").and_then(|v| v.as_f64()), Some(600.0));
        assert_eq!(
            merged
                .get("financial_health")
                .and_then(|h| h.get("debt_ratio"))
                .and_then(|v| v.as_f64()),
            Some(18.0)
        );
        assert_eq!(
            merged.get("net_margin").and_then(|v| v.as_f64()),
            Some(52.0)
        );
    }

    #[test]
    fn map_em_main_finance_includes_cashflow_and_shares() {
        let mut row = HashMap::new();
        row.insert("REPORT_DATE".into(), json!("2024-12-31"));
        row.insert("NETCASH_OPERATE".into(), json!(60_000_000_000.0));
        row.insert("TOTAL_SHARE".into(), json!(1_256_197_800.0));
        row.insert("EPSJB".into(), json!(59.5));
        row.insert("BPS".into(), json!(185.0));
        row.insert("ZCFZL".into(), json!(18.0));
        let out = map_em_main_finance_json(&[row]);
        assert_eq!(out.get("fcf_yi").and_then(|v| v.as_f64()), Some(600.0));
        assert!(
            out.get("shares_outstanding_yi")
                .and_then(|v| v.as_f64())
                .unwrap()
                > 12.0
        );
        assert_eq!(out.get("eps").and_then(|v| v.as_f64()), Some(59.5));
    }

    #[test]
    fn shares_yi_from_total_share_normalizes_raw_count() {
        assert!((shares_yi_from_total_share(1_256_197_800.0) - 12.561978).abs() < 0.001);
    }

    #[test]
    fn needs_f10_supplement_when_equity_or_cash_missing() {
        assert!(needs_f10_supplement(&json!({
            "roe": 20.0,
            "fcf_yi": 5.0,
            "financial_health": {"debt_ratio": 30.0}
        })));
    }
}
