//! Declarative strategy definition types and rule DSL parser.
//!
//! A `DeclarativeStrategyDef` is the JSON-serializable representation of a
//! user-defined strategy. It is validated, then compiled into a
//! `DeclarativeStrategy` for execution.

use serde::{Deserialize, Serialize};

use crate::error::StrategyError;

/// Known indicator type names.
const KNOWN_INDICATOR_TYPES: &[&str] = &["sma", "ema", "rsi", "macd", "bollinger"];

/// Default source for indicators that don't specify one.
fn default_source() -> String {
    "close".to_string()
}

/// Default version.
fn default_version() -> u32 {
    1
}

/// Default position sizing.
fn default_position_sizing() -> String {
    "full".to_string()
}

/// A complete declarative strategy definition, deserialized from JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeclarativeStrategyDef {
    /// Strategy name: must match `^[a-z][a-z0-9_]*$`.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Strategy version (default: 1).
    #[serde(default = "default_version")]
    pub version: u32,
    /// Author identifier: "builtin" or "user".
    #[serde(default)]
    pub author: String,
    /// Indicator declarations (at least 1).
    pub indicators: Vec<IndicatorDef>,
    /// Buy/sell rules (at least one must be present).
    pub rules: RulesDef,
    /// Default parameter values for `run_backtest`.
    #[serde(default)]
    pub default_params: serde_json::Value,
    /// Position sizing: "full", "half", "quarter".
    #[serde(default = "default_position_sizing")]
    pub position_sizing: String,
    /// Market rules, e.g. ["t+0"].
    #[serde(default)]
    pub market_rules: Vec<String>,
}

/// A single indicator declaration within a strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorDef {
    /// Identifier used in rules to reference this indicator's output.
    pub id: String,
    /// Indicator type: "sma", "ema", "rsi", "macd", "bollinger".
    #[serde(rename = "type")]
    pub indicator_type: String,
    /// Indicator-specific parameters.
    pub params: serde_json::Value,
    /// Input source: "close" (default) or the id of a previously declared indicator.
    #[serde(default = "default_source")]
    pub source: String,
}

/// Buy/sell rule definitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesDef {
    /// Buy rule expression (at least one of buy/sell must be present).
    pub buy: Option<String>,
    /// Sell rule expression.
    pub sell: Option<String>,
}

// ── Rule AST ───────────────────────────────────────────────────────────

/// A parsed rule expression.
#[derive(Debug, Clone)]
pub enum RuleExpr {
    /// `left` crosses above `right` (prev_left <= prev_right && cur_left > cur_right).
    CrossesAbove { left: String, right: RuleOperand },
    /// `left` crosses below `right` (prev_left >= prev_right && cur_left < cur_right).
    CrossesBelow { left: String, right: RuleOperand },
    /// `left` is above `right` (cur_left > right_value).
    Above { left: String, right: RuleOperand },
    /// `left` is below `right` (cur_left < right_value).
    Below { left: String, right: RuleOperand },
}

/// The right-hand side of a rule expression: either an indicator id or a literal number.
#[derive(Debug, Clone)]
pub enum RuleOperand {
    /// Reference to another indicator by id.
    Indicator(String),
    /// A literal numeric value.
    Literal(f64),
}

// ── Parsing ────────────────────────────────────────────────────────────

/// Parse a rule expression string into a `RuleExpr`.
///
/// Supported syntax: `<indicator_id> <operator> <indicator_id|number>`
/// Operators: `crosses_above`, `crosses_below`, `above`, `below`
pub fn parse_rule(expr: &str) -> Result<RuleExpr, StrategyError> {
    let parts: Vec<&str> = expr.trim().splitn(3, ' ').collect();
    if parts.len() != 3 {
        return Err(StrategyError::InvalidRuleExpression(format!(
            "Rule must be '<left> <operator> <right>', got: '{expr}'"
        )));
    }

    let left = parts[0].to_string();
    let op = parts[1];
    let right_str = parts[2];

    // Try parsing right as a number first, then as an indicator id.
    let right = match right_str.parse::<f64>() {
        Ok(n) => RuleOperand::Literal(n),
        Err(_) => {
            // Validate it looks like an identifier (no spaces, not empty).
            if right_str.is_empty() || right_str.contains(' ') {
                return Err(StrategyError::InvalidRuleExpression(format!(
                    "Invalid right operand: '{right_str}'"
                )));
            }
            RuleOperand::Indicator(right_str.to_string())
        }
    };

    match op {
        "crosses_above" => Ok(RuleExpr::CrossesAbove { left, right }),
        "crosses_below" => Ok(RuleExpr::CrossesBelow { left, right }),
        "above" => Ok(RuleExpr::Above { left, right }),
        "below" => Ok(RuleExpr::Below { left, right }),
        _ => Err(StrategyError::InvalidRuleExpression(format!(
            "Unknown operator '{op}'. Supported: crosses_above, crosses_below, above, below"
        ))),
    }
}

// ── Validation ─────────────────────────────────────────────────────────

impl DeclarativeStrategyDef {
    /// Validate the strategy definition for completeness and correctness.
    ///
    /// Checks:
    /// - name matches `^[a-z][a-z0-9_]*$`
    /// - at least 1 indicator
    /// - indicator ids are unique
    /// - indicator types are known
    /// - source is "close" or references a previously declared indicator id
    /// - at least one of buy/sell rules is present
    /// - rules parse correctly and reference existing indicator ids
    pub fn validate(&self) -> Result<(), StrategyError> {
        // Name format: must match ^[a-z][a-z0-9_]*$
        if self.name.is_empty()
            || !self
                .name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
        {
            return Err(StrategyError::DefinitionError(format!(
                "Strategy name '{}' must start with a lowercase letter",
                self.name
            )));
        }
        if !self
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(StrategyError::DefinitionError(format!(
                "Strategy name '{}' must contain only lowercase letters, digits, and underscores",
                self.name
            )));
        }

        // At least 1 indicator.
        if self.indicators.is_empty() {
            return Err(StrategyError::DefinitionError(
                "Strategy must have at least 1 indicator".into(),
            ));
        }

        // Collect declared ids, check uniqueness and types.
        let mut declared_ids = vec!["close".to_string()]; // "close" is always available.
        for (i, ind) in self.indicators.iter().enumerate() {
            // Id uniqueness.
            if declared_ids.contains(&ind.id) {
                return Err(StrategyError::DefinitionError(format!(
                    "Duplicate indicator id: '{}'",
                    ind.id
                )));
            }

            // Type is known.
            if !KNOWN_INDICATOR_TYPES.contains(&ind.indicator_type.as_str()) {
                return Err(StrategyError::UnknownIndicatorType(format!(
                    "Unknown indicator type '{}' for indicator '{}' (index {i})",
                    ind.indicator_type, ind.id
                )));
            }

            // Source references a previously declared id (or "close").
            if !declared_ids.contains(&ind.source) {
                return Err(StrategyError::DefinitionError(format!(
                    "Indicator '{}' references source '{}' which is not yet declared \
                     (only 'close' or previously declared indicator ids are allowed)",
                    ind.id, ind.source
                )));
            }

            declared_ids.push(ind.id.clone());
        }

        // At least one rule.
        if self.rules.buy.is_none() && self.rules.sell.is_none() {
            return Err(StrategyError::DefinitionError(
                "Strategy must have at least one buy or sell rule".into(),
            ));
        }

        // Parse and validate rules.
        if let Some(ref buy_expr) = self.rules.buy {
            let rule = parse_rule(buy_expr)?;
            validate_rule_refs(&rule, &declared_ids)?;
        }
        if let Some(ref sell_expr) = self.rules.sell {
            let rule = parse_rule(sell_expr)?;
            validate_rule_refs(&rule, &declared_ids)?;
        }

        Ok(())
    }
}

/// Validate that a rule's indicator references exist in the declared ids.
fn validate_rule_refs(rule: &RuleExpr, declared_ids: &[String]) -> Result<(), StrategyError> {
    match rule {
        RuleExpr::CrossesAbove { left, right }
        | RuleExpr::CrossesBelow { left, right }
        | RuleExpr::Above { left, right }
        | RuleExpr::Below { left, right } => {
            if !declared_ids.contains(left) {
                return Err(StrategyError::DefinitionError(format!(
                    "Rule references unknown indicator '{left}'"
                )));
            }
            if let RuleOperand::Indicator(id) = right
                && !declared_ids.contains(id)
            {
                return Err(StrategyError::DefinitionError(format!(
                    "Rule references unknown indicator '{id}'"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rule_crosses_above() {
        let rule = parse_rule("rsi_val crosses_above 30").unwrap();
        assert!(matches!(
            rule,
            RuleExpr::CrossesAbove {
                left,
                right: RuleOperand::Literal(30.0)
            } if left == "rsi_val"
        ));
    }

    #[test]
    fn test_parse_rule_crosses_below_indicator() {
        let rule = parse_rule("sma_short crosses_below sma_long").unwrap();
        assert!(matches!(
            rule,
            RuleExpr::CrossesBelow {
                left,
                right: RuleOperand::Indicator(id)
            } if left == "sma_short" && id == "sma_long"
        ));
    }

    #[test]
    fn test_parse_rule_above() {
        let rule = parse_rule("rsi_val above 70").unwrap();
        assert!(matches!(rule, RuleExpr::Above { .. }));
    }

    #[test]
    fn test_parse_rule_below() {
        let rule = parse_rule("rsi_val below 30").unwrap();
        assert!(matches!(rule, RuleExpr::Below { .. }));
    }

    #[test]
    fn test_parse_rule_invalid_operator() {
        let result = parse_rule("rsi_val near 30");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rule_too_few_parts() {
        let result = parse_rule("rsi_val");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_sma_cross() {
        let def = DeclarativeStrategyDef {
            name: "sma_cross".to_string(),
            description: "SMA crossover".to_string(),
            version: 1,
            author: "builtin".to_string(),
            indicators: vec![
                IndicatorDef {
                    id: "sma_short".into(),
                    indicator_type: "sma".into(),
                    params: serde_json::json!({"period": 20}),
                    source: "close".into(),
                },
                IndicatorDef {
                    id: "sma_long".into(),
                    indicator_type: "sma".into(),
                    params: serde_json::json!({"period": 50}),
                    source: "close".into(),
                },
            ],
            rules: RulesDef {
                buy: Some("sma_short crosses_above sma_long".into()),
                sell: Some("sma_short crosses_below sma_long".into()),
            },
            default_params: serde_json::json!({"short_window": 20, "long_window": 50}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        assert!(def.validate().is_ok());
    }

    #[test]
    fn test_validate_bad_name() {
        let def = DeclarativeStrategyDef {
            name: "Bad-Name".to_string(),
            description: "test".to_string(),
            version: 1,
            author: String::new(),
            indicators: vec![IndicatorDef {
                id: "s".into(),
                indicator_type: "sma".into(),
                params: serde_json::json!({"period": 10}),
                source: "close".into(),
            }],
            rules: RulesDef {
                buy: Some("s above 1".into()),
                sell: None,
            },
            default_params: serde_json::json!({}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        assert!(def.validate().is_err());
    }

    #[test]
    fn test_validate_unknown_indicator_type() {
        let def = DeclarativeStrategyDef {
            name: "test_strat".to_string(),
            description: "test".to_string(),
            version: 1,
            author: String::new(),
            indicators: vec![IndicatorDef {
                id: "x".into(),
                indicator_type: "unknown_type".into(),
                params: serde_json::json!({}),
                source: "close".into(),
            }],
            rules: RulesDef {
                buy: Some("x above 1".into()),
                sell: None,
            },
            default_params: serde_json::json!({}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        let err = def.validate().unwrap_err();
        assert!(matches!(err, StrategyError::UnknownIndicatorType(_)));
    }

    #[test]
    fn test_validate_forward_reference_source() {
        let def = DeclarativeStrategyDef {
            name: "test_strat".to_string(),
            description: "test".to_string(),
            version: 1,
            author: String::new(),
            indicators: vec![
                IndicatorDef {
                    id: "a".into(),
                    indicator_type: "sma".into(),
                    params: serde_json::json!({"period": 10}),
                    source: "b".into(), // forward reference → error
                },
                IndicatorDef {
                    id: "b".into(),
                    indicator_type: "sma".into(),
                    params: serde_json::json!({"period": 20}),
                    source: "close".into(),
                },
            ],
            rules: RulesDef {
                buy: Some("a above 1".into()),
                sell: None,
            },
            default_params: serde_json::json!({}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        assert!(def.validate().is_err());
    }

    #[test]
    fn test_validate_no_rules() {
        let def = DeclarativeStrategyDef {
            name: "test_strat".to_string(),
            description: "test".to_string(),
            version: 1,
            author: String::new(),
            indicators: vec![IndicatorDef {
                id: "s".into(),
                indicator_type: "sma".into(),
                params: serde_json::json!({"period": 10}),
                source: "close".into(),
            }],
            rules: RulesDef {
                buy: None,
                sell: None,
            },
            default_params: serde_json::json!({}),
            position_sizing: "full".into(),
            market_rules: vec![],
        };
        assert!(def.validate().is_err());
    }
}
