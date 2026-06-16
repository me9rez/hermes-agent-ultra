//! Trading parity tests.
//!
//! Loads golden fixtures from `crates/hermes-parity-tests/fixtures/trading_*/`
//! and validates Rust `hermes-trading` output against the expected shape.
//!
//! All network calls are replaced by `MockProvider`, so these tests are
//! deterministic and do not require API keys.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde_json::Value;

use hermes_trading::{
    AutoRouter, BacktestEngine, DataSource, Interval, MockProvider, OhlcvRequest,
};

/// A single fixture file containing one or more cases.
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

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn load_fixtures() -> Vec<(PathBuf, FixtureFile)> {
    let dir = fixture_dir();
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }

    for group_entry in fs::read_dir(&dir).expect("read fixtures dir") {
        let group_entry = group_entry.expect("valid dir entry");
        let group_path = group_entry.path();
        if !group_path.is_dir() {
            continue;
        }
        let name = group_path.file_name().unwrap().to_string_lossy();
        if !name.starts_with("trading_") {
            continue;
        }
        for file_entry in fs::read_dir(&group_path).expect("read fixture group") {
            let file_entry = file_entry.expect("valid file entry");
            let path = file_entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(&path).expect("read fixture file");
            let fixture: FixtureFile = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            out.push((path, fixture));
        }
    }
    out
}

fn parse_date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .unwrap_or_else(|e| panic!("invalid date '{value}': {e}"))
}

fn request_from_input(input: &Value) -> OhlcvRequest {
    let end_date = input
        .get("end_date")
        .and_then(|v| v.as_str())
        .map(parse_date)
        .unwrap_or_else(|| chrono::Utc::now().date_naive());
    let start_days_back = if input.get("strategy").is_some() {
        180
    } else {
        30
    };
    let start_date = input
        .get("start_date")
        .and_then(|v| v.as_str())
        .map(parse_date)
        .unwrap_or_else(|| end_date - chrono::Duration::days(start_days_back));
    let interval = match input.get("interval").and_then(|v| v.as_str()) {
        Some("weekly") => Interval::Weekly,
        _ => Interval::Daily,
    };

    OhlcvRequest {
        symbol: input
            .get("symbol")
            .and_then(|v| v.as_str())
            .expect("symbol required")
            .to_string(),
        start: start_date,
        end: end_date,
        interval,
    }
}

fn mock_router() -> AutoRouter {
    let mock = MockProvider::new();
    AutoRouter::with_providers(mock.clone(), mock)
}

fn source_from_input(input: &Value) -> Result<DataSource, String> {
    match input.get("source").and_then(|v| v.as_str()) {
        None => Ok(DataSource::Auto),
        Some(s) => DataSource::parse(s).map_err(|e| e.to_string()),
    }
}

async fn run_case(case: &FixtureCase) -> Result<Value, String> {
    match case.op.as_str() {
        "get_market_data" => {
            let req = request_from_input(&case.input);
            let source = source_from_input(&case.input)?;
            let refresh = case
                .input
                .get("refresh")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let data = mock_router()
                .fetch_ohlcv_with_source(&req, source, refresh)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_value(&data).map_err(|e| e.to_string())
        }
        "run_backtest" => {
            let req = request_from_input(&case.input);
            let source = source_from_input(&case.input)?;
            let refresh = case
                .input
                .get("refresh")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let data = mock_router()
                .fetch_ohlcv_with_source(&req, source, refresh)
                .await
                .map_err(|e| e.to_string())?;
            let strategy = case
                .input
                .get("strategy")
                .and_then(|v| v.as_str())
                .expect("strategy required");
            let params = case.input.get("params").cloned().unwrap_or_default();
            let card = BacktestEngine::run(&data, strategy, &params).map_err(|e| e.to_string())?;
            serde_json::to_value(&card).map_err(|e| e.to_string())
        }
        other => Err(format!("unknown op: {other}")),
    }
}

fn assert_expected(case_id: &str, actual: &Value, expected: &Value) {
    if let Some(symbol) = expected.get("has_symbol").and_then(|v| v.as_str()) {
        let actual_symbol = actual
            .get("symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>");
        assert_eq!(actual_symbol, symbol, "[{case_id}] symbol mismatch");
    }

    if let Some(interval) = expected.get("has_interval").and_then(|v| v.as_str()) {
        let actual_interval = actual
            .get("interval")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>");
        assert_eq!(actual_interval, interval, "[{case_id}] interval mismatch");
    }

    if let Some(partial) = expected.get("partial_eq").and_then(|v| v.as_bool()) {
        let actual_partial = actual
            .get("partial")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert_eq!(actual_partial, partial, "[{case_id}] partial mismatch");
    }

    if let Some(min_rows) = expected.get("min_rows").and_then(|v| v.as_u64()) {
        let rows = actual
            .get("rows")
            .and_then(|v| v.as_array())
            .expect("rows array");
        assert!(
            rows.len() >= min_rows as usize,
            "[{case_id}] expected at least {min_rows} rows, got {}",
            rows.len()
        );
    }

    if let Some(columns) = expected.get("columns").and_then(|v| v.as_array()) {
        let cols: BTreeSet<String> = columns
            .iter()
            .map(|v| v.as_str().expect("column name string").to_string())
            .collect();
        let rows = actual
            .get("rows")
            .and_then(|v| v.as_array())
            .expect("rows array");
        let first = rows.first().expect("at least one row");
        let first_obj = first.as_object().expect("row object");
        for col in &cols {
            assert!(
                first_obj.contains_key(col),
                "[{case_id}] missing column '{col}'"
            );
        }
    }

    if let Some(fields) = expected.get("has_fields").and_then(|v| v.as_array()) {
        let obj = actual.as_object().expect("run_card object");
        for field in fields {
            let field = field.as_str().expect("field name string");
            assert!(
                obj.contains_key(field),
                "[{case_id}] missing field '{field}'"
            );
        }
    }

    if let Some(strategy) = expected.get("strategy_eq").and_then(|v| v.as_str()) {
        let actual_strategy = actual
            .get("strategy")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>");
        assert_eq!(actual_strategy, strategy, "[{case_id}] strategy mismatch");
    }

    if let Some(min_trades) = expected.get("trade_count_gte").and_then(|v| v.as_u64()) {
        let trade_count = actual
            .get("trade_count")
            .and_then(|v| v.as_u64())
            .expect("trade_count");
        assert!(
            trade_count >= min_trades,
            "[{case_id}] expected trade_count >= {min_trades}, got {trade_count}"
        );
    }

    if let Some(max_dd) = expected.get("max_drawdown_lte").and_then(|v| v.as_f64()) {
        let actual_dd = actual
            .get("max_drawdown_pct")
            .and_then(|v| v.as_f64())
            .expect("max_drawdown_pct");
        assert!(
            actual_dd <= max_dd,
            "[{case_id}] expected max_drawdown <= {max_dd}, got {actual_dd}"
        );
    }
}

#[tokio::test]
async fn run_all_trading_fixtures() {
    let fixtures = load_fixtures();
    assert!(
        !fixtures.is_empty(),
        "no trading fixture files found under {:?}",
        fixture_dir()
    );

    let mut ran = 0;
    let mut skipped = 0;

    for (_path, fixture) in fixtures {
        for case in fixture.cases {
            if case.skip {
                skipped += 1;
                continue;
            }

            let result = run_case(&case).await;
            match result {
                Ok(actual) => {
                    if let Some(error_contains) =
                        case.expected.get("error_contains").and_then(|v| v.as_str())
                    {
                        panic!(
                            "[{case_id}] expected error containing '{error_contains}', but succeeded: {actual}",
                            case_id = case.id
                        );
                    }
                    assert_expected(&case.id, &actual, &case.expected);
                }
                Err(err) => {
                    if let Some(error_contains) =
                        case.expected.get("error_contains").and_then(|v| v.as_str())
                    {
                        assert!(
                            err.to_lowercase().contains(&error_contains.to_lowercase()),
                            "[{case_id}] expected error containing '{error_contains}', got: {err}",
                            case_id = case.id
                        );
                    } else {
                        panic!("[{case_id}] unexpected error: {err}", case_id = case.id);
                    }
                }
            }
            ran += 1;
        }
    }

    assert!(
        ran > 0,
        "no trading cases were run (all skipped); fixtures loaded from {:?}",
        fixture_dir()
    );
    println!("trading parity: {ran} cases ran, {skipped} skipped");
}
