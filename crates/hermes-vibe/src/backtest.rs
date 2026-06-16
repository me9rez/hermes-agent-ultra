//! Backtest engine: runs template strategies on OHLCV data.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::VibeError;
use crate::indicators::{rsi, sma};
use crate::types::OhlcvData;

/// Results of a backtest run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCard {
    /// Unique run identifier: `{symbol}-{strategy}-{timestamp}`
    #[serde(default)]
    pub id: String,
    /// ISO 8601 timestamp of when this run was created
    #[serde(default)]
    pub created_at: String,
    pub symbol: String,
    pub strategy: String,
    pub params: serde_json::Value,
    pub total_return_pct: f64,
    pub max_drawdown_pct: f64,
    pub trade_count: usize,
    pub sharpe_ratio: f64,
    pub win_rate_pct: f64,
    pub period: Period,
}

impl RunCard {
    /// Generate a persistence ID: `{symbol}-{strategy}-{timestamp}`.
    ///
    /// Slashes in the symbol are replaced with underscores to ensure
    /// the ID is safe to use as a directory name.
    pub fn generate_id(&self, now: &DateTime<Utc>) -> String {
        let safe_symbol = self.symbol.replace('/', "_");
        let ts = now.format("%Y%m%dT%H%M%SZ");
        format!("{safe_symbol}-{}-{ts}", self.strategy)
    }

    /// Attach persistence metadata (`id` and `created_at`) to this RunCard.
    pub fn with_persistence_meta(mut self, id: String, created_at: String) -> Self {
        self.id = id;
        self.created_at = created_at;
        self
    }
}

/// Date range of the backtest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Period {
    pub start: String,
    pub end: String,
}

/// The backtest engine.
pub struct BacktestEngine;

/// Metadata about an available strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyInfo {
    /// Strategy identifier (used in `run_backtest` `strategy` param).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Default parameter values.
    pub default_params: serde_json::Value,
}

/// Registry of all available backtest strategies.
pub struct StrategyRegistry;

impl StrategyRegistry {
    /// Return metadata for all built-in strategies.
    pub fn all() -> Vec<StrategyInfo> {
        vec![
            StrategyInfo {
                name: "sma_cross".to_string(),
                description: "SMA crossover: buy on golden cross, sell on death cross".to_string(),
                default_params: serde_json::json!({
                    "short_window": 20,
                    "long_window": 50
                }),
            },
            StrategyInfo {
                name: "rsi_revert".to_string(),
                description: "RSI mean reversion: buy when RSI crosses above oversold, sell when crosses below overbought".to_string(),
                default_params: serde_json::json!({
                    "rsi_period": 14,
                    "oversold": 30,
                    "overbought": 70
                }),
            },
        ]
    }
}

/// Internal record of a completed round-trip trade.
struct Trade {
    entry_price: f64,
    exit_price: f64,
}

/// Trading signal type, used to bridge from `hermes-strategies::Decision`
/// to `BacktestEngine` without creating a circular dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// Buy signal.
    Buy,
    /// Sell signal.
    Sell,
    /// No action.
    Hold,
}

impl BacktestEngine {
    /// Run a named strategy on the given OHLCV data.
    pub fn run(
        data: &OhlcvData,
        strategy: &str,
        params: &serde_json::Value,
    ) -> Result<RunCard, VibeError> {
        match strategy {
            "sma_cross" => Self::run_sma_cross(data, params),
            "rsi_revert" => Self::run_rsi_revert(data, params),
            _ => Err(VibeError::UnsupportedStrategy(strategy.to_string())),
        }
    }

    /// Run backtest from pre-computed signals.
    ///
    /// This is the bridge method for the declarative strategy framework:
    /// `hermes-tools` converts `hermes-strategies::Decision` → `SignalKind`,
    /// then calls this method to run the trade simulation.
    pub fn run_from_signals(
        data: &OhlcvData,
        strategy_name: &str,
        params: &serde_json::Value,
        signals: &[SignalKind],
    ) -> Result<RunCard, VibeError> {
        if signals.len() != data.len() {
            return Err(VibeError::Backtest(format!(
                "Signal length {} does not match data length {}",
                signals.len(),
                data.len()
            )));
        }

        // Fix 6: Defensive check for empty data to prevent panic on .first().unwrap().
        if data.is_empty() {
            return Err(VibeError::Backtest("Cannot run backtest on empty data".into()));
        }

        let trades = simulate_trades_from_signals(data, signals);
        let (total_return_pct, max_drawdown_pct, sharpe_ratio, trade_count, win_rate_pct) =
            compute_metrics(&trades);

        let period = Period {
            start: data.rows.first().unwrap().date.to_string(),
            end: data.rows.last().unwrap().date.to_string(),
        };

        Ok(RunCard {
            id: String::new(),
            created_at: String::new(),
            symbol: data.symbol.clone(),
            strategy: strategy_name.to_string(),
            params: params.clone(),
            total_return_pct,
            max_drawdown_pct,
            trade_count,
            sharpe_ratio,
            win_rate_pct,
            period,
        })
    }

    /// SMA crossover strategy.
    ///
    /// * Golden cross (short SMA crosses above long SMA) → buy (full position)
    /// * Death cross (short SMA crosses below long SMA) → sell (full position)
    fn run_sma_cross(
        data: &OhlcvData,
        params: &serde_json::Value,
    ) -> Result<RunCard, VibeError> {
        let short_window = params
            .get("short_window")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;
        let long_window = params
            .get("long_window")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        if data.len() < long_window {
            return Err(VibeError::Backtest(format!(
                "Insufficient data: have {} rows, need at least {long_window}",
                data.len()
            )));
        }

        let closes: Vec<f64> = data.rows.iter().map(|r| r.close).collect();
        let sma_short = sma(&closes, short_window);
        let sma_long = sma(&closes, long_window);

        // Simulate trades.
        let mut trades: Vec<Trade> = Vec::new();
        let mut entry_price: Option<f64> = None;
        // Track previous-bar SMA values to detect cross.
        let mut prev_short: Option<f64> = None;
        let mut prev_long: Option<f64> = None;

        for i in 0..closes.len() {
            let s = sma_short[i];
            let l = sma_long[i];
            if let (Some(ps), Some(pl)) = (prev_short, prev_long)
                && let (Some(cs), Some(cl)) = (s, l)
            {
                // Golden cross: short crosses above long
                if ps <= pl && cs > cl && entry_price.is_none() {
                    entry_price = Some(closes[i]);
                }
                // Death cross: short crosses below long
                if ps >= pl && cs < cl
                    && let Some(ep) = entry_price.take()
                {
                    trades.push(Trade {
                        entry_price: ep,
                        exit_price: closes[i],
                    });
                }
            }
            prev_short = s;
            prev_long = l;
        }

        // If still holding at the end, close at last price.
        if let Some(ep) = entry_price {
            trades.push(Trade {
                entry_price: ep,
                exit_price: *closes.last().unwrap(),
            });
        }

        // Compute metrics.
        let initial_capital = 10_000.0_f64;
        let mut capital = initial_capital;
        let mut equity_curve = vec![capital];
        let mut wins = 0usize;

        for t in &trades {
            let ret = (t.exit_price - t.entry_price) / t.entry_price;
            capital *= 1.0 + ret;
            equity_curve.push(capital);
            if t.exit_price > t.entry_price {
                wins += 1;
            }
        }

        let total_return_pct = (capital / initial_capital - 1.0) * 100.0;

        // Max drawdown from equity curve.
        let max_drawdown_pct = compute_max_drawdown(&equity_curve);

        // Daily returns for Sharpe (using equity curve steps).
        let sharpe_ratio = compute_sharpe(&equity_curve);

        let trade_count = trades.len();
        let win_rate_pct = if trade_count > 0 {
            wins as f64 / trade_count as f64 * 100.0
        } else {
            0.0
        };

        let period = Period {
            start: data.rows.first().unwrap().date.to_string(),
            end: data.rows.last().unwrap().date.to_string(),
        };

        Ok(RunCard {
            id: String::new(),
            created_at: String::new(),
            symbol: data.symbol.clone(),
            strategy: "sma_cross".to_string(),
            params: params.clone(),
            total_return_pct,
            max_drawdown_pct,
            trade_count,
            sharpe_ratio,
            win_rate_pct,
            period,
        })
    }

    /// RSI mean-reversion strategy.
    ///
    /// * Buy when RSI crosses above `oversold` from below.
    /// * Sell when RSI crosses below `overbought` from above.
    /// * Default: `rsi_period=14`, `oversold=30`, `overbought=70`.
    fn run_rsi_revert(
        data: &OhlcvData,
        params: &serde_json::Value,
    ) -> Result<RunCard, VibeError> {
        let rsi_period = params
            .get("rsi_period")
            .and_then(|v| v.as_u64())
            .unwrap_or(14) as usize;
        let oversold = params
            .get("oversold")
            .and_then(|v| v.as_f64())
            .unwrap_or(30.0);
        let overbought = params
            .get("overbought")
            .and_then(|v| v.as_f64())
            .unwrap_or(70.0);

        let min_rows = rsi_period + 2;
        if data.len() < min_rows {
            return Err(VibeError::Backtest(format!(
                "Insufficient data: have {} rows, need at least {min_rows}",
                data.len()
            )));
        }

        let closes: Vec<f64> = data.rows.iter().map(|r| r.close).collect();
        let rsi_values = rsi(&closes, rsi_period);

        let mut trades: Vec<Trade> = Vec::new();
        let mut entry_price: Option<f64> = None;
        let mut prev_rsi: Option<f64> = None;

        for i in 0..closes.len() {
            let cur = rsi_values[i];
            if let (Some(pr), Some(cr)) = (prev_rsi, cur) {
                // Buy: RSI crosses above oversold from below.
                if pr <= oversold && cr > oversold && entry_price.is_none() {
                    entry_price = Some(closes[i]);
                }
                // Sell: RSI crosses below overbought from above.
                if pr >= overbought && cr < overbought
                    && let Some(ep) = entry_price.take()
                {
                    trades.push(Trade {
                        entry_price: ep,
                        exit_price: closes[i],
                    });
                }
            }
            prev_rsi = cur;
        }

        // If still holding at the end, close at last price.
        if let Some(ep) = entry_price {
            trades.push(Trade {
                entry_price: ep,
                exit_price: *closes.last().unwrap(),
            });
        }

        let initial_capital = 10_000.0_f64;
        let mut capital = initial_capital;
        let mut equity_curve = vec![capital];
        let mut wins = 0usize;

        for t in &trades {
            let ret = (t.exit_price - t.entry_price) / t.entry_price;
            capital *= 1.0 + ret;
            equity_curve.push(capital);
            if t.exit_price > t.entry_price {
                wins += 1;
            }
        }

        let total_return_pct = (capital / initial_capital - 1.0) * 100.0;
        let max_drawdown_pct = compute_max_drawdown(&equity_curve);
        let sharpe_ratio = compute_sharpe(&equity_curve);
        let trade_count = trades.len();
        let win_rate_pct = if trade_count > 0 {
            wins as f64 / trade_count as f64 * 100.0
        } else {
            0.0
        };

        let period = Period {
            start: data.rows.first().unwrap().date.to_string(),
            end: data.rows.last().unwrap().date.to_string(),
        };

        Ok(RunCard {
            id: String::new(),
            created_at: String::new(),
            symbol: data.symbol.clone(),
            strategy: "rsi_revert".to_string(),
            params: params.clone(),
            total_return_pct,
            max_drawdown_pct,
            trade_count,
            sharpe_ratio,
            win_rate_pct,
            period,
        })
    }
}

/// Compute maximum drawdown percentage from an equity curve.
/// Returns a non-positive number (e.g. -15.0 means 15% drawdown).
fn compute_max_drawdown(equity: &[f64]) -> f64 {
    if equity.is_empty() {
        return 0.0;
    }
    let mut peak = equity[0];
    let mut max_dd = 0.0_f64;
    for &val in equity {
        if val > peak {
            peak = val;
        }
        let dd = (val - peak) / peak * 100.0;
        if dd < max_dd {
            max_dd = dd;
        }
    }
    max_dd
}

/// Compute annualized Sharpe ratio from an equity curve.
/// Assumes risk-free rate = 0, annualization factor = √252.
fn compute_sharpe(equity: &[f64]) -> f64 {
    if equity.len() < 2 {
        return 0.0;
    }
    let returns: Vec<f64> = equity
        .windows(2)
        .map(|w| (w[1] - w[0]) / w[0])
        .collect();
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();
    if std_dev == 0.0 {
        return 0.0;
    }
    mean / std_dev * (252.0_f64).sqrt()
}

/// Simulate trades from a signal sequence.
///
/// Buy → enter position; Sell → exit position.
/// If still holding at the end, close at last price.
fn simulate_trades_from_signals(data: &OhlcvData, signals: &[SignalKind]) -> Vec<Trade> {
    let closes: Vec<f64> = data.rows.iter().map(|r| r.close).collect();
    let mut trades = Vec::new();
    let mut entry_price: Option<f64> = None;

    for (i, signal) in signals.iter().enumerate() {
        match signal {
            SignalKind::Buy if entry_price.is_none() => {
                entry_price = Some(closes[i]);
            }
            SignalKind::Sell
                if entry_price.is_some()
                    && let Some(ep) = entry_price.take() =>
            {
                trades.push(Trade {
                    entry_price: ep,
                    exit_price: closes[i],
                });
            }
            _ => {}
        }
    }

    // If still holding at the end, close at last price.
    if let Some(ep) = entry_price {
        trades.push(Trade {
            entry_price: ep,
            exit_price: *closes.last().unwrap(),
        });
    }

    trades
}

/// Compute metrics from a list of trades.
///
/// Returns `(total_return_pct, max_drawdown_pct, sharpe_ratio, trade_count, win_rate_pct)`.
fn compute_metrics(trades: &[Trade]) -> (f64, f64, f64, usize, f64) {
    let initial_capital = 10_000.0_f64;
    let mut capital = initial_capital;
    let mut equity_curve = vec![capital];
    let mut wins = 0usize;

    for t in trades {
        let ret = (t.exit_price - t.entry_price) / t.entry_price;
        capital *= 1.0 + ret;
        equity_curve.push(capital);
        if t.exit_price > t.entry_price {
            wins += 1;
        }
    }

    let total_return_pct = (capital / initial_capital - 1.0) * 100.0;
    let max_drawdown_pct = compute_max_drawdown(&equity_curve);
    let sharpe_ratio = compute_sharpe(&equity_curve);
    let trade_count = trades.len();
    let win_rate_pct = if trade_count > 0 {
        wins as f64 / trade_count as f64 * 100.0
    } else {
        0.0
    };

    (total_return_pct, max_drawdown_pct, sharpe_ratio, trade_count, win_rate_pct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    use crate::types::{Interval, OhlcvData, OhlcvRow};

    /// Generate deterministic OHLCV data with a sine-wave trend that
    /// produces both uptrends and downtrends → triggers golden/death crosses.
    fn mock_ohlcv(days: usize) -> OhlcvData {
        let base_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let mut rows = Vec::with_capacity(days);
        for i in 0..days {
            // Smooth sine overlay on a slight upward drift.
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
        }
    }

    #[test]
    fn test_sma_cross_backtest() {
        let data = mock_ohlcv(120);
        let params = serde_json::json!({"short_window": 5, "long_window": 10});
        let card = BacktestEngine::run(&data, "sma_cross", &params).unwrap();
        assert!(
            card.trade_count > 0,
            "Expected at least 1 trade, got {}",
            card.trade_count
        );
        assert!(
            card.max_drawdown_pct <= 0.0,
            "Max drawdown should be <= 0"
        );
        assert!(
            card.win_rate_pct >= 0.0 && card.win_rate_pct <= 100.0,
            "Win rate out of range: {}",
            card.win_rate_pct
        );
        assert_eq!(card.symbol, "MOCK-USD");
        assert_eq!(card.strategy, "sma_cross");
    }

    #[test]
    fn test_unsupported_strategy() {
        let data = mock_ohlcv(100);
        let params = serde_json::json!({});
        let result = BacktestEngine::run(&data, "unknown", &params);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            VibeError::UnsupportedStrategy(_)
        ));
    }

    #[test]
    fn test_insufficient_data() {
        let data = mock_ohlcv(5);
        let params = serde_json::json!({"short_window": 5, "long_window": 10});
        let result = BacktestEngine::run(&data, "sma_cross", &params);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), VibeError::Backtest(_)));
    }

    #[test]
    fn test_default_params() {
        let data = mock_ohlcv(100);
        let params = serde_json::json!({});
        // defaults: short=20, long=50 → need >=50 rows
        let card = BacktestEngine::run(&data, "sma_cross", &params).unwrap();
        assert_eq!(card.strategy, "sma_cross");
    }

    #[test]
    fn test_run_card_serialization() {
        let data = mock_ohlcv(120);
        let params = serde_json::json!({"short_window": 5, "long_window": 10});
        let card = BacktestEngine::run(&data, "sma_cross", &params).unwrap();
        let json = serde_json::to_string(&card).unwrap();
        let deserialized: RunCard = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, card.symbol);
        assert_eq!(deserialized.trade_count, card.trade_count);
    }

    #[test]
    fn test_max_drawdown_flat() {
        let equity = vec![100.0, 100.0, 100.0];
        assert!((compute_max_drawdown(&equity) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_max_drawdown_simple() {
        let equity = vec![100.0, 110.0, 90.0, 105.0];
        let dd = compute_max_drawdown(&equity);
        // Peak=110, trough=90 → dd = (90-110)/110*100 ≈ -18.18%
        assert!((dd - (-18.18181818181818)).abs() < 0.001);
    }

    #[test]
    fn test_sharpe_flat() {
        let equity = vec![100.0; 10];
        assert!((compute_sharpe(&equity) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_runcard_generate_id() {
        let card = RunCard {
            id: String::new(),
            created_at: String::new(),
            symbol: "BTC-USDT".into(),
            strategy: "sma_cross".into(),
            params: serde_json::json!({}),
            total_return_pct: 0.0,
            max_drawdown_pct: 0.0,
            trade_count: 0,
            sharpe_ratio: 0.0,
            win_rate_pct: 0.0,
            period: Period {
                start: "2024-01-01".into(),
                end: "2024-06-01".into(),
            },
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-16T14:30:22Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let id = card.generate_id(&now);
        assert_eq!(id, "BTC-USDT-sma_cross-20260616T143022Z");
    }

    #[test]
    fn test_runcard_backward_compat_deserialize() {
        // Old-format JSON without id/created_at should deserialize with empty defaults.
        let old_json = r#"{
            "symbol": "BTC-USDT",
            "strategy": "sma_cross",
            "params": {},
            "total_return_pct": 5.0,
            "max_drawdown_pct": -2.0,
            "trade_count": 3,
            "sharpe_ratio": 1.2,
            "win_rate_pct": 66.67,
            "period": {"start": "2024-01-01", "end": "2024-06-01"}
        }"#;
        let card: RunCard = serde_json::from_str(old_json).unwrap();
        assert_eq!(card.id, "");
        assert_eq!(card.created_at, "");
        assert_eq!(card.symbol, "BTC-USDT");
        assert_eq!(card.trade_count, 3);
    }

    /// Generate mock data with strong oscillations suitable for RSI signals.
    fn mock_rsi_data(days: usize) -> OhlcvData {
        let base_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let mut rows = Vec::with_capacity(days);
        for i in 0..days {
            let t = i as f64;
            // Sharper sine wave produces bigger RSI swings.
            let price = 100.0 + 20.0 * (t * std::f64::consts::PI / 15.0).sin();
            rows.push(OhlcvRow {
                date: base_date + chrono::Duration::days(i as i64),
                open: price * 0.999,
                high: price * 1.02,
                low: price * 0.98,
                close: price,
                volume: 1_000_000.0,
            });
        }
        OhlcvData {
            symbol: "MOCK-USD".to_string(),
            interval: Interval::Daily,
            rows,
        }
    }

    #[test]
    fn test_rsi_revert_backtest() {
        let data = mock_rsi_data(120);
        let params = serde_json::json!({"rsi_period": 14, "oversold": 30, "overbought": 70});
        let card = BacktestEngine::run(&data, "rsi_revert", &params).unwrap();
        assert_eq!(card.strategy, "rsi_revert");
        assert_eq!(card.symbol, "MOCK-USD");
        assert!(card.trade_count >= 1, "Expected at least 1 trade, got {}", card.trade_count);
        assert!(card.max_drawdown_pct <= 0.0, "Max drawdown should be <= 0");
        assert!(card.win_rate_pct >= 0.0 && card.win_rate_pct <= 100.0);
    }

    #[test]
    fn test_rsi_revert_insufficient_data() {
        let data = mock_rsi_data(10);
        let params = serde_json::json!({"rsi_period": 14});
        let result = BacktestEngine::run(&data, "rsi_revert", &params);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), VibeError::Backtest(_)));
    }

    #[test]
    fn test_strategy_registry() {
        let strategies = StrategyRegistry::all();
        assert!(strategies.len() >= 2, "Expected at least 2 strategies");
        let names: Vec<&str> = strategies.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"sma_cross"));
        assert!(names.contains(&"rsi_revert"));
    }
}
