//! Equity research parity tests (UZI fin_models golden fixtures).

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use hermes_trading::research::models::{
    CompsPeer, CompsTarget, ThreeStmtResult, build_comps_table, compute_dcf, compute_wacc,
    project_three_stmt, quick_lbo,
};
use hermes_trading::research::profile::AnalysisProfile;
use hermes_trading::research::scoring::{generate_panel, score_dimensions};
use hermes_trading::research::types::FeatureVector;

#[derive(Debug, serde::Deserialize)]
struct FixtureFile {
    #[allow(dead_code)]
    schema_version: u32,
    #[allow(dead_code)]
    fixture_group: String,
    cases: Vec<FixtureCase>,
}

#[derive(Debug, serde::Deserialize)]
struct FixtureCase {
    id: String,
    op: String,
    input: Value,
    expected: Value,
    #[serde(default)]
    skip: bool,
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/trading_research/models_golden.json")
}

fn load_fixtures() -> FixtureFile {
    let content = fs::read_to_string(fixture_path()).expect("read models_golden.json");
    serde_json::from_str(&content).expect("parse fixture")
}

fn features_from(v: &Value) -> FeatureVector {
    let mut core = v.clone();
    if let Some(obj) = core.as_object_mut() {
        obj.remove("raw_dims");
    }
    let mut f: FeatureVector = serde_json::from_value(core).unwrap_or_else(|_| FeatureVector {
        symbol: v
            .get("symbol")
            .and_then(|s| s.as_str())
            .unwrap_or("TEST")
            .to_string(),
        ..Default::default()
    });
    if f.symbol.is_empty() {
        f.symbol = "TEST".into();
    }
    macro_rules! set_f64 {
        ($field:ident) => {
            if f.$field.is_none() {
                if let Some(n) = v.get(stringify!($field)).and_then(|x| x.as_f64()) {
                    f.$field = Some(n);
                }
            }
        };
    }
    set_f64!(price);
    set_f64!(market_cap_yi);
    set_f64!(shares_outstanding_yi);
    set_f64!(revenue_latest_yi);
    set_f64!(net_margin);
    set_f64!(total_debt_yi);
    set_f64!(cash_yi);
    set_f64!(fcf_latest_yi);
    set_f64!(ebitda_yi);
    set_f64!(equity_yi);
    if let Some(b) = v.get("fcf_positive").and_then(|x| x.as_bool()) {
        f.fcf_positive = Some(b);
    }
    if let Some(n) = v.get("debt_ratio").and_then(|x| x.as_f64()) {
        f.debt_ratio = Some(n);
    }
    if let Some(n) = v.get("pe_quantile_5y").and_then(|x| x.as_f64()) {
        f.pe_quantile_5y = Some(n);
    }
    if let Some(n) = v.get("roe_latest").and_then(|x| x.as_f64()) {
        f.roe_latest = Some(n);
    }
    if let Some(n) = v.get("revenue_growth_latest").and_then(|x| x.as_f64()) {
        f.revenue_growth_latest = Some(n);
    }
    if let Some(s) = v.get("stage").and_then(|x| x.as_str()) {
        f.stage = Some(s.to_string());
    }
    if let Some(s) = v.get("ma_align").and_then(|x| x.as_str()) {
        f.ma_align = Some(s.to_string());
    }
    if let Some(n) = v.get("change_pct").and_then(|x| x.as_f64()) {
        f.change_pct = Some(n);
    }
    if let Some(n) = v.get("pe").and_then(|x| x.as_f64()) {
        f.pe = Some(n);
    }
    f
}

fn approx_eq(a: f64, b: f64, tol_pct: f64) -> bool {
    if b == 0.0 {
        return a.abs() < 0.01;
    }
    ((a - b) / b).abs() <= tol_pct
}

fn run_case(case: &FixtureCase) {
    if case.skip {
        return;
    }
    match case.op.as_str() {
        "compute_wacc" => {
            let r = compute_wacc(None);
            let exp = case.expected["wacc"].as_f64().unwrap();
            assert!(
                approx_eq(r.wacc, exp, 0.01),
                "{} wacc: {} vs {}",
                case.id,
                r.wacc,
                exp
            );
        }
        "compute_dcf" => {
            let f = features_from(&case.input);
            let r = compute_dcf(&f, None);
            let exp = &case.expected;
            assert!(approx_eq(
                r.intrinsic_per_share,
                exp["intrinsic_per_share"].as_f64().unwrap(),
                0.01
            ));
            assert!(approx_eq(
                r.safety_margin_pct,
                exp["safety_margin_pct"].as_f64().unwrap(),
                0.05
            ));
            assert!(approx_eq(
                r.sensitivity_table.center_cell,
                exp["center_cell"].as_f64().unwrap(),
                0.01
            ));
        }
        "build_comps" => {
            let target: CompsTarget = serde_json::from_value(case.input["target"].clone()).unwrap();
            let peers: Vec<CompsPeer> =
                serde_json::from_value(case.input["peers"].clone()).unwrap();
            let r = build_comps_table(target, &peers);
            let hermes_trading::research::models::CompsResult::Ok(ok) = r else {
                panic!("{} expected comps ok", case.id);
            };
            let median = ok.peer_stats.get("pe").map(|s| s.median).unwrap();
            assert!(approx_eq(
                median,
                case.expected["median_pe"].as_f64().unwrap(),
                0.01
            ));
        }
        "quick_lbo" => {
            let f = features_from(&case.input);
            let r = quick_lbo(&f, None);
            assert!(approx_eq(
                r.irr_pct,
                case.expected["irr_pct"].as_f64().unwrap(),
                0.02
            ));
        }
        "project_three_stmt" => {
            let f = features_from(&case.input);
            let ThreeStmtResult::Ok(ok) = project_three_stmt(&f, None) else {
                panic!("{} three_stmt failed", case.id);
            };
            let y5 = ok.income_statement.net_income.last().copied().unwrap();
            assert!(approx_eq(
                y5,
                case.expected["y5_ni"].as_f64().unwrap(),
                0.02
            ));
        }
        "persona_panel" => {
            let f = features_from(&case.input);
            let raw = case
                .input
                .get("raw_dims")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let profile = AnalysisProfile::medium();
            let scored = score_dimensions(f.symbol.as_str(), &raw, &f, &profile);
            let p1 = generate_panel(&scored, &f, &profile);
            let p2 = generate_panel(&scored, &f, &profile);
            assert_eq!(
                p1.panel_consensus, p2.panel_consensus,
                "{} panel not deterministic",
                case.id
            );
            if let Some(exp) = case
                .expected
                .get("panel_consensus")
                .and_then(|v| v.as_f64())
            {
                assert!(
                    approx_eq(p1.panel_consensus, exp, 0.02),
                    "{} panel {} vs {}",
                    case.id,
                    p1.panel_consensus,
                    exp
                );
            }
            if let Some(max) = case
                .expected
                .get("panel_consensus_lte")
                .and_then(|v| v.as_f64())
            {
                assert!(
                    p1.panel_consensus <= max,
                    "{} panel {} > {max}",
                    case.id,
                    p1.panel_consensus
                );
            }
            if let Some(signal) = case.expected.get("buffett_signal").and_then(|v| v.as_str()) {
                let buffett = p1
                    .investors
                    .iter()
                    .find(|v| v.id == "buffett")
                    .expect("buffett vote");
                assert_eq!(
                    buffett.signal, signal,
                    "{} buffett signal {} != {signal}",
                    case.id, buffett.signal
                );
            }
            if let Some(max) = case
                .expected
                .get("buffett_score_lte")
                .and_then(|v| v.as_f64())
            {
                let buffett = p1
                    .investors
                    .iter()
                    .find(|v| v.id == "buffett")
                    .expect("buffett vote");
                assert!(
                    buffett.score <= max,
                    "{} buffett score {} > {max}",
                    case.id,
                    buffett.score
                );
            }
            if let Some(spec) = case.expected.get("investor_signal_eq") {
                let id = spec["id"].as_str().expect("investor id");
                let signal = spec["signal"].as_str().expect("signal");
                let vote = p1
                    .investors
                    .iter()
                    .find(|v| v.id == id)
                    .unwrap_or_else(|| panic!("{} missing investor {id}", case.id));
                assert_eq!(
                    vote.signal, signal,
                    "{} {id} signal {} != {signal}",
                    case.id, vote.signal
                );
            }
        }
        other => panic!("unknown op {other}"),
    }
}

#[test]
fn equity_research_models_parity() {
    let fixture = load_fixtures();
    for case in &fixture.cases {
        run_case(case);
    }
}
