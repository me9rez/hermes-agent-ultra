//! Technical indicator trait and built-in implementations.

use serde::{Deserialize, Serialize};

/// Trait for computing technical indicators over price series.
pub trait Indicator: Send + Sync {
    /// Compute the indicator value at the given index in the close-price series.
    ///
    /// `closes` is the full close-price slice; `index` is the position to
    /// evaluate (e.g. the current bar). Implementations should return `None`
    /// when insufficient lookback data is available.
    fn compute(&self, closes: &[f64], index: usize) -> Option<f64>;

    /// Human-readable indicator name (for logging / diagnostics).
    fn name(&self) -> &str;

    /// Minimum number of data points required before this indicator produces
    /// a value.
    fn min_periods(&self) -> usize;
}

// ── Built-in indicators ──────────────────────────────────────────────────

/// Simple Moving Average.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sma {
    pub period: usize,
}

impl Sma {
    pub fn new(period: usize) -> Self {
        Self { period }
    }
}

impl Indicator for Sma {
    fn compute(&self, closes: &[f64], index: usize) -> Option<f64> {
        if index + 1 < self.period || closes.len() <= index {
            return None;
        }
        let start = index + 1 - self.period;
        let sum: f64 = closes[start..=index].iter().sum();
        Some(sum / self.period as f64)
    }

    fn name(&self) -> &str {
        "SMA"
    }

    fn min_periods(&self) -> usize {
        self.period
    }
}

/// Exponential Moving Average.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ema {
    pub period: usize,
}

impl Ema {
    pub fn new(period: usize) -> Self {
        Self { period }
    }
}

impl Indicator for Ema {
    fn compute(&self, closes: &[f64], index: usize) -> Option<f64> {
        if index + 1 < self.period || closes.len() <= index {
            return None;
        }
        let k = 2.0 / (self.period as f64 + 1.0);
        // Bootstrap with SMA of first `period` values.
        let start = index + 1 - self.period;
        let mut ema: f64 = closes[start..=index].iter().sum::<f64>() / self.period as f64;
        // Then apply EMA recurrence from start+1..=index.
        for i in (start + 1)..=index {
            ema = closes[i] * k + ema * (1.0 - k);
        }
        Some(ema)
    }

    fn name(&self) -> &str {
        "EMA"
    }

    fn min_periods(&self) -> usize {
        self.period
    }
}

/// Relative Strength Index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rsi {
    pub period: usize,
}

impl Rsi {
    pub fn new(period: usize) -> Self {
        Self { period }
    }
}

impl Indicator for Rsi {
    fn compute(&self, closes: &[f64], index: usize) -> Option<f64> {
        if index + 1 <= self.period || closes.len() <= index {
            return None;
        }
        let start = index + 1 - self.period;
        let mut gains = 0.0_f64;
        let mut losses = 0.0_f64;
        for i in start..=index {
            let delta = closes[i] - closes[i - 1];
            if delta >= 0.0 {
                gains += delta;
            } else {
                losses -= delta;
            }
        }
        if losses == 0.0 {
            return Some(100.0);
        }
        let rs = (gains / self.period as f64) / (losses / self.period as f64);
        Some(100.0 - 100.0 / (1.0 + rs))
    }

    fn name(&self) -> &str {
        "RSI"
    }

    fn min_periods(&self) -> usize {
        self.period + 1
    }
}

/// Moving Average Convergence Divergence (MACD).
///
/// Returns the MACD line (fast EMA − slow EMA).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macd {
    pub fast: usize,
    pub slow: usize,
}

impl Macd {
    pub fn new(fast: usize, slow: usize) -> Self {
        Self { fast, slow }
    }
}

impl Indicator for Macd {
    fn compute(&self, closes: &[f64], index: usize) -> Option<f64> {
        let fast_ema = Ema::new(self.fast).compute(closes, index)?;
        let slow_ema = Ema::new(self.slow).compute(closes, index)?;
        Some(fast_ema - slow_ema)
    }

    fn name(&self) -> &str {
        "MACD"
    }

    fn min_periods(&self) -> usize {
        self.slow
    }
}

/// Bollinger Bands — returns the **middle band** (SMA).
///
/// Use `BollingerWidth` for the band spread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bollinger {
    pub period: usize,
    pub num_std: f64,
}

impl Bollinger {
    pub fn new(period: usize, num_std: f64) -> Self {
        Self { period, num_std }
    }

    /// Compute (lower, middle, upper) at the given index.
    pub fn bands(&self, closes: &[f64], index: usize) -> Option<(f64, f64, f64)> {
        let middle = Sma::new(self.period).compute(closes, index)?;
        if index + 1 < self.period {
            return None;
        }
        let start = index + 1 - self.period;
        let variance: f64 = closes[start..=index]
            .iter()
            .map(|&c| (c - middle).powi(2))
            .sum::<f64>()
            / self.period as f64;
        let std = variance.sqrt();
        Some((middle - self.num_std * std, middle, middle + self.num_std * std))
    }
}

impl Indicator for Bollinger {
    fn compute(&self, closes: &[f64], index: usize) -> Option<f64> {
        // Returns the middle band by default.
        Sma::new(self.period).compute(closes, index)
    }

    fn name(&self) -> &str {
        "Bollinger"
    }

    fn min_periods(&self) -> usize {
        self.period
    }
}
