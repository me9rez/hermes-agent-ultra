---
name: multi-factor-backtest
description: "Multi-factor strategy backtester: define factors, run historical backtests, evaluate performance metrics."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [backtest, factor, strategy, quant, performance, finance]
    category: finance
---

# Multi-Factor Backtest

Multi-factor strategy backtester for evaluating trading strategies against historical data.

## Capabilities

- **Factor Definition**: Combine built-in indicators (SMA, EMA, RSI, MACD, Bollinger) into custom strategy signals
- **Historical Backtesting**: Run strategies over OHLCV data fetched via `hermes-vibe`
- **Performance Metrics**: Compute returns, drawdown, Sharpe-like ratios from strategy decisions
- **Multi-Strategy Comparison**: Run multiple strategies side-by-side on the same data

## Usage

```
Backtest a dual moving average crossover strategy (SMA-10 / SMA-30) on AAPL for the past year
```

```
Compare RSI mean-reversion vs MACD trend-following on BTC-USDT daily data
```

```
Run my custom Bollinger squeeze strategy on the CSI 300 constituents and rank by Sharpe
```

## Architecture

- `hermes-vibe` — OHLCV data fetching (`MarketDataProvider` trait)
- `hermes-strategies` — `Strategy` trait, `Indicator` implementations, `Decision` output
- `hermes-copilot-lite` — `CopilotLite::analyze()` orchestration and `AnalysisReport` generation

## 0py Constraint

All computation is Rust-native. No Python, PyO3, or subprocess calls.
