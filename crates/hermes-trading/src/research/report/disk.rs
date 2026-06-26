//! Optional on-disk equity research reports (wave 2b PR-3).

use std::path::{Path, PathBuf};

use hermes_config::hermes_home;

use crate::error::TradingError;
use crate::research::analyze::AnalyzeStockResult;

/// Paths written under `{HERMES_HOME}/reports/{symbol}_{date}/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenReportPaths {
    pub html: PathBuf,
    pub analysis_json: PathBuf,
}

/// Directory name segment: `600519_SH_2026-06-25`.
#[must_use]
pub fn report_dir_name(symbol: &str, date: &str) -> String {
    let safe_sym = symbol.replace('.', "_");
    format!("{safe_sym}_{date}")
}

/// Write institutional HTML + full analysis JSON to disk.
pub fn write_equity_report(
    result: &AnalyzeStockResult,
    html: &str,
    home_override: Option<&Path>,
) -> Result<WrittenReportPaths, TradingError> {
    let home = home_override.map_or_else(hermes_home, PathBuf::from);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dir = home
        .join("reports")
        .join(report_dir_name(&result.symbol, &date));
    std::fs::create_dir_all(&dir)?;
    let html_path = dir.join("full-report-standalone.html");
    let analysis_json_path = dir.join("analysis.json");
    std::fs::write(&html_path, html)?;
    std::fs::write(&analysis_json_path, serde_json::to_string_pretty(result)?)?;
    Ok(WrittenReportPaths {
        html: html_path,
        analysis_json: analysis_json_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::synthesis::SynthesisReport;
    use crate::research::types::DataConfidence;

    fn stub_result(symbol: &str) -> AnalyzeStockResult {
        AnalyzeStockResult {
            symbol: symbol.into(),
            depth: "medium".into(),
            dcf: serde_json::json!({}),
            comps: serde_json::json!({}),
            three_statement: serde_json::json!({}),
            lbo: serde_json::json!({}),
            scores: serde_json::json!({"fundamental_score": 7.5}),
            personas: serde_json::json!({"panel_consensus": 7.2}),
            data_confidence: DataConfidence {
                score: 0.6,
                present: vec!["price".into()],
                missing: vec![],
            },
            missing_dims: vec![],
            dim_summary: vec![],
            used_fallback: vec![],
            summary_markdown: "# test".into(),
            synthesis: SynthesisReport {
                headline: "test".into(),
                verdict: "hold".into(),
                confidence_tier: "medium".into(),
                key_metrics: vec![],
                risks: vec![],
                missing_highlights: vec![],
                panel_summary: crate::research::synthesis::PanelSummary {
                    consensus: 7.0,
                    vote_buy: 40,
                    vote_avoid: 5,
                    investor_count: 66,
                },
                dcf_one_liner: "test".into(),
            },
            content: crate::research::report::ReportContent::default(),
        }
    }

    #[test]
    fn report_dir_name_sanitizes_symbol() {
        assert_eq!(
            report_dir_name("600519.SH", "2026-06-25"),
            "600519_SH_2026-06-25"
        );
    }

    #[test]
    fn write_equity_report_creates_html_and_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = stub_result("600519.SH");
        let paths =
            write_equity_report(&result, "<html>ok</html>", Some(tmp.path())).expect("write");
        assert!(paths.html.exists());
        assert!(paths.analysis_json.exists());
        let html = std::fs::read_to_string(&paths.html).expect("read html");
        assert!(html.contains("ok"));
        let json = std::fs::read_to_string(&paths.analysis_json).expect("read json");
        assert!(json.contains("600519.SH"));
    }
}
