//! Fetcher mapper golden tests (offline JSON fixtures, no network).

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use hermes_trading::research::analyze::analyze_stock;
use hermes_trading::research::confidence_supplement::supplement_snapshot_confidence;
use hermes_trading::research::fetchers::bridge::apply_dims_to_snapshot;
use hermes_trading::research::fetchers::types::{CollectOutput, DimQuality, DimResult, Market};
use hermes_trading::research::profile::AnalysisProfile;
use hermes_trading::research::scoring::score_dimensions;
use hermes_trading::research::types::{DataConfidence, FundamentalsSnapshot};

#[derive(Debug, serde::Deserialize)]
struct FixtureFile {
    cases: Vec<FixtureCase>,
}

#[derive(Debug, serde::Deserialize)]
struct FixtureCase {
    id: String,
    op: String,
    input: Value,
    expected: Value,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/trading_research_fetch/fetcher_golden.json")
}

fn collect_from_input(input: &Value) -> CollectOutput {
    let symbol = input["symbol"].as_str().unwrap_or("TEST.SH");
    let mut output = CollectOutput {
        ticker: symbol.into(),
        market: Market::A,
        dims: Default::default(),
    };
    if let Some(dims) = input.get("dims").and_then(|v| v.as_object()) {
        for (key, wrapper) in dims {
            let data = wrapper.get("data").cloned().unwrap_or(Value::Null);
            output.dims.insert(
                key.clone(),
                DimResult::ok(key, symbol, data, "fixture", DimQuality::Partial),
            );
        }
    }
    output
}

fn run_bridge_and_score(case: &FixtureCase) {
    let collect = collect_from_input(&case.input);
    let raw_dims = collect.build_raw_dims();
    let mut snap = FundamentalsSnapshot {
        symbol: case.input["symbol"].as_str().unwrap_or("TEST").into(),
        ..Default::default()
    };
    apply_dims_to_snapshot(&mut snap, &collect);
    supplement_snapshot_confidence(&mut snap);
    let confidence = DataConfidence::from_snapshot(&snap);
    let scored = score_dimensions(&snap.symbol, &raw_dims, &snap, &AnalysisProfile::medium());
    let exp = &case.expected;

    if let Some(min) = exp.get("min_confidence").and_then(|v| v.as_f64()) {
        assert!(
            confidence.score >= min,
            "{} confidence {} < {min}",
            case.id,
            confidence.score
        );
    }
    if let Some(max) = exp.get("max_confidence").and_then(|v| v.as_f64()) {
        assert!(
            confidence.score <= max,
            "{} confidence {} > {max}",
            case.id,
            confidence.score
        );
    }
    if let Some(min) = exp.get("min_fundamental_score").and_then(|v| v.as_f64()) {
        assert!(
            scored.fundamental_score >= min,
            "{} score {} < {min}",
            case.id,
            scored.fundamental_score
        );
    }
    if let Some(max) = exp.get("max_fundamental_score").and_then(|v| v.as_f64()) {
        assert!(
            scored.fundamental_score <= max,
            "{} score {} > {max}",
            case.id,
            scored.fundamental_score
        );
    }
    if exp
        .get("has_industry_pe")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        assert!(
            snap.industry_pe.is_some(),
            "{} expected industry_pe",
            case.id
        );
    }
    if let Some(lte) = exp.get("pe_dim_score_lte").and_then(|v| v.as_u64()) {
        let pe_score = scored
            .dimensions
            .get("10_valuation")
            .map(|d| d.score)
            .unwrap_or(0);
        assert!(
            u64::from(pe_score) <= lte,
            "{} pe dim {pe_score} > {lte}",
            case.id
        );
    }
    if exp
        .get("has_missing_valuation_dim")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let val_dim = scored.dimensions.get("10_valuation").expect("10_valuation");
        assert!(
            val_dim
                .missing
                .iter()
                .any(|m| m == "pe_percentile" || m == "10_valuation"),
            "{} expected valuation missing flags, got {:?}",
            case.id,
            val_dim.missing
        );
    }
    if exp
        .get("has_missing_events_dim")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let events_dim = scored.dimensions.get("15_events").expect("15_events");
        assert!(
            events_dim
                .missing
                .iter()
                .any(|m| m == "announcements" || m == "news"),
            "{} expected events missing flags, got {:?}",
            case.id,
            events_dim.missing
        );
    }
    if let Some(eq) = exp.get("events_dim_score_eq").and_then(|v| v.as_u64()) {
        let score = scored
            .dimensions
            .get("15_events")
            .map(|d| d.score)
            .unwrap_or(0);
        assert_eq!(
            u64::from(score),
            eq,
            "{} events dim score {score} != {eq}",
            case.id
        );
    }
    if let Some(eq) = exp.get("trap_dim_score_eq").and_then(|v| v.as_u64()) {
        let score = scored
            .dimensions
            .get("18_trap")
            .map(|d| d.score)
            .unwrap_or(0);
        assert_eq!(
            u64::from(score),
            eq,
            "{} trap dim score {score} != {eq}",
            case.id
        );
    }
    if exp
        .get("has_missing_research_dim")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let research_dim = scored.dimensions.get("6_research").expect("6_research");
        assert!(
            research_dim
                .missing
                .iter()
                .any(|m| m == "research_reports" || m == "6_research"),
            "{} expected research missing flags, got {:?}",
            case.id,
            research_dim.missing
        );
    }
    if let Some(eq) = exp.get("research_dim_score_eq").and_then(|v| v.as_u64()) {
        let score = scored
            .dimensions
            .get("6_research")
            .map(|d| d.score)
            .unwrap_or(0);
        assert_eq!(
            u64::from(score),
            eq,
            "{} research dim score {score} != {eq}",
            case.id
        );
    }
    if exp
        .get("has_missing_lhb_dim")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let lhb_dim = scored.dimensions.get("16_lhb").expect("16_lhb");
        assert!(
            lhb_dim
                .missing
                .iter()
                .any(|m| m == "lhb_count_30d" || m == "16_lhb"),
            "{} expected lhb missing flags, got {:?}",
            case.id,
            lhb_dim.missing
        );
    }
    if let Some(eq) = exp.get("lhb_dim_score_eq").and_then(|v| v.as_u64()) {
        let score = scored
            .dimensions
            .get("16_lhb")
            .map(|d| d.score)
            .unwrap_or(0);
        assert_eq!(
            u64::from(score),
            eq,
            "{} lhb dim score {score} != {eq}",
            case.id
        );
    }
    if exp
        .get("has_matched_youzi")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        assert!(
            !snap.matched_youzi.is_empty(),
            "{} expected matched_youzi on snapshot",
            case.id
        );
    }
}

fn run_build_synthesis(case: &FixtureCase) {
    let collect = collect_from_input(&case.input);
    let raw_dims = collect.build_raw_dims();
    let mut snap = FundamentalsSnapshot {
        symbol: case.input["symbol"].as_str().unwrap_or("TEST").into(),
        ..Default::default()
    };
    apply_dims_to_snapshot(&mut snap, &collect);
    supplement_snapshot_confidence(&mut snap);
    let result = analyze_stock(
        &snap,
        Some(&raw_dims),
        None,
        &AnalysisProfile::medium(),
        Some(&collect),
    );
    let syn = &result.synthesis;
    let exp = &case.expected;

    if let Some(v) = exp.get("verdict").and_then(|x| x.as_str()) {
        assert_eq!(syn.verdict, v, "{} verdict", case.id);
    }
    if let Some(v) = exp.get("confidence_tier").and_then(|x| x.as_str()) {
        assert_eq!(syn.confidence_tier, v, "{} confidence_tier", case.id);
    }
    if let Some(min) = exp.get("min_panel_consensus").and_then(|x| x.as_f64()) {
        assert!(
            syn.panel_summary.consensus >= min,
            "{} panel consensus {} < {min}",
            case.id,
            syn.panel_summary.consensus
        );
    }
    if let Some(needle) = exp.get("headline_contains").and_then(|x| x.as_str()) {
        assert!(
            syn.headline.contains(needle),
            "{} headline missing {needle:?}: {}",
            case.id,
            syn.headline
        );
    }
    if let Some(needle) = exp.get("dcf_one_liner_contains").and_then(|x| x.as_str()) {
        assert!(
            syn.dcf_one_liner.contains(needle),
            "{} dcf_one_liner missing {needle:?}: {}",
            case.id,
            syn.dcf_one_liner
        );
    }
    if let Some(min) = exp.get("min_key_metrics").and_then(|x| x.as_u64()) {
        assert!(
            syn.key_metrics.len() >= min as usize,
            "{} key_metrics len {}",
            case.id,
            syn.key_metrics.len()
        );
    }
}

#[test]
fn quick_scan_profile_golden() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/trading_research_fetch/quick_scan_profile.json");
    let content = fs::read_to_string(&path).expect("read quick_scan_profile");
    let golden: Value = serde_json::from_str(&content).expect("parse");

    let lite = AnalysisProfile::lite();
    let medium = AnalysisProfile::medium();

    let expected_fetchers: Vec<&str> = golden["lite_fetchers"]
        .as_array()
        .expect("lite_fetchers")
        .iter()
        .map(|v| v.as_str().expect("fetcher str"))
        .collect();
    for key in &expected_fetchers {
        assert!(lite.should_run_fetcher(key), "lite missing fetcher {key}");
    }
    assert_eq!(expected_fetchers.len(), 8, "lite fetcher count");
    assert!(!lite.should_run_fetcher("3_macro"));

    let expected_investors: Vec<&str> = golden["lite_investors"]
        .as_array()
        .expect("lite_investors")
        .iter()
        .map(|v| v.as_str().expect("investor str"))
        .collect();
    assert_eq!(
        lite.lite_investor_ids().expect("lite ids"),
        expected_investors.as_slice()
    );

    assert!(!lite.run_comps_lbo_three_stmt);
    assert!(medium.run_comps_lbo_three_stmt);
    assert!(lite.allow_web_supplement);
    assert!(medium.allow_web_supplement);
}

#[test]
fn equity_research_fetcher_golden() {
    let content = fs::read_to_string(fixture_path()).expect("read fetcher golden");
    let fixture: FixtureFile = serde_json::from_str(&content).expect("parse");
    for case in &fixture.cases {
        match case.op.as_str() {
            "bridge_and_score" => run_bridge_and_score(case),
            "build_synthesis" => run_build_synthesis(case),
            other => panic!("unknown op {other}"),
        }
    }
}

#[test]
fn fetcher_golden_raw_dims_shape() {
    let collect = collect_from_input(&json!({
        "symbol": "600519.SH",
        "dims": { "0_basic": { "data": { "price": 1.0 } } }
    }));
    let raw = collect.build_raw_dims();
    assert!(raw.get("0_basic").and_then(|v| v.get("data")).is_some());
}
