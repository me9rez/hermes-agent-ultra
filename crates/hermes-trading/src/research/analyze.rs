//! End-to-end stock analysis orchestrator.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::quote_data::QuoteData;
use crate::research::fetchers::types::{CollectOutput, DimSummaryEntry};
use crate::research::models::{
    CompsPeer, CompsTarget, ThreeStmtResult, build_comps_table, compute_dcf, project_three_stmt,
    quick_lbo,
};
use crate::research::profile::AnalysisProfile;
use crate::research::report::{ReportIdentity, build_report_content, infer_target_name_from_peers};
use crate::research::report::{render_quick_scan_markdown, render_summary_markdown};
use crate::research::scoring::{generate_panel, score_dimensions};
use crate::research::synthesis::{SynthesisReport, build_synthesis_parts};
use crate::research::types::{DataConfidence, FeatureVector, FundamentalsSnapshot};
use crate::text_encoding::is_usable_company_name;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzeStockResult {
    pub symbol: String,
    pub depth: String,
    pub dcf: Value,
    pub comps: Value,
    pub three_statement: Value,
    pub lbo: Value,
    pub scores: Value,
    pub personas: Value,
    pub data_confidence: DataConfidence,
    pub missing_dims: Vec<String>,
    pub dim_summary: Vec<DimSummaryEntry>,
    pub used_fallback: Vec<String>,
    /// Deterministic markdown: quick-scan (lite) or full 19-dim + panel (medium).
    pub summary_markdown: String,
    /// Structured conclusion for HTML / agents (wave 2b).
    pub synthesis: SynthesisReport,
    /// Content-first blocks for HTML / brief (fundamentals / sector / external).
    #[serde(default)]
    pub content: crate::research::report::ReportContent,
}

/// Run analysis pipeline on a fundamentals snapshot.
#[must_use]
pub fn analyze_stock(
    snap: &FundamentalsSnapshot,
    raw_dims: Option<&Value>,
    peers: Option<&[CompsPeer]>,
    profile: &AnalysisProfile,
    collect: Option<&CollectOutput>,
) -> AnalyzeStockResult {
    let features: FeatureVector = snap.clone();
    let mut used_fallback = Vec::new();

    let dcf = compute_dcf(&features, None);
    used_fallback.extend(dcf.used_fallback.clone());

    let comps = if profile.run_comps_lbo_three_stmt {
        let effective_peers: Vec<CompsPeer> = match peers {
            Some(p) if !p.is_empty() => p.to_vec(),
            _ => peers_from_raw_dims(raw_dims).unwrap_or_default(),
        };
        let company_name = snap
            .name
            .clone()
            .filter(|n| is_usable_company_name(n))
            .or_else(|| infer_target_name_from_peers(&effective_peers, &snap.symbol));
        let code = snap
            .symbol
            .split('.')
            .next()
            .unwrap_or(&snap.symbol)
            .to_string();
        let target = CompsTarget {
            name: company_name.clone(),
            ticker: Some(code),
            price: snap.price,
            pe: snap.pe,
            pb: snap.pb,
            eps: snap.eps,
            bvps: snap.bvps,
            ..Default::default()
        };
        if effective_peers.is_empty() {
            serde_json::json!({"error": "no peers provided"})
        } else {
            serde_json::to_value(build_comps_table(target, &effective_peers)).unwrap_or(Value::Null)
        }
    } else {
        serde_json::json!({"skipped": "lite mode — comps not run"})
    };

    let three_stmt = if profile.run_comps_lbo_three_stmt {
        match project_three_stmt(&features, None) {
            ThreeStmtResult::Ok(ok) => {
                used_fallback.extend(ok.used_fallback.clone());
                serde_json::to_value(ok).unwrap_or(Value::Null)
            }
            ThreeStmtResult::Error { error, .. } => {
                serde_json::json!({"error": error})
            }
        }
    } else {
        serde_json::json!({"skipped": "lite mode"})
    };

    let lbo = if profile.run_comps_lbo_three_stmt {
        let lbo = quick_lbo(&features, None);
        used_fallback.extend(lbo.used_fallback.clone());
        serde_json::to_value(&lbo).unwrap_or(Value::Null)
    } else {
        serde_json::json!({"skipped": "lite mode"})
    };

    let dims = raw_dims
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let scored = score_dimensions(&snap.symbol, &dims, &features, profile);
    let mut missing_dims: BTreeSet<String> = BTreeSet::new();
    if let Some(c) = collect {
        missing_dims.extend(c.missing_dim_keys());
    }
    for (key, d) in &scored.dimensions {
        if !d.missing.is_empty() && !crate::research::report_filter::is_web_only_dim(key) {
            missing_dims.insert(key.clone());
        }
    }
    let missing_dims: Vec<String> = crate::research::report_filter::user_missing_dims(
        &missing_dims.into_iter().collect::<Vec<_>>(),
    );
    let dim_summary = collect.map(CollectOutput::dim_summary).unwrap_or_default();
    let panel = generate_panel(&scored, &features, profile);
    let data_confidence = DataConfidence::from_snapshot(snap);
    let summary_markdown = if profile.is_lite() {
        render_quick_scan_markdown(
            &snap.symbol,
            &scored,
            &panel.investors,
            &data_confidence,
            &dcf,
            profile,
        )
    } else {
        render_summary_markdown(
            &snap.symbol,
            &scored,
            &panel,
            &data_confidence,
            Some(dcf.verdict.as_str()),
        )
    };

    let mut identity = ReportIdentity::from_snapshot(snap);
    identity.enrich_from_comps(&comps);
    identity.fundamental_score = Some(scored.fundamental_score);
    let content = build_report_content(&dims, snap, &comps, &scored, &dcf);
    let synthesis = build_synthesis_parts(
        &identity,
        profile.depth_label(),
        &scored,
        &panel,
        &dcf,
        &data_confidence,
        &missing_dims,
        &used_fallback,
    );

    AnalyzeStockResult {
        symbol: snap.symbol.clone(),
        depth: profile.depth_label().to_string(),
        dcf: serde_json::to_value(&dcf).unwrap_or(Value::Null),
        comps,
        three_statement: three_stmt,
        lbo,
        scores: serde_json::to_value(&scored).unwrap_or(Value::Null),
        personas: serde_json::to_value(&panel).unwrap_or(Value::Null),
        data_confidence,
        missing_dims,
        dim_summary,
        used_fallback,
        summary_markdown,
        synthesis,
        content,
    }
}

/// Apply web-search overlay to a cached analysis result.
pub fn apply_external_context(
    result: &mut AnalyzeStockResult,
    overlay: &crate::research::report::ExternalContextOverlay,
) {
    crate::research::report::merge_external_overlay(&mut result.content, overlay);
}

/// Build snapshot from quote + optional JSON fundamentals.
#[must_use]
pub fn snapshot_from_inputs(
    quote: &QuoteData,
    fundamentals: Option<&Value>,
) -> FundamentalsSnapshot {
    let mut snap = FundamentalsSnapshot::from_quote(quote);
    if let Some(f) = fundamentals {
        snap.merge_json(f);
    }
    snap
}

fn peers_from_raw_dims(raw_dims: Option<&Value>) -> Option<Vec<CompsPeer>> {
    let table = raw_dims?
        .get("4_peers")?
        .get("data")?
        .get("peer_table")?
        .as_array()?;
    if table.is_empty() {
        return None;
    }
    let peers: Vec<CompsPeer> = table
        .iter()
        .map(|row| CompsPeer {
            name: row.get("name").and_then(|v| v.as_str()).map(str::to_string),
            ticker: row
                .get("ticker")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            pe: row.get("pe").and_then(|v| v.as_f64()),
            pb: row.get("pb").and_then(|v| v.as_f64()),
            ..Default::default()
        })
        .collect();
    if peers.is_empty() { None } else { Some(peers) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::types::FundamentalsSnapshot;
    use serde_json::json;

    #[test]
    fn peers_from_raw_dims_maps_peer_table() {
        let raw = json!({
            "4_peers": {
                "data": {
                    "peer_table": [
                        {"name": "五粮液", "ticker": "000858.SZ", "pe": 18.0, "pb": 4.2}
                    ]
                }
            }
        });
        let peers = peers_from_raw_dims(Some(&raw)).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].name.as_deref(), Some("五粮液"));
        assert_eq!(peers[0].pe, Some(18.0));
    }

    #[test]
    fn lite_skips_comps_lbo_three_stmt() {
        let snap = FundamentalsSnapshot {
            symbol: "600519.SH".into(),
            price: Some(100.0),
            ..Default::default()
        };
        let result = analyze_stock(&snap, None, None, &AnalysisProfile::lite(), None);
        assert_eq!(result.depth, "lite");
        assert!(result.comps.get("skipped").is_some());
        assert!(result.summary_markdown.contains("速判"));
    }

    #[test]
    fn medium_includes_full_panel_path() {
        let snap = FundamentalsSnapshot {
            symbol: "600519.SH".into(),
            price: Some(100.0),
            ..Default::default()
        };
        let result = analyze_stock(&snap, None, None, &AnalysisProfile::medium(), None);
        assert_eq!(result.depth, "medium");
        assert!(result.summary_markdown.contains("19 维"));
    }
}
