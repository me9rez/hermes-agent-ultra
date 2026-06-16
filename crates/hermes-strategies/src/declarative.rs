//! Declarative strategy execution engine.
//!
//! A `DeclarativeStrategy` is compiled from a `DeclarativeStrategyDef` (parsed
//! from JSON). It instantiates concrete `Indicator` implementations, computes
//! indicator series, evaluates buy/sell rules, and produces `Decision`s.

use std::collections::HashMap;

use hermes_trading::types::OhlcvData;

use crate::dsl::{DeclarativeStrategyDef, IndicatorDef, RuleExpr, RuleOperand};
use crate::error::StrategyError;
use crate::indicators::{Bollinger, Ema, Indicator, Macd, Rsi, Sma};
use crate::strategy::{Decision, Position, Signal, Strategy};

/// A compiled declarative strategy, ready for execution.
pub struct DeclarativeStrategy {
    def: DeclarativeStrategyDef,
    buy_rule: Option<RuleExpr>,
    sell_rule: Option<RuleExpr>,
    /// (id, indicator, source) — in declaration order.
    indicator_builders: Vec<(String, Box<dyn Indicator>, String)>,
}

impl DeclarativeStrategy {
    /// Compile a strategy from its definition.
    ///
    /// This validates the definition, parses rules, and instantiates indicators.
    pub fn from_def(def: DeclarativeStrategyDef) -> Result<Self, StrategyError> {
        // Validate first.
        def.validate()?;

        // Parse rules.
        let buy_rule = def
            .rules
            .buy
            .as_deref()
            .map(crate::dsl::parse_rule)
            .transpose()?;
        let sell_rule = def
            .rules
            .sell
            .as_deref()
            .map(crate::dsl::parse_rule)
            .transpose()?;

        // Instantiate indicators.
        let mut indicator_builders = Vec::with_capacity(def.indicators.len());
        for ind_def in &def.indicators {
            let indicator = instantiate_indicator(ind_def)?;
            indicator_builders.push((ind_def.id.clone(), indicator, ind_def.source.clone()));
        }

        Ok(Self {
            def,
            buy_rule,
            sell_rule,
            indicator_builders,
        })
    }

    /// Reference to the underlying definition.
    pub fn def(&self) -> &DeclarativeStrategyDef {
        &self.def
    }
}

impl Strategy for DeclarativeStrategy {
    fn run(&self, data: &OhlcvData) -> Result<Vec<Decision>, StrategyError> {
        let closes: Vec<f64> = data.rows.iter().map(|r| r.close).collect();
        let n = closes.len();

        // 1. Compute each indicator series, in declaration order.
        //    Store results by id for rule evaluation.
        let mut series_map: HashMap<String, Vec<Option<f64>>> = HashMap::new();
        for (id, indicator, source) in &self.indicator_builders {
            let input: Vec<f64> = if source == "close" {
                closes.clone()
            } else {
                // Use the output of a previously computed indicator.
                // Fix 10: Log warning if source is not found instead of silently falling back.
                series_map
                    .get(source)
                    .map(|s| s.iter().map(|v| v.unwrap_or(0.0)).collect())
                    .unwrap_or_else(|| {
                        tracing::warn!(indicator = %id, source = %source, "Chained source not found, falling back to close prices");
                        closes.clone()
                    })
            };
            let series = indicator.compute_series(&input);
            series_map.insert(id.clone(), series);
        }

        // 2. Evaluate rules per bar.
        let mut decisions = Vec::with_capacity(n);
        for i in 0..n {
            let (buy_signal, sell_signal) =
                evaluate_rules_at_bar(i, &self.buy_rule, &self.sell_rule, &series_map);

            let signal = if buy_signal {
                Signal::Buy
            } else if sell_signal {
                Signal::Sell
            } else {
                Signal::Hold
            };

            decisions.push(Decision {
                signal,
                position: Position::Flat, // simplified: position tracking not in strategy layer
                confidence: if buy_signal || sell_signal { 1.0 } else { 0.0 },
                reason: if buy_signal {
                    "Buy rule triggered".into()
                } else if sell_signal {
                    "Sell rule triggered".into()
                } else {
                    String::new()
                },
            });
        }

        Ok(decisions)
    }

    fn name(&self) -> &str {
        &self.def.name
    }
}

/// Instantiate an `Indicator` from its definition.
fn instantiate_indicator(def: &IndicatorDef) -> Result<Box<dyn Indicator>, StrategyError> {
    match def.indicator_type.as_str() {
        "sma" => {
            let period = def
                .params
                .get("period")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    StrategyError::InvalidParams(format!(
                        "SMA indicator '{}' missing 'period' param",
                        def.id
                    ))
                })? as usize;
            // Fix 7: Validate period > 0 to prevent division by zero.
            if period == 0 {
                return Err(StrategyError::InvalidParams(format!(
                    "SMA indicator '{}' period must be > 0",
                    def.id
                )));
            }
            Ok(Box::new(Sma::new(period)))
        }
        "ema" => {
            let period = def
                .params
                .get("period")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    StrategyError::InvalidParams(format!(
                        "EMA indicator '{}' missing 'period' param",
                        def.id
                    ))
                })? as usize;
            // Fix 7: Validate period > 0.
            if period == 0 {
                return Err(StrategyError::InvalidParams(format!(
                    "EMA indicator '{}' period must be > 0",
                    def.id
                )));
            }
            Ok(Box::new(Ema::new(period)))
        }
        "rsi" => {
            let period = def
                .params
                .get("period")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    StrategyError::InvalidParams(format!(
                        "RSI indicator '{}' missing 'period' param",
                        def.id
                    ))
                })? as usize;
            // Fix 7: Validate period > 0.
            if period == 0 {
                return Err(StrategyError::InvalidParams(format!(
                    "RSI indicator '{}' period must be > 0",
                    def.id
                )));
            }
            Ok(Box::new(Rsi::new(period)))
        }
        "macd" => {
            let fast = def
                .params
                .get("fast")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    StrategyError::InvalidParams(format!(
                        "MACD indicator '{}' missing 'fast' param",
                        def.id
                    ))
                })? as usize;
            let slow = def
                .params
                .get("slow")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    StrategyError::InvalidParams(format!(
                        "MACD indicator '{}' missing 'slow' param",
                        def.id
                    ))
                })? as usize;
            Ok(Box::new(Macd::new(fast, slow)))
        }
        "bollinger" => {
            let period = def
                .params
                .get("period")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| {
                    StrategyError::InvalidParams(format!(
                        "Bollinger indicator '{}' missing 'period' param",
                        def.id
                    ))
                })? as usize;
            let num_std = def
                .params
                .get("num_std")
                .and_then(|v| v.as_f64())
                .unwrap_or(2.0);
            Ok(Box::new(Bollinger::new(period, num_std)))
        }
        other => Err(StrategyError::UnknownIndicatorType(other.to_string())),
    }
}

/// Evaluate buy/sell rules at a single bar index.
///
/// Returns `(buy_triggered, sell_triggered)`.
fn evaluate_rules_at_bar(
    bar_index: usize,
    buy_rule: &Option<RuleExpr>,
    sell_rule: &Option<RuleExpr>,
    series_map: &HashMap<String, Vec<Option<f64>>>,
) -> (bool, bool) {
    let buy = buy_rule
        .as_ref()
        .map(|r| evaluate_rule_at_bar(r, bar_index, series_map))
        .unwrap_or(false);
    let sell = sell_rule
        .as_ref()
        .map(|r| evaluate_rule_at_bar(r, bar_index, series_map))
        .unwrap_or(false);
    (buy, sell)
}

/// Evaluate a single rule at a bar index.
fn evaluate_rule_at_bar(
    rule: &RuleExpr,
    bar_index: usize,
    series_map: &HashMap<String, Vec<Option<f64>>>,
) -> bool {
    match rule {
        RuleExpr::CrossesAbove { left, right } => {
            // Fix 3: Require bar_index > 0 for cross detection (need previous bar).
            if bar_index == 0 {
                return false;
            }
            let prev = get_value(left, bar_index - 1, series_map);
            let cur = get_value(left, bar_index, series_map);
            let prev_right = get_operand_value(right, bar_index - 1, series_map);
            let cur_right = get_operand_value(right, bar_index, series_map);
            match (prev, cur, prev_right, cur_right) {
                (Some(p), Some(c), Some(pr), Some(cr)) => p <= pr && c > cr,
                _ => false,
            }
        }
        RuleExpr::CrossesBelow { left, right } => {
            // Fix 3: Require bar_index > 0 for cross detection (need previous bar).
            if bar_index == 0 {
                return false;
            }
            let prev = get_value(left, bar_index - 1, series_map);
            let cur = get_value(left, bar_index, series_map);
            let prev_right = get_operand_value(right, bar_index - 1, series_map);
            let cur_right = get_operand_value(right, bar_index, series_map);
            match (prev, cur, prev_right, cur_right) {
                (Some(p), Some(c), Some(pr), Some(cr)) => p >= pr && c < cr,
                _ => false,
            }
        }
        RuleExpr::Above { left, right } => {
            let cur = get_value(left, bar_index, series_map);
            let cur_right = get_operand_value(right, bar_index, series_map);
            match (cur, cur_right) {
                (Some(c), Some(cr)) => c > cr,
                _ => false,
            }
        }
        RuleExpr::Below { left, right } => {
            let cur = get_value(left, bar_index, series_map);
            let cur_right = get_operand_value(right, bar_index, series_map);
            match (cur, cur_right) {
                (Some(c), Some(cr)) => c < cr,
                _ => false,
            }
        }
    }
}

/// Get the value of an indicator at a specific bar index.
fn get_value(
    id: &str,
    bar_index: usize,
    series_map: &HashMap<String, Vec<Option<f64>>>,
) -> Option<f64> {
    series_map
        .get(id)
        .and_then(|s| s.get(bar_index).copied().flatten())
}

/// Get the value of a rule operand (indicator or literal) at a bar index.
fn get_operand_value(
    operand: &RuleOperand,
    bar_index: usize,
    series_map: &HashMap<String, Vec<Option<f64>>>,
) -> Option<f64> {
    match operand {
        RuleOperand::Indicator(id) => get_value(id, bar_index, series_map),
        RuleOperand::Literal(v) => Some(*v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{IndicatorDef, RulesDef};
    use chrono::NaiveDate;
    use hermes_trading::types::{Interval, OhlcvRow};

    fn mock_ohlcv(days: usize) -> OhlcvData {
        let base_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let mut rows = Vec::with_capacity(days);
        for i in 0..days {
            let t = i as f64;
            let price = 100.0 + 0.1 * t + 15.0 * (t * std::f64::consts::PI / 30.0).sin();
            rows.push(OhlcvRow {
                date: base_date + chrono::Duration::days(i as i64),
                open: price * 0.999,
                high: price * 1.01,
                low: price * 0.99,
                close: price,
                volume: 1_000_000.0,
            });
        }
        OhlcvData {
            symbol: "MOCK-USD".to_string(),
            interval: Interval::Daily,
            rows,
            partial: false,
        }
    }

    #[test]
    fn test_declarative_sma_cross() {
        let def = DeclarativeStrategyDef {
            name: "sma_cross".into(),
            description: "SMA crossover".into(),
            version: 1,
            author: "builtin".into(),
            indicators: vec![
                IndicatorDef {
                    id: "sma_short".into(),
                    indicator_type: "sma".into(),
                    params: serde_json::json!({"period": 5}),
                    source: "close".into(),
                },
                IndicatorDef {
                    id: "sma_long".into(),
                    indicator_type: "sma".into(),
                    params: serde_json::json!({"period": 10}),
                    source: "close".into(),
                },
            ],
            rules: RulesDef {
                buy: Some("sma_short crosses_above sma_long".into()),
                sell: Some("sma_short crosses_below sma_long".into()),
            },
            default_params: serde_json::json!({}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        let strategy = DeclarativeStrategy::from_def(def).unwrap();
        let data = mock_ohlcv(120);
        let decisions = strategy.run(&data).unwrap();
        assert_eq!(decisions.len(), 120);
        let buy_count = decisions.iter().filter(|d| d.signal == Signal::Buy).count();
        let sell_count = decisions
            .iter()
            .filter(|d| d.signal == Signal::Sell)
            .count();
        assert!(buy_count > 0, "Expected at least 1 buy signal");
        assert!(sell_count > 0, "Expected at least 1 sell signal");
    }

    #[test]
    fn test_declarative_rsi_revert() {
        let def = DeclarativeStrategyDef {
            name: "rsi_revert".into(),
            description: "RSI mean reversion".into(),
            version: 1,
            author: "builtin".into(),
            indicators: vec![IndicatorDef {
                id: "rsi_val".into(),
                indicator_type: "rsi".into(),
                params: serde_json::json!({"period": 14}),
                source: "close".into(),
            }],
            rules: RulesDef {
                buy: Some("rsi_val crosses_above 30".into()),
                sell: Some("rsi_val crosses_below 70".into()),
            },
            default_params: serde_json::json!({}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        let strategy = DeclarativeStrategy::from_def(def).unwrap();
        let data = mock_ohlcv(120);
        let decisions = strategy.run(&data).unwrap();
        assert_eq!(decisions.len(), 120);
    }

    #[test]
    fn test_declarative_chained_indicator() {
        // MACD line → EMA signal line
        let def = DeclarativeStrategyDef {
            name: "macd_cross".into(),
            description: "MACD crossover".into(),
            version: 1,
            author: "user".into(),
            indicators: vec![
                IndicatorDef {
                    id: "macd_line".into(),
                    indicator_type: "macd".into(),
                    params: serde_json::json!({"fast": 12, "slow": 26}),
                    source: "close".into(),
                },
                IndicatorDef {
                    id: "signal_line".into(),
                    indicator_type: "ema".into(),
                    params: serde_json::json!({"period": 9}),
                    source: "macd_line".into(), // chained!
                },
            ],
            rules: RulesDef {
                buy: Some("macd_line crosses_above signal_line".into()),
                sell: Some("macd_line crosses_below signal_line".into()),
            },
            default_params: serde_json::json!({}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        let strategy = DeclarativeStrategy::from_def(def).unwrap();
        let data = mock_ohlcv(120);
        let decisions = strategy.run(&data).unwrap();
        assert_eq!(decisions.len(), 120);
    }
}
