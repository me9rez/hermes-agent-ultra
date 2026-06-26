//! 01 / CORE · one-shot conclusion bento (UZI dashboard parity, rule-based).

use serde_json::Value;

use crate::research::report::content::ReportContent;
use crate::research::report::identity::ReportIdentity;
use crate::research::report::sections::util::escape_html;
use crate::research::synthesis::SynthesisReport;

#[must_use]
pub fn render_core_section(
    identity: &ReportIdentity,
    syn: &SynthesisReport,
    content: &ReportContent,
    raw_dims: &Value,
) -> String {
    let conclusion = core_conclusion_text(syn);
    let trend = map_trend(raw_dims);
    let price = map_price(identity, raw_dims);
    let volume = map_volume(raw_dims, &content.flows_events.bullets);
    let chips = map_chips(raw_dims);
    let news = map_intel_line(&content.flows_events.bullets, 0).unwrap_or_else(|| "—".into());
    let risks = map_risks(syn);
    let catalysts = map_intel_line(&content.flows_events.bullets, 1)
        .or_else(|| map_catalyst_fallback(syn))
        .unwrap_or_else(|| "—".into());

    let metrics = render_key_metrics(syn);

    let cell_trend = data_cell("📈", "趋势 TREND", &trend);
    let cell_price = data_cell("💰", "价位 PRICE", &price);
    let cell_volume = data_cell("📊", "量能 VOLUME", &volume);
    let cell_chips = data_cell("🎯", "筹码 CHIPS", &chips);
    let cell_news = format!(
        r#"<div class="data-cell span-2"><div class="icon">📰</div><div class="key">新闻 NEWS</div><div class="value">{}</div></div>"#,
        escape_html(&news)
    );
    let cell_risks = data_cell("⚠️", "风险 RISKS", &risks);
    let cell_catalysts = data_cell("🚀", "催化 CATALYSTS", &catalysts);

    format!(
        r#"<section class="card" id="section-core">
<div class="section-head">
<div class="section-tag">01 / CORE</div>
<h2 class="section-title">核心结论</h2>
<div class="section-line"></div>
</div>
<div class="dashboard-bento">
<div class="core-conclusion">
<div class="label">// ONE-SHOT CONCLUSION</div>
<div class="text">{conclusion}</div>
</div>
{cell_trend}{cell_price}{cell_volume}{cell_chips}
{cell_news}
{cell_risks}{cell_catalysts}
{metrics}
</div>
</section>"#,
        conclusion = escape_html(&conclusion),
        cell_trend = cell_trend,
        cell_price = cell_price,
        cell_volume = cell_volume,
        cell_chips = cell_chips,
        cell_news = cell_news,
        cell_risks = cell_risks,
        cell_catalysts = cell_catalysts,
        metrics = metrics,
    )
}

fn core_conclusion_text(syn: &SynthesisReport) -> String {
    if syn.dcf_one_liner.is_empty() {
        syn.headline.clone()
    } else {
        format!("{}\n{}", syn.headline, syn.dcf_one_liner)
    }
}

fn data_cell(icon: &str, key: &str, value: &str) -> String {
    format!(
        r#"<div class="data-cell"><div class="icon">{icon}</div><div class="key">{}</div><div class="value">{}</div></div>"#,
        escape_html(key),
        escape_html(value),
    )
}

fn render_key_metrics(syn: &SynthesisReport) -> String {
    if syn.key_metrics.is_empty() {
        return String::new();
    }
    let cells: String = syn
        .key_metrics
        .iter()
        .take(6)
        .map(|m| {
            format!(
                r#"<div class="core-metric"><div class="k">{}</div><div class="v">{}</div></div>"#,
                escape_html(&m.label),
                escape_html(&m.value),
            )
        })
        .collect();
    format!(r#"<div class="core-metrics">{cells}</div>"#)
}

fn map_trend(raw_dims: &Value) -> String {
    let kline = dim_data(raw_dims, "2_kline");
    let stage = str_field(kline, "stage");
    let align = str_field(kline, "ma_align");
    match (stage, align) {
        (Some(s), Some(a)) if !s.is_empty() && !a.is_empty() => format!("{s} · {a}"),
        (Some(s), _) if !s.is_empty() => s,
        (_, Some(a)) if !a.is_empty() => a,
        _ => "—".into(),
    }
}

fn map_price(identity: &ReportIdentity, raw_dims: &Value) -> String {
    let price = identity
        .price
        .or_else(|| f64_field(dim_data(raw_dims, "0_basic"), "price"));
    let change = identity
        .change_pct
        .or_else(|| f64_field(dim_data(raw_dims, "0_basic"), "change_pct"));
    match (price, change) {
        (Some(p), Some(c)) => format!("¥{p:.2} ({c:+.2}%)"),
        (Some(p), None) => format!("¥{p:.2}"),
        _ => "—".into(),
    }
}

fn map_volume(raw_dims: &Value, flow_bullets: &[String]) -> String {
    if let Some(net) = f64_field(dim_data(raw_dims, "12_capital_flow"), "main_fund_5d_net_yi") {
        return format!("5日主力 {net:+.2} 亿");
    }
    flow_bullets
        .iter()
        .find(|b| b.contains("主力") || b.contains("净流入"))
        .cloned()
        .unwrap_or_else(|| "—".into())
}

fn map_chips(raw_dims: &Value) -> String {
    let holders = dim_data(raw_dims, "6_fund_holders");
    let chg = f64_field(holders, "holder_change_ratio");
    let cnt = holders
        .get("holder_count")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            holders
                .get("holder_count")
                .and_then(|v| v.as_f64())
                .map(|f| f as u64)
        });
    match (chg, cnt) {
        (Some(h), Some(c)) => format!("户数 {c} · 变化 {h:+.1}%"),
        (Some(h), None) => format!("户数变化 {h:+.1}%"),
        (None, Some(c)) => format!("股东户数 {c}"),
        _ => "—".into(),
    }
}

fn map_intel_line(bullets: &[String], index: usize) -> Option<String> {
    bullets.get(index).cloned()
}

fn map_risks(syn: &SynthesisReport) -> String {
    if syn.risks.is_empty() {
        return "—".into();
    }
    syn.risks
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn map_catalyst_fallback(syn: &SynthesisReport) -> Option<String> {
    if syn.panel_summary.vote_buy == 0 {
        return None;
    }
    Some(format!(
        "评委买入 {} 票 · 共识 {:.1}/10",
        syn.panel_summary.vote_buy, syn.panel_summary.consensus
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::report::content::{FlowsEventsBlock, ReportContent};
    use crate::research::synthesis::{KeyMetric, PanelSummary, SynthesisReport};

    fn stub_syn() -> SynthesisReport {
        SynthesisReport {
            headline: "600519.SH · 深度分析：偏多".into(),
            verdict: "buy".into(),
            confidence_tier: "high".into(),
            key_metrics: vec![KeyMetric {
                label: "Alpha".into(),
                value: "72.5/100".into(),
            }],
            risks: vec!["估值偏高".into(), "宏观波动".into()],
            missing_highlights: vec![],
            panel_summary: PanelSummary {
                consensus: 7.5,
                vote_buy: 40,
                vote_avoid: 5,
                investor_count: 66,
            },
            dcf_one_liner: "🟡 略微低估 · 安全边际 +12.0%".into(),
        }
    }

    #[test]
    fn core_section_renders_bento() {
        let identity = ReportIdentity {
            company_name: Some("贵州茅台".into()),
            symbol: "600519.SH".into(),
            price: Some(1680.0),
            change_pct: Some(1.2),
            industry: Some("白酒".into()),
            market_cap_yi: None,
            pe: Some(28.5),
            pb: None,
            fundamental_score: Some(72.5),
        };
        let raw = serde_json::json!({
            "2_kline": { "data": { "stage": "Stage 2 上升", "ma_align": "多头排列" } },
            "12_capital_flow": { "data": { "main_fund_5d_net_yi": 3.5 } },
            "6_fund_holders": { "data": { "holder_change_ratio": -8.0, "holder_count": 95000 } }
        });
        let content = ReportContent {
            flows_events: FlowsEventsBlock {
                bullets: vec![
                    "5日主力净流入 +3.5 亿元".into(),
                    "近期待覆盖券商研报 10 篇".into(),
                ],
            },
            ..Default::default()
        };
        let html = render_core_section(&identity, &stub_syn(), &content, &raw);
        assert!(html.contains("01 / CORE"));
        assert!(html.contains("核心结论"));
        assert!(html.contains("ONE-SHOT CONCLUSION"));
        assert!(html.contains("dashboard-bento"));
        assert!(html.contains("趋势 TREND"));
        assert!(html.contains("Stage 2"));
        assert!(html.contains("¥1680.00"));
        assert!(html.contains("估值偏高"));
        assert!(html.contains("Alpha"));
    }
}
