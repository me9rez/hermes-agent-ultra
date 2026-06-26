//! Deterministic synthesis summary from analyze_stock outputs (wave 2b PR-1).

use serde::{Deserialize, Serialize};

use crate::research::models::dcf::{DcfResult, compute_dcf};
use crate::research::report::ReportIdentity;
use crate::research::scoring::{PanelResult, ScoreDimensionsResult};
use crate::research::types::{DataConfidence, FundamentalsSnapshot};

/// Structured conclusion block for HTML, agents, and parity goldens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SynthesisReport {
    pub headline: String,
    pub verdict: String,
    pub confidence_tier: String,
    pub key_metrics: Vec<KeyMetric>,
    pub risks: Vec<String>,
    pub missing_highlights: Vec<String>,
    pub panel_summary: PanelSummary,
    pub dcf_one_liner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyMetric {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelSummary {
    pub consensus: f64,
    pub vote_buy: u32,
    pub vote_avoid: u32,
    pub investor_count: u32,
}

/// Slim JSON payload for `format=synthesis` (wave 2b PR-3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SynthesisFormatOutput {
    pub symbol: String,
    pub depth: String,
    pub synthesis: SynthesisReport,
    pub data_confidence: DataConfidence,
    pub missing_dims: Vec<String>,
    pub fundamental_score: f64,
    pub panel_consensus: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_paths: Option<ReportPaths>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReportPaths {
    pub html: String,
    pub analysis_json: String,
}

/// Build synthesis from a completed analysis result (re-parses embedded JSON fields).
#[must_use]
pub fn build_synthesis(result: &crate::research::analyze::AnalyzeStockResult) -> SynthesisReport {
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
    let dcf: DcfResult = serde_json::from_value(result.dcf.clone()).unwrap_or_else(|_| {
        compute_dcf(
            &FundamentalsSnapshot {
                symbol: result.symbol.clone(),
                ..Default::default()
            },
            None,
        )
    });
    build_synthesis_parts(
        &ReportIdentity::from_analyze_result(result),
        result.depth.as_str(),
        &scored,
        &panel,
        &dcf,
        &result.data_confidence,
        &result.missing_dims,
        &result.used_fallback,
    )
}

#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn build_synthesis_parts(
    identity: &ReportIdentity,
    depth: &str,
    scored: &ScoreDimensionsResult,
    panel: &PanelResult,
    dcf: &DcfResult,
    confidence: &DataConfidence,
    missing_dims: &[String],
    used_fallback: &[String],
) -> SynthesisReport {
    let confidence_tier = confidence_tier(confidence.score);
    let verdict = synthesis_verdict(
        scored.fundamental_score,
        panel.panel_consensus,
        &dcf.verdict,
        confidence.score,
    );
    let panel_summary = PanelSummary {
        consensus: panel.panel_consensus,
        vote_buy: panel.vote_distribution.strongly_buy + panel.vote_distribution.buy,
        vote_avoid: panel.vote_distribution.avoid,
        investor_count: panel.investors.len() as u32,
    };
    let dcf_one_liner = format!("{} · 安全边际 {:+.1}%", dcf.verdict, dcf.safety_margin_pct);
    let headline = build_headline(
        identity,
        depth,
        scored.fundamental_score,
        panel.panel_consensus,
        &verdict,
        confidence.score,
    );
    SynthesisReport {
        headline,
        verdict,
        confidence_tier,
        key_metrics: build_key_metrics(identity, scored, panel, confidence, dcf),
        risks: collect_risks(scored, used_fallback),
        missing_highlights: collect_missing_highlights(confidence, missing_dims),
        panel_summary,
        dcf_one_liner,
    }
}

/// Build slim agent-facing JSON for `analyze_stock(format=synthesis)`.
#[must_use]
pub fn build_synthesis_format_output(
    result: &crate::research::analyze::AnalyzeStockResult,
) -> SynthesisFormatOutput {
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
    SynthesisFormatOutput {
        symbol: result.symbol.clone(),
        depth: result.depth.clone(),
        synthesis: result.synthesis.clone(),
        data_confidence: result.data_confidence.clone(),
        missing_dims: result.missing_dims.clone(),
        fundamental_score: scored.fundamental_score,
        panel_consensus: panel.panel_consensus,
        report_paths: None,
    }
}

fn confidence_tier(score: f64) -> String {
    if score >= 0.65 {
        "high".into()
    } else if score >= 0.45 {
        "medium".into()
    } else {
        "low".into()
    }
}

fn synthesis_verdict(
    fundamental_score: f64,
    panel_consensus: f64,
    dcf_verdict: &str,
    confidence: f64,
) -> String {
    if confidence < 0.40 {
        return "insufficient_data".into();
    }
    let dcf_bull = dcf_verdict.contains("低估");
    let dcf_bear = dcf_verdict.contains("高估");
    // `fundamental_score` is 0–10 (same scale as dim averages); markdown labels it "/100".
    if fundamental_score >= 7.5 && panel_consensus >= 8.0 && dcf_bull {
        return "strongly_buy".into();
    }
    if fundamental_score >= 6.0
        || panel_consensus >= 7.0
        || (panel_consensus >= 6.5 && dcf_bull)
        || (fundamental_score >= 5.5 && panel_consensus >= 6.5 && confidence >= 0.55)
    {
        return "buy".into();
    }
    if fundamental_score < 4.5 || panel_consensus < 4.0 || (dcf_bear && panel_consensus < 5.5) {
        return "avoid".into();
    }
    "watch".into()
}

fn build_headline(
    identity: &ReportIdentity,
    depth: &str,
    fundamental_score: f64,
    panel_consensus: f64,
    verdict: &str,
    confidence: f64,
) -> String {
    let stance = match verdict {
        "strongly_buy" => "强烈偏多",
        "buy" => "偏多",
        "avoid" => "偏空",
        "insufficient_data" => "数据不足",
        _ => "观望",
    };
    let mode = if depth == "lite" {
        "速判"
    } else {
        "深度分析"
    };
    format!(
        "{} · {mode}：{stance} · 综合 {fundamental_score:.1}/100 · 评委 {panel_consensus:.1}/10 · 置信度 {:.0}%",
        identity.title_prefix(),
        confidence * 100.0
    )
}

fn build_key_metrics(
    identity: &ReportIdentity,
    scored: &ScoreDimensionsResult,
    panel: &PanelResult,
    confidence: &DataConfidence,
    dcf: &DcfResult,
) -> Vec<KeyMetric> {
    let mut out = Vec::new();
    if let Some(name) = &identity.company_name {
        out.push(KeyMetric {
            label: "公司".into(),
            value: name.clone(),
        });
    }
    if let Some(price) = identity.price {
        out.push(KeyMetric {
            label: "现价".into(),
            value: format!("¥{price:.2}"),
        });
    }
    if let Some(ind) = &identity.industry {
        out.push(KeyMetric {
            label: "行业".into(),
            value: ind.clone(),
        });
    }
    if let Some(cap) = identity.market_cap_yi {
        out.push(KeyMetric {
            label: "市值".into(),
            value: format!("{cap:.0} 亿"),
        });
    }
    if let Some(pe) = identity.pe {
        out.push(KeyMetric {
            label: "PE".into(),
            value: format!("{pe:.1}"),
        });
    }
    if let Some(pb) = identity.pb {
        out.push(KeyMetric {
            label: "PB".into(),
            value: format!("{pb:.2}"),
        });
    }
    out.extend([
        KeyMetric {
            label: "基本面综合".into(),
            value: format!("{:.1}/100", scored.fundamental_score),
        },
        KeyMetric {
            label: "评委共识".into(),
            value: format!("{:.1}/10", panel.panel_consensus),
        },
        KeyMetric {
            label: "数据置信度".into(),
            value: format!("{:.0}%", confidence.score * 100.0),
        },
        KeyMetric {
            label: "DCF 安全边际".into(),
            value: format!("{:+.1}%", dcf.safety_margin_pct),
        },
    ]);
    if dcf.intrinsic_per_share > 0.0 {
        out.push(KeyMetric {
            label: "DCF 内在价值".into(),
            value: format!("¥{}", dcf.intrinsic_per_share),
        });
    }
    out
}

fn collect_risks(scored: &ScoreDimensionsResult, used_fallback: &[String]) -> Vec<String> {
    let mut risks = Vec::new();
    for dim in scored.dimensions.values() {
        for fail in &dim.reasons_fail {
            if risks.len() >= 3 {
                break;
            }
            if !risks.contains(fail) {
                risks.push(fail.clone());
            }
        }
    }
    for dim in scored.dimensions.values() {
        if risks.len() >= 3 {
            break;
        }
        if dim.score <= 4 && !dim.label.is_empty() && !risks.contains(&dim.label) {
            risks.push(dim.label.clone());
        }
    }
    for fb in used_fallback {
        if risks.len() >= 4 {
            break;
        }
        let line = format!("模型假设: {fb}");
        if !risks.contains(&line) {
            risks.push(line);
        }
    }
    risks
}

fn collect_missing_highlights(confidence: &DataConfidence, missing_dims: &[String]) -> Vec<String> {
    crate::research::report_filter::user_missing_highlights(&confidence.missing, missing_dims)
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

    fn moutai_collect() -> CollectOutput {
        let symbol = "600519.SH";
        let dims = json!({
            "0_basic": {
                "data": {
                    "name": "贵州茅台",
                    "industry": "白酒",
                    "price": 1680.0,
                    "pe_ttm": 28.5,
                    "pb": 8.2,
                    "market_cap_yi": 21000,
                    "shares_outstanding_yi": 12.56
                }
            },
            "1_financials": {
                "data": {
                    "roe": 32.0,
                    "net_margin": 52.0,
                    "revenue_latest_yi": 1500,
                    "fcf_yi": 600,
                    "financial_health": { "debt_ratio": 18.0 }
                }
            },
            "10_valuation": {
                "data": { "pe_ttm": 28.5, "pe_percentile": 35.0 }
            },
            "4_peers": {
                "data": {
                    "peer_table": [
                        { "name": "五粮液", "pe": 18.0 },
                        { "name": "泸州老窖", "pe": 16.0 }
                    ]
                }
            },
            "6_research": { "data": { "research_count": 10 } },
            "7_industry": { "data": { "industry": "白酒", "growth": 12.0, "industry_pe": 22.0 } },
            "6_fund_holders": { "data": { "holder_change_ratio": -8.0, "holder_count": 95000 } },
            "12_capital_flow": { "data": { "main_fund_5d_net_yi": 3.5 } }
        });
        let mut output = CollectOutput {
            ticker: symbol.into(),
            market: Market::A,
            dims: Default::default(),
        };
        if let Some(obj) = dims.as_object() {
            for (key, wrapper) in obj {
                let data = wrapper.get("data").cloned().unwrap_or(Value::Null);
                output.dims.insert(
                    key.clone(),
                    DimResult::ok(key, symbol, data, "fixture", DimQuality::Partial),
                );
            }
        }
        output
    }

    #[test]
    fn moutai_synthesis_smoke() {
        let collect = moutai_collect();
        let raw_dims = collect.build_raw_dims();
        let mut snap = FundamentalsSnapshot {
            symbol: "600519.SH".into(),
            ..Default::default()
        };
        apply_dims_to_snapshot(&mut snap, &collect);
        let result = analyze_stock(
            &snap,
            Some(&raw_dims),
            None,
            &AnalysisProfile::medium(),
            Some(&collect),
        );
        let syn = &result.synthesis;
        assert_eq!(syn.confidence_tier, "high");
        assert!(syn.headline.contains("600519.SH"));
        assert!(syn.dcf_one_liner.contains("安全边际"));
        assert!(syn.panel_summary.consensus >= 7.0);
        assert_eq!(syn.verdict, "buy");
        assert!(!syn.key_metrics.is_empty());

        let slim = build_synthesis_format_output(&result);
        assert_eq!(slim.symbol, "600519.SH");
        assert_eq!(slim.synthesis.verdict, "buy");
        assert!(slim.fundamental_score > 0.0);
    }

    use crate::research::models::compute_dcf;

    #[test]
    fn insufficient_data_when_confidence_low() {
        let snap = FundamentalsSnapshot {
            symbol: "600157.SH".into(),
            price: Some(1.2),
            ..Default::default()
        };
        let dcf = compute_dcf(&snap, None);
        let syn = build_synthesis_parts(
            &ReportIdentity::from_snapshot(&snap),
            "medium",
            &ScoreDimensionsResult {
                ticker: "600157.SH".into(),
                fundamental_score: 3.5,
                dimensions: Default::default(),
            },
            &PanelResult {
                investors: vec![],
                vote_distribution: Default::default(),
                signal_distribution: Default::default(),
                panel_consensus: 4.0,
            },
            &dcf,
            &DataConfidence {
                score: 0.35,
                present: vec!["price".into()],
                missing: vec!["fcf_latest_yi".into()],
            },
            &["3_macro".into()],
            &[],
        );
        assert_eq!(syn.verdict, "insufficient_data");
        assert_eq!(syn.confidence_tier, "low");
        assert!(syn.missing_highlights.is_empty());
    }
}
