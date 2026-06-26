//! Institutional standalone HTML report (content-first layout).

use crate::research::analyze::AnalyzeStockResult;
use crate::research::report::sections::escape_html;
use crate::research::report::sections::{
    render_appendix, render_dimensions_section, render_external_section, render_flows_section,
    render_fundamentals_section, render_panel_section, render_sector_section, render_shell_start,
    render_warn_banner,
};
use crate::research::report_filter::show_gaps_section;
use crate::research::scoring::{PanelResult, ScoreDimensionsResult};

const CONFIDENCE_WARN_THRESHOLD: f64 = 0.55;
pub const MAX_HTML_BYTES: usize = 150_000;

/// Render institutional HTML from a completed analysis (uses embedded `synthesis` + `content`).
#[must_use]
pub fn render_institutional_html(result: &AnalyzeStockResult, narrative: Option<&str>) -> String {
    let mut result = result.clone();
    crate::research::report_filter::scrub_user_report(&mut result);
    let syn = &result.synthesis;
    let scored: ScoreDimensionsResult =
        serde_json::from_value(result.scores.clone()).unwrap_or(ScoreDimensionsResult {
            ticker: result.symbol.clone(),
            fundamental_score: 0.0,
            dimensions: Default::default(),
        });
    let panel: PanelResult =
        serde_json::from_value(result.personas.clone()).unwrap_or(PanelResult {
            investors: Vec::new(),
            vote_distribution: Default::default(),
            signal_distribution: Default::default(),
            panel_consensus: scored.fundamental_score,
        });

    let identity = crate::research::report::ReportIdentity::from_analyze_result(&result);

    let mut html = render_shell_start(&identity, syn);
    if result.data_confidence.score < CONFIDENCE_WARN_THRESHOLD {
        html.push_str(&render_warn_banner(result.data_confidence.score));
    }
    html.push_str(&render_fundamentals_section(&result.content.fundamentals));
    html.push_str(&render_sector_section(&result.content.sector));
    html.push_str(&render_external_section(&result.content.external));
    html.push_str(&render_flows_section(&result.content.flows_events));
    html.push_str(&render_dimensions_section(&scored));
    html.push_str(&render_panel_section(&panel));
    if show_gaps_section(&result.missing_dims, &syn.missing_highlights) {
        html.push_str(&render_gaps_section(
            &result.missing_dims,
            &syn.missing_highlights,
        ));
    }
    if !syn.risks.is_empty() {
        html.push_str(&render_risks_section(&syn.risks));
    }
    html.push_str(&render_appendix(&result));
    if let Some(text) = narrative {
        html.push_str(&render_narrative_section(text));
    }
    html.push_str("</body></html>");

    if html.len() > MAX_HTML_BYTES {
        tracing::warn!(
            symbol = %result.symbol,
            bytes = html.len(),
            max = MAX_HTML_BYTES,
            "institutional HTML exceeds size cap — truncating appendix"
        );
        // Re-render without appendix if over cap (panel already collapsed to Top20).
        html = render_shell_start(&identity, syn);
        if result.data_confidence.score < CONFIDENCE_WARN_THRESHOLD {
            html.push_str(&render_warn_banner(result.data_confidence.score));
        }
        html.push_str(&render_fundamentals_section(&result.content.fundamentals));
        html.push_str(&render_sector_section(&result.content.sector));
        html.push_str(&render_external_section(&result.content.external));
        html.push_str(&render_flows_section(&result.content.flows_events));
        html.push_str(&render_dimensions_section(&scored));
        html.push_str(&render_panel_section(&panel));
        if !syn.risks.is_empty() {
            html.push_str(&render_risks_section(&syn.risks));
        }
        if let Some(text) = narrative {
            html.push_str(&render_narrative_section(text));
        }
        html.push_str("</body></html>");
    }

    debug_assert!(
        html.len() <= MAX_HTML_BYTES,
        "institutional HTML exceeds {MAX_HTML_BYTES} bytes"
    );
    html
}

fn render_gaps_section(missing_dims: &[String], highlights: &[String]) -> String {
    use crate::research::report::dim_viz::render_missing_chip;
    let mut chips: Vec<String> = highlights
        .iter()
        .map(|h| render_missing_chip(&escape_html(h)))
        .collect();
    for d in missing_dims {
        let esc = escape_html(d);
        if !highlights.iter().any(|h| h == d) {
            chips.push(render_missing_chip(&esc));
        }
    }
    format!(
        r#"<section class="card"><h2>数据缺口</h2><div class="chips">{}</div></section>"#,
        chips.join("")
    )
}

fn render_risks_section(risks: &[String]) -> String {
    let items: String = risks
        .iter()
        .map(|r| format!("<li>{}</li>", escape_html(r)))
        .collect();
    format!(r#"<section class="card"><h2>关键风险</h2><ul class="risk">{items}</ul></section>"#)
}

fn render_narrative_section(text: &str) -> String {
    format!(
        r#"<section class="card"><h2>分析结论</h2><div class="narrative">{}</div></section>"#,
        escape_html(text)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::analyze::analyze_stock;
    use crate::research::fetchers::bridge::apply_dims_to_snapshot;
    use crate::research::fetchers::types::{CollectOutput, DimQuality, DimResult, Market};
    use crate::research::profile::AnalysisProfile;
    use crate::research::types::FundamentalsSnapshot;
    use serde_json::{Value, json};

    fn moutai_result() -> AnalyzeStockResult {
        let symbol = "600519.SH";
        let dims = json!({
            "0_basic": { "data": { "name": "贵州茅台", "industry": "白酒", "price": 1680.0, "pe_ttm": 28.5, "pb": 8.2, "market_cap_yi": 21000, "shares_outstanding_yi": 12.56 } },
            "1_financials": { "data": { "roe": 32.0, "net_margin": 52.0, "revenue_latest_yi": 1500, "fcf_yi": 600, "financial_health": { "debt_ratio": 18.0 } } },
            "10_valuation": { "data": { "pe_ttm": 28.5, "pe_percentile": 35.0 } },
            "4_peers": { "data": { "peer_table": [{ "name": "五粮液", "ticker": "000858", "pe": 18.0, "pb": 4.2 }] } },
            "6_research": { "data": { "research_count": 10 } },
            "7_industry": { "data": { "industry": "白酒", "growth": 12.0, "industry_pe": 22.0 } },
            "6_fund_holders": { "data": { "holder_change_ratio": -8.0, "holder_count": 95000 } },
            "12_capital_flow": { "data": { "main_fund_5d_net_yi": 3.5 } }
        });
        let mut collect = CollectOutput {
            ticker: symbol.into(),
            market: Market::A,
            dims: Default::default(),
        };
        if let Some(obj) = dims.as_object() {
            for (key, wrapper) in obj {
                let data = wrapper.get("data").cloned().unwrap_or(Value::Null);
                collect.dims.insert(
                    key.clone(),
                    DimResult::ok(key, symbol, data, "fixture", DimQuality::Partial),
                );
            }
        }
        let raw_dims = collect.build_raw_dims();
        let mut snap = FundamentalsSnapshot {
            symbol: symbol.into(),
            ..Default::default()
        };
        apply_dims_to_snapshot(&mut snap, &collect);
        analyze_stock(
            &snap,
            Some(&raw_dims),
            None,
            &AnalysisProfile::medium(),
            Some(&collect),
        )
    }

    #[test]
    fn institutional_html_contains_content_sections() {
        let html = render_institutional_html(&moutai_result(), None);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("600519.SH"));
        assert!(html.contains("公司基本面"));
        assert!(html.contains("板块与同业"));
        assert!(html.contains("政策 / 宏观 / 舆情"));
        assert!(html.contains("19 维评分"));
        assert!(html.contains("66 位评委"));
        assert!(html.len() < MAX_HTML_BYTES);
    }

    #[test]
    fn institutional_html_shows_warn_when_low_confidence() {
        let mut result = moutai_result();
        result.data_confidence.score = 0.40;
        result.synthesis.confidence_tier = "low".into();
        let html = render_institutional_html(&result, None);
        assert!(html.contains("数据置信度"));
    }
}
