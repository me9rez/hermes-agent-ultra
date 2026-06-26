//! Deterministic report content blocks for HTML / brief (content-first layout).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::research::models::dcf::DcfResult;
use crate::research::scoring::ScoreDimensionsResult;
use crate::research::types::FundamentalsSnapshot;

/// User-facing report body: fundamentals → sector → external → flows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ReportContent {
    pub fundamentals: FundamentalsBlock,
    pub sector: SectorBlock,
    pub external: ExternalBlock,
    pub flows_events: FlowsEventsBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FundamentalsBlock {
    pub bullets: Vec<String>,
    pub metrics: Vec<ContentMetric>,
    pub pe_percentile: Option<f64>,
    pub dcf_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SectorBlock {
    pub industry_name: Option<String>,
    pub growth_pct: Option<f64>,
    pub industry_pe: Option<f64>,
    pub company_pe: Option<f64>,
    pub relative_note: Option<String>,
    pub peer_rows: Vec<PeerRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PeerRow {
    pub name: String,
    pub ticker: Option<String>,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ExternalBlock {
    pub coverage: ExternalCoverage,
    pub macro_bullets: Vec<String>,
    pub policy_bullets: Vec<String>,
    pub sentiment_bullets: Vec<String>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExternalCoverage {
    #[default]
    NotRetrieved,
    HttpPartial,
    WebFilled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FlowsEventsBlock {
    pub bullets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContentMetric {
    pub label: String,
    pub value: String,
}

/// Structured overlay from web_search (slash gap-fill).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ExternalContextOverlay {
    pub macro_bullets: Vec<String>,
    pub policy_bullets: Vec<String>,
    pub sentiment_bullets: Vec<String>,
    pub sources: Vec<String>,
    /// Optional structured macro KPIs for DEEP SCAN 四宫格.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_cycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx_trend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geo_risk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commodity: Option<String>,
}

/// Build deterministic content from collected dims + models.
#[must_use]
pub fn build_report_content(
    raw_dims: &Value,
    snap: &FundamentalsSnapshot,
    comps: &Value,
    scored: &ScoreDimensionsResult,
    dcf: &DcfResult,
) -> ReportContent {
    let basic = dim_data(raw_dims, "0_basic");
    let financials = dim_data(raw_dims, "1_financials");
    let valuation = dim_data(raw_dims, "10_valuation");
    let industry = dim_data(raw_dims, "7_industry");
    let capital = dim_data(raw_dims, "12_capital_flow");
    let research = dim_data(raw_dims, "6_research");
    let lhb = dim_data(raw_dims, "16_lhb");
    let events = dim_data(raw_dims, "15_events");

    let fundamentals = build_fundamentals_block(snap, financials, valuation, dcf);
    let sector = build_sector_block(snap, industry, comps, scored);
    let external = ExternalBlock {
        coverage: ExternalCoverage::NotRetrieved,
        ..Default::default()
    };
    let flows_events = build_flows_block(capital, research, lhb, events, basic);

    ReportContent {
        fundamentals,
        sector,
        external,
        flows_events,
    }
}

/// Merge web_search overlay into cached content.
pub fn merge_external_overlay(content: &mut ReportContent, overlay: &ExternalContextOverlay) {
    let has_content = !overlay.macro_bullets.is_empty()
        || !overlay.policy_bullets.is_empty()
        || !overlay.sentiment_bullets.is_empty();
    if !has_content {
        return;
    }
    content.external.macro_bullets = overlay.macro_bullets.clone();
    content.external.policy_bullets = overlay.policy_bullets.clone();
    content.external.sentiment_bullets = overlay.sentiment_bullets.clone();
    content.external.sources = overlay
        .sources
        .iter()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .collect();
    content.external.coverage = ExternalCoverage::WebFilled;
}

/// Patch `3_macro` dim data from web overlay (四宫格 KPI).
pub fn merge_macro_dim_from_overlay(raw_dims: &mut Value, overlay: &ExternalContextOverlay) {
    let rate = overlay
        .rate_cycle
        .clone()
        .or_else(|| overlay.macro_bullets.first().cloned());
    let fx = overlay
        .fx_trend
        .clone()
        .or_else(|| overlay.macro_bullets.get(1).cloned());
    let geo = overlay
        .geo_risk
        .clone()
        .or_else(|| overlay.macro_bullets.get(2).cloned());
    let commodity = overlay
        .commodity
        .clone()
        .or_else(|| overlay.macro_bullets.get(3).cloned());
    if rate.is_none() && fx.is_none() && geo.is_none() && commodity.is_none() {
        return;
    }
    let data = serde_json::json!({
        "rate_cycle": rate.unwrap_or_else(|| "中性（货币政策）".into()),
        "fx_trend": fx.unwrap_or_else(|| "中性（人民币走势）".into()),
        "geo_risk": geo.unwrap_or_else(|| "中性（地缘风险）".into()),
        "commodity": commodity.unwrap_or_else(|| "中性（大宗周期）".into()),
    });
    let Some(obj) = raw_dims.as_object_mut() else {
        return;
    };
    let entry = obj
        .entry("3_macro")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(entry_obj) = entry.as_object_mut() {
        entry_obj.insert("data".into(), data);
    }
}

/// Whether slash should block delivery waiting for web gap-fill (low confidence only).
#[must_use]
pub fn needs_external_web_fill(content: &ReportContent, confidence: f64) -> bool {
    if content.external.coverage == ExternalCoverage::WebFilled {
        return false;
    }
    confidence < 0.55
}

fn build_fundamentals_block(
    snap: &FundamentalsSnapshot,
    financials: &Value,
    valuation: &Value,
    dcf: &DcfResult,
) -> FundamentalsBlock {
    let roe = f64_field(financials, "roe").or(snap.roe_latest);
    let net_margin = f64_field(financials, "net_margin").or(snap.net_margin);
    let debt = financials
        .get("financial_health")
        .and_then(|h| h.get("debt_ratio"))
        .and_then(|v| v.as_f64());
    let fcf = f64_field(financials, "fcf_yi").or(snap.fcf_latest_yi);
    let pe = f64_field(valuation, "pe_ttm").or(snap.pe);
    let pe_pct = f64_field(valuation, "pe_percentile").or(snap.pe_quantile_5y);

    let mut bullets = Vec::new();
    if let (Some(r), Some(m)) = (roe, net_margin) {
        bullets.push(format!("ROE {r:.1}% · 净利率 {m:.1}%"));
    } else if let Some(r) = roe {
        bullets.push(format!("ROE {r:.1}%"));
    }
    if let Some(d) = debt {
        bullets.push(format!("资产负债率 {d:.1}%"));
    }
    if let Some(f) = fcf {
        bullets.push(format!("自由现金流 {f:.1} 亿元"));
    }
    if let Some(p) = pe {
        let pct_note = pe_pct.map(|q| format!(" · 5年分位 {q:.0}%"));
        bullets.push(format!("PE {p:.1}{}", pct_note.unwrap_or_default()));
        if let Some(q) = pe_pct {
            bullets.push(pe_percentile_note(q));
        }
    }
    bullets.push(format!(
        "DCF {} · 安全边际 {:+.1}%",
        dcf.verdict, dcf.safety_margin_pct
    ));

    let mut metrics = Vec::new();
    push_metric(&mut metrics, "ROE", roe, "%");
    push_metric(&mut metrics, "净利率", net_margin, "%");
    push_metric(&mut metrics, "PE(TTM)", pe, "x");
    push_metric(&mut metrics, "PB", snap.pb, "x");
    if let Some(cap) = snap.market_cap_yi {
        metrics.push(ContentMetric {
            label: "总市值".into(),
            value: format!("{cap:.0} 亿"),
        });
    }

    FundamentalsBlock {
        bullets,
        metrics,
        pe_percentile: pe_pct,
        dcf_summary: Some(format!(
            "{} · 安全边际 {:+.1}%",
            dcf.verdict, dcf.safety_margin_pct
        )),
    }
}

fn build_sector_block(
    snap: &FundamentalsSnapshot,
    industry: &Value,
    comps: &Value,
    scored: &ScoreDimensionsResult,
) -> SectorBlock {
    let industry_name = str_field(industry, "industry")
        .or_else(|| snap.industry.clone())
        .filter(|s| s != "—" && !s.is_empty());
    let growth = f64_field(industry, "growth");
    let industry_pe = f64_field(industry, "industry_pe").or(snap.industry_pe);
    let company_pe = snap.pe.or_else(|| {
        comps
            .get("target")
            .and_then(|t| t.get("pe"))
            .and_then(|v| v.as_f64())
    });

    let relative_note = match (company_pe, industry_pe) {
        (Some(c), Some(i)) if i > 0.0 => {
            let prem = (c / i - 1.0) * 100.0;
            Some(if prem > 5.0 {
                format!("公司 PE 较行业中位数溢价 {prem:.0}%")
            } else if prem < -5.0 {
                format!("公司 PE 较行业中位数折价 {prem:.0}%")
            } else {
                "公司 PE 与行业中位数接近".into()
            })
        }
        _ => scored.dimensions.get("7_industry").map(|d| d.label.clone()),
    };

    let peer_rows = peer_rows_from_comps(comps);

    SectorBlock {
        industry_name,
        growth_pct: growth,
        industry_pe,
        company_pe,
        relative_note,
        peer_rows,
    }
}

fn build_flows_block(
    capital: &Value,
    research: &Value,
    lhb: &Value,
    events: &Value,
    basic: &Value,
) -> FlowsEventsBlock {
    let mut bullets = Vec::new();
    if let Some(net) = f64_field(capital, "main_fund_5d_net_yi") {
        bullets.push(format!("5日主力净流入 {net:+.2} 亿元"));
    }
    if let Some(count) = research.get("research_count").and_then(|v| v.as_u64()) {
        bullets.push(format!("近期待覆盖券商研报 {count} 篇"));
    }
    if let Some(count) = lhb.get("lhb_count_30d").and_then(|v| v.as_u64())
        && count > 0
    {
        bullets.push(format!("近30日龙虎榜上榜 {count} 次"));
    }
    if let Some(arr) = events.get("recent_events").and_then(|v| v.as_array())
        && !arr.is_empty()
    {
        bullets.push(format!("近期重要事件 {} 条", arr.len()));
    }
    if let Some(pct) = f64_field(basic, "change_pct") {
        bullets.push(format!("当日涨跌幅 {pct:+.2}%"));
    }
    FlowsEventsBlock { bullets }
}

fn peer_rows_from_comps(comps: &Value) -> Vec<PeerRow> {
    let Some(peers) = comps.get("peers").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    peers
        .iter()
        .take(8)
        .filter_map(|p| {
            let name = p.get("name").and_then(|v| v.as_str())?;
            Some(PeerRow {
                name: name.to_string(),
                ticker: p.get("ticker").and_then(|v| v.as_str()).map(str::to_string),
                pe: p.get("pe").and_then(|v| v.as_f64()),
                pb: p.get("pb").and_then(|v| v.as_f64()),
            })
        })
        .collect()
}

fn dim_data<'a>(raw: &'a Value, key: &str) -> &'a Value {
    raw.get(key)
        .and_then(|w| w.get("data"))
        .unwrap_or(&Value::Null)
}

fn f64_field(obj: &Value, key: &str) -> Option<f64> {
    obj.get(key).and_then(|v| v.as_f64())
}

fn str_field(obj: &Value, key: &str) -> Option<String> {
    obj.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn push_metric(metrics: &mut Vec<ContentMetric>, label: &str, val: Option<f64>, unit: &str) {
    if let Some(v) = val {
        let value = if unit == "%" {
            format!("{v:.1}%")
        } else if unit == "x" {
            format!("{v:.2}")
        } else {
            format!("{v:.2}{unit}")
        };
        metrics.push(ContentMetric {
            label: label.into(),
            value,
        });
    }
}

fn pe_percentile_note(q: f64) -> String {
    if q < 30.0 {
        format!("PE 5年分位 {q:.0}% — 相对历史偏低")
    } else if q > 70.0 {
        format!("PE 5年分位 {q:.0}% — 相对历史偏高")
    } else {
        format!("PE 5年分位 {q:.0}% — 处于历史中位")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::models::dcf::compute_dcf;
    use crate::research::profile::AnalysisProfile;
    use crate::research::scoring::score_dimensions;
    use serde_json::json;

    fn moutai_dims() -> Value {
        json!({
            "0_basic": { "data": { "name": "贵州茅台", "industry": "白酒", "price": 1680.0, "pe_ttm": 28.5, "pb": 8.2, "market_cap_yi": 21000, "change_pct": 0.5 } },
            "1_financials": { "data": { "roe": 32.0, "net_margin": 52.0, "fcf_yi": 600, "financial_health": { "debt_ratio": 18.0 } } },
            "10_valuation": { "data": { "pe_ttm": 28.5, "pe_percentile": 35.0 } },
            "7_industry": { "data": { "industry": "白酒", "growth": 12.0, "industry_pe": 22.0 } },
            "4_peers": { "data": { "peer_table": [{ "name": "五粮液", "ticker": "000858", "pe": 18.0, "pb": 4.2 }] } },
            "6_research": { "data": { "research_count": 10 } },
            "12_capital_flow": { "data": { "main_fund_5d_net_yi": 3.5 } }
        })
    }

    #[test]
    fn build_content_moutai_smoke() {
        let raw = moutai_dims();
        let snap = FundamentalsSnapshot {
            symbol: "600519.SH".into(),
            name: Some("贵州茅台".into()),
            industry: Some("白酒".into()),
            price: Some(1680.0),
            pe: Some(28.5),
            pb: Some(8.2),
            market_cap_yi: Some(21000.0),
            ..Default::default()
        };
        let comps = json!({
            "target": { "name": "贵州茅台", "pe": 28.5 },
            "peers": [{ "name": "五粮液", "ticker": "000858", "pe": 18.0, "pb": 4.2 }]
        });
        let scored = score_dimensions("600519.SH", &raw, &snap, &AnalysisProfile::medium());
        let dcf = compute_dcf(&snap, None);
        let content = build_report_content(&raw, &snap, &comps, &scored, &dcf);
        assert!(!content.fundamentals.bullets.is_empty());
        assert_eq!(content.sector.industry_name.as_deref(), Some("白酒"));
        assert_eq!(content.external.coverage, ExternalCoverage::NotRetrieved);
        assert!(
            content
                .flows_events
                .bullets
                .iter()
                .any(|b| b.contains("研报"))
        );
    }

    #[test]
    fn merge_external_sets_web_filled() {
        let mut content = ReportContent::default();
        merge_external_overlay(
            &mut content,
            &ExternalContextOverlay {
                policy_bullets: vec!["消费税政策稳定".into()],
                sources: vec!["gov.cn".into()],
                ..Default::default()
            },
        );
        assert_eq!(content.external.coverage, ExternalCoverage::WebFilled);
        assert_eq!(content.external.policy_bullets.len(), 1);
    }

    #[test]
    fn merge_macro_dim_from_overlay_sets_quad() {
        let mut raw = json!({});
        merge_macro_dim_from_overlay(
            &mut raw,
            &ExternalContextOverlay {
                rate_cycle: Some("宽松".into()),
                fx_trend: Some("偏弱".into()),
                geo_risk: Some("可控".into()),
                commodity: Some("底部".into()),
                ..Default::default()
            },
        );
        let data = raw["3_macro"]["data"].as_object().unwrap();
        assert_eq!(data["rate_cycle"], "宽松");
        assert_eq!(data["commodity"], "底部");
    }

    #[test]
    fn needs_external_web_fill_when_low_confidence() {
        let content = ReportContent::default();
        assert!(needs_external_web_fill(&content, 0.40));
        assert!(!needs_external_web_fill(&content, 0.75));
    }
}
