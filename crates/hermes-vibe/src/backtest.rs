//! Backtest engine: runs template strategies on OHLCV data.

use serde::{Deserialize, Serialize};

use crate::error::VibeError;
use crate::indicators::sma;
use crate::types::OhlcvData;

/// Results of a backtest run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCard {
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

/// Date range of the backtest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Period {
    pub start: String,
    pub end: String,
}

/// The backtest engine.
pub struct BacktestEngine;

/// Internal record of a completed round-trip trade.
struct Trade {
    entry_price: f64,
    exit_price: f64,
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
            _ => Err(VibeError::UnsupportedStrategy(strategy.to_string())),
        }
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
}
