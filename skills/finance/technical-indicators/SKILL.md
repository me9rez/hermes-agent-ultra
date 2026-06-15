---
name: technical-indicators
description: "Technical indicator library: SMA, EMA, RSI, MACD, Bollinger Bands, and extensible indicator trait."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [indicator, sma, ema, rsi, macd, bollinger, technical, finance]
    category: finance
---

# Technical Indicators

A Rust-native technical indicator library providing standard TA indicators and an extensible `Indicator` trait.

## Built-in Indicators

| Indicator | Description | Parameters |
|-----------|-------------|------------|
| **SMA** | Simple Moving Average | `period` |
| **EMA** | Exponential Moving Average | `period` |
| **RSI** | Relative Strength Index | `period` |
| **MACD** | Moving Average Convergence Divergence | `fast`, `slow` |
| **Bollinger** | Bollinger Bands (middle + bands) | `period`, `num_std` |

## Usage

```
Calculate the 14-day RSI for 600519.SH over the last 60 trading days
```

```
Show me the Bollinger Bands (20, 2) for BTC-USDT daily data
```

```
Compute MACD(12, 26) for AAPL and identify crossover points
```

## Extending

Implement the `Indicator` trait from `hermes-strategies`:

```rust
use hermes_strategies::Indicator;

struct MyIndicator { /* params */ }

impl Indicator for MyIndicator {
    fn compute(&self, closes: &[f64], index: usize) -> Option<f64> { /* ... */ }
    fn name(&self) -> &str { "MyIndicator" }
    fn min_periods(&self) -> usize { /* ... */ }
}
```

## Architecture

All indicators live in `hermes-strategies::indicators` and implement the `Indicator` trait:
- `compute(closes, index) -> Option<f64>` — value at a given bar
- `name() -> &str` — diagnostic label
- `min_periods() -> usize` — warmup requirement

## 0py Constraint

Pure Rust computation. No Python, NumPy, or TA-Lib C bindings required.
