//! A-share basic supplement via Eastmoney individual info (akshare).

use serde_json::{Value, json};

use crate::error::TradingError;
use crate::research::types::FundamentalsSnapshot;

use super::{client, code6, map_err};

/// Parsed supplement fields from `stock_individual_info_em`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BasicInfoSupplement {
    pub name: Option<String>,
    pub industry: Option<String>,
    pub market_cap_yi: Option<f64>,
    pub shares_outstanding_yi: Option<f64>,
    pub price: Option<f64>,
}

pub async fn fetch_basic_info_supplement(
    symbol: &str,
) -> Result<BasicInfoSupplement, TradingError> {
    let code = code6(symbol)?;
    let items = client()
        .stock_individual_info_em(&code)
        .await
        .map_err(map_err)?;
    Ok(map_individual_info(&items))
}

pub fn map_individual_info(items: &[impl IndividualInfoRow]) -> BasicInfoSupplement {
    let mut out = BasicInfoSupplement::default();
    for item in items {
        let label = item.info_label();
        match label {
            "股票简称" => out.name = item.info_value_str(),
            "行业" => out.industry = item.info_value_str(),
            "总市值" => out.market_cap_yi = item.info_value_f64_yi(),
            "总股本" => {
                out.shares_outstanding_yi = item
                    .info_value_str()
                    .and_then(|s| shares_outstanding_yi_from_value(&json!(s)))
                    .or_else(|| {
                        item.info_value_f64()
                            .and_then(|n| shares_outstanding_yi_from_value(&json!(n)))
                    });
            }
            "最新价" => out.price = item.info_value_f64(),
            _ => {}
        }
    }
    out
}

pub trait IndividualInfoRow {
    fn info_label(&self) -> &str;
    fn info_value_str(&self) -> Option<String>;
    fn info_value_f64(&self) -> Option<f64>;
    fn info_value_f64_yi(&self) -> Option<f64>;
}

impl IndividualInfoRow for akshare::stock::eastmoney_detail::StockInfoItem {
    fn info_label(&self) -> &str {
        self.item.as_str()
    }
    fn info_value_str(&self) -> Option<String> {
        json_str(&self.value)
    }
    fn info_value_f64(&self) -> Option<f64> {
        self.value.as_f64()
    }
    fn info_value_f64_yi(&self) -> Option<f64> {
        json_f64_yi(&self.value)
    }
}

pub fn apply_supplement(snap: &mut FundamentalsSnapshot, sup: &BasicInfoSupplement) {
    if snap.name.is_none() {
        snap.name.clone_from(&sup.name);
    }
    if snap.industry.is_none() {
        snap.industry.clone_from(&sup.industry);
    }
    if snap.market_cap_yi.is_none() {
        snap.market_cap_yi = sup.market_cap_yi;
    }
    if snap.shares_outstanding_yi.is_none() {
        snap.shares_outstanding_yi = sup.shares_outstanding_yi;
    }
    if snap.price.is_none() {
        snap.price = sup.price;
    }
}

fn json_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Eastmoney individual-info values are raw yuan; convert to 亿.
fn json_f64_yi(v: &Value) -> Option<f64> {
    let raw = v
        .as_f64()
        .or_else(|| v.as_str().and_then(|s| s.replace(',', "").parse().ok()))?;
    if raw <= 0.0 {
        return None;
    }
    Some(if raw > 1_000_000.0 { raw / 1e8 } else { raw })
}

/// Parse 总股本 / 流通股本 — handles 亿, 万股, and raw share counts.
fn shares_outstanding_yi_from_value(v: &Value) -> Option<f64> {
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if let Some(stripped) = s.strip_suffix('亿') {
            return stripped.trim().parse().ok();
        }
        if s.contains('万') {
            let n: f64 = s
                .replace("万股", "")
                .replace('万', "")
                .trim()
                .parse()
                .ok()?;
            return Some(n / 10_000.0);
        }
    }
    let raw = v
        .as_f64()
        .or_else(|| v.as_str().and_then(|s| s.replace(',', "").parse().ok()))?;
    if raw <= 0.0 {
        return None;
    }
    // Raw share count (e.g. 1_256_197_800) → 亿 shares.
    if raw > 1_000_000.0 {
        Some(raw / 1e8)
    } else if raw > 100.0 {
        // 万股-style magnitude (e.g. 125619.78 万股).
        Some(raw / 10_000.0)
    } else {
        // Already in 亿 (e.g. 12.56).
        Some(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct MockItem {
        label: &'static str,
        value: Value,
    }

    impl IndividualInfoRow for MockItem {
        fn info_label(&self) -> &str {
            self.label
        }
        fn info_value_str(&self) -> Option<String> {
            json_str(&self.value)
        }
        fn info_value_f64(&self) -> Option<f64> {
            self.value.as_f64()
        }
        fn info_value_f64_yi(&self) -> Option<f64> {
            json_f64_yi(&self.value)
        }
    }

    #[test]
    fn map_individual_info_fixture() {
        let items = [
            MockItem {
                label: "股票简称",
                value: json!("贵州茅台"),
            },
            MockItem {
                label: "行业",
                value: json!("酿酒行业"),
            },
            MockItem {
                label: "总市值",
                value: json!(2_100_000_000_000.0),
            },
            MockItem {
                label: "总股本",
                value: json!(1_256_197_800.0),
            },
        ];
        let sup = map_individual_info(&items);
        assert_eq!(sup.name.as_deref(), Some("贵州茅台"));
        assert_eq!(sup.industry.as_deref(), Some("酿酒行业"));
        assert!((sup.market_cap_yi.unwrap() - 21_000.0).abs() < 0.1);
        assert!(sup.shares_outstanding_yi.unwrap() > 10.0);
    }

    #[test]
    fn shares_outstanding_parses_wan_gu_and_yi_suffix() {
        assert!(
            (shares_outstanding_yi_from_value(&json!("12.56亿")).unwrap() - 12.56).abs() < 0.01
        );
        assert!(
            (shares_outstanding_yi_from_value(&json!("125619.78万股")).unwrap() - 12.561978).abs()
                < 0.01
        );
        assert!(
            (shares_outstanding_yi_from_value(&json!(1_256_197_800.0)).unwrap() - 12.561978).abs()
                < 0.01
        );
    }
}
