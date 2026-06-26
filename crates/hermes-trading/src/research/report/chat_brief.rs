//! WeCom-friendly chat brief: synthesis + 19-dim table (no 66-judge detail).

use crate::research::analyze::AnalyzeStockResult;
use crate::research::scoring::{PanelResult, ScoreDimensionsResult};
use crate::research::types::DataConfidence;

use super::labels::{DIM_ORDER, dimension_display_name};
use super::markdown::score_badge;
use crate::research::report::ReportIdentity;
use crate::research::report_filter::scrub_dim_label;

/// Chat summary for `/analyze-stock`: fits WeCom ~4000 chars; full panel lives in HTML attachment.
#[must_use]
pub fn render_chat_brief_markdown(result: &AnalyzeStockResult) -> String {
    let mut result = result.clone();
    crate::research::report_filter::scrub_user_report(&mut result);
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
    render_chat_brief_parts(
        &ReportIdentity::from_analyze_result(&result),
        &result.synthesis.headline,
        &result.synthesis.dcf_one_liner,
        &result.content,
        &scored,
        &panel,
        result.synthesis.panel_summary.investor_count,
        &result.data_confidence,
        &result.used_fallback,
    )
}

#[must_use]
#[allow(clippy::too_many_arguments)]
fn render_chat_brief_parts(
    identity: &ReportIdentity,
    headline: &str,
    dcf_one_liner: &str,
    content: &crate::research::report::ReportContent,
    scored: &ScoreDimensionsResult,
    panel: &PanelResult,
    investor_count: u32,
    confidence: &DataConfidence,
    used_fallback: &[String],
) -> String {
    let mut out = format!(
        "## {} · 深度分析（摘要）\n\n{headline}\n\n",
        identity.title_prefix()
    );
    out.push_str("| 指标 | 数值 |\n| --- | --- |\n");
    out.push_str(&format!(
        "| 基本面综合 | {:.1}/100 |\n",
        scored.fundamental_score
    ));
    out.push_str(&format!(
        "| 评委共识 | {:.1}/10（{} 位） |\n",
        panel.panel_consensus,
        if panel.investors.is_empty() {
            investor_count
        } else {
            panel.investors.len() as u32
        }
    ));
    out.push_str(&format!(
        "| 数据置信度 | {:.0}% |\n",
        confidence.score * 100.0
    ));
    out.push_str(&format!("| DCF | {dcf_one_liner} |\n\n"));

    out.push_str("### 公司基本面\n\n");
    for b in &content.fundamentals.bullets {
        out.push_str(&format!("- {b}\n"));
    }
    out.push('\n');

    out.push_str("### 板块与同业\n\n");
    if let Some(ind) = &content.sector.industry_name {
        out.push_str(&format!("- 行业：{ind}\n"));
    }
    if let Some(note) = &content.sector.relative_note {
        out.push_str(&format!("- {note}\n"));
    }
    if content.sector.peer_rows.is_empty() && content.sector.industry_name.is_none() {
        out.push_str("- （同业数据待补充）\n");
    }
    out.push('\n');

    out.push_str("### 政策 / 宏观 / 舆情\n\n");
    match content.external.coverage {
        crate::research::report::content::ExternalCoverage::WebFilled => {
            for b in content
                .external
                .policy_bullets
                .iter()
                .chain(content.external.macro_bullets.iter())
                .chain(content.external.sentiment_bullets.iter())
            {
                out.push_str(&format!("- {b}\n"));
            }
        }
        _ => {
            out.push_str("- 本次未检索；详见 HTML 附件「政策 / 宏观 / 舆情」章节。\n");
        }
    }
    out.push('\n');

    out.push_str("### 19 维评分概览\n\n");
    out.push_str("| 维度 | 评分 | 说明 |\n| --- | --- | --- |\n");
    for key in DIM_ORDER {
        let Some(d) = scored.dimensions.get(*key) else {
            continue;
        };
        let name = if d.display_name.is_empty() {
            dimension_display_name(key)
        } else {
            d.display_name.clone()
        };
        let badge = score_badge(d.score);
        out.push_str(&format!(
            "| {name} | {}/{}{} | {} |\n",
            d.score,
            10,
            badge,
            scrub_dim_label(&d.label)
        ));
    }

    let vd = &panel.vote_distribution;
    out.push_str("\n### 评委投票（汇总）\n\n");
    out.push_str(&format!(
        "强烈买入 {} · 买入 {} · 观望 {} · 回避 {} · 跳过 {}\n",
        vd.strongly_buy,
        vd.buy,
        vd.watch + vd.wait,
        vd.avoid,
        vd.skip + vd.n_a
    ));

    out.push_str(
        "\n> ⚠️ 以上分析仅供参考，不构成投资建议。完整评委列表、DCF 假设与图表见附件 HTML。\n",
    );
    if !used_fallback.is_empty() {
        out.push_str(&format!(
            "> 部分字段使用 fallback 估算：{}。\n",
            used_fallback.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::synthesis::SynthesisReport;
    use crate::research::types::DataConfidence;

    fn stub_result() -> AnalyzeStockResult {
        AnalyzeStockResult {
            symbol: "600519.SH".into(),
            depth: "medium".into(),
            dcf: serde_json::json!({}),
            comps: serde_json::json!({}),
            three_statement: serde_json::json!({}),
            lbo: serde_json::json!({}),
            scores: serde_json::json!({
                "fundamental_score": 5.6,
                "dimensions": {
                    "financials": { "score": 7, "label": "ROE 10.6%", "display_name": "财务面" }
                }
            }),
            personas: serde_json::json!({
                "panel_consensus": 77.5,
                "investors": [{ "id": "buffett" }],
                "vote_distribution": { "strongly_buy": 30, "buy": 0, "watch": 2, "avoid": 10, "skip": 24, "wait": 0, "n_a": 0 },
                "signal_distribution": { "bullish": 30, "neutral": 2, "bearish": 10, "skip": 24 }
            }),
            data_confidence: DataConfidence {
                score: 0.75,
                present: vec!["price".into()],
                missing: vec![],
            },
            missing_dims: vec![],
            dim_summary: vec![],
            used_fallback: vec!["fcf_from_revenue_margin".into()],
            summary_markdown: String::new(),
            synthesis: SynthesisReport {
                headline: "600519.SH · 深度分析：偏多 · 综合 5.6/100 · 评委 77.5/10 · 置信度 75%"
                    .into(),
                verdict: "buy".into(),
                confidence_tier: "high".into(),
                key_metrics: vec![],
                risks: vec![],
                missing_highlights: vec![],
                panel_summary: crate::research::synthesis::PanelSummary {
                    consensus: 77.5,
                    vote_buy: 30,
                    vote_avoid: 10,
                    investor_count: 66,
                },
                dcf_one_liner: "🔴 明显高估 · 安全边际 -46.1%".into(),
            },
            content: crate::research::report::ReportContent::default(),
        }
    }

    #[test]
    fn chat_brief_under_wecom_limit_and_omits_judge_table() {
        let md = render_chat_brief_markdown(&stub_result());
        assert!(
            md.chars().count() < 4000,
            "brief should fit WeCom single message"
        );
        assert!(md.contains("摘要"));
        assert!(md.contains("19 维"));
        assert!(!md.contains("| 沃伦·巴菲特 |"));
        assert!(md.contains("fcf_from_revenue_margin"));
    }
}
