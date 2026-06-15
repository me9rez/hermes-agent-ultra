---
name: vibe-research
description: Quantitative research with real market data and backtesting. No API key required.
version: 0.1.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [Finance, Quantitative, Backtest, Market-Data, A-Share, Crypto]
    category: finance
    related_skills: [stocks]
---

# Vibe Research Skill

Pure Rust quantitative research — fetch real OHLCV market data and run template
backtests without any API key or Python dependency.

## When to Use

- User asks for historical K-line / candlestick / OHLCV data
- User wants to backtest a moving average strategy (SMA cross)
- User asks about A-share (沪深股票) or crypto (BTC/ETH) price history
- User wants quantitative performance metrics (return, drawdown, Sharpe)

## When NOT to Use

- User asks for **news or research reports** → use `web_search`
- User asks for **real-time quote only** (no history needed) → `get_market_data` with 1-day range is fine
- User asks to **place orders or trade** → not supported in P0
- User asks about **fundamentals** (PE, revenue) → use `web_search`
- User asks about markets not supported (futures, options, forex) → inform limitation

## Available Tools

### `get_market_data`

Fetch OHLCV data for a symbol over a date range.

**Parameters:**
| Param | Required | Default | Description |
|-------|----------|---------|-------------|
| `symbol` | ✅ | — | `BTC-USDT`, `000001.SZ`, `600519.SH` |
| `start_date` | — | 30 days ago | `YYYY-MM-DD` |
| `end_date` | — | today | `YYYY-MM-DD` |
| `interval` | — | `daily` | `daily` or `weekly` |

**Supported Markets:**
- A-shares: Shenzhen (`.SZ`) and Shanghai (`.SH`) via EastMoney
- Crypto: Any pair on Binance (e.g. `BTC-USDT`, `ETH-USDT`)

### `run_backtest`

Run a template strategy backtest on historical data.

**Parameters:**
| Param | Required | Default | Description |
|-------|----------|---------|-------------|
| `symbol` | ✅ | — | Same as `get_market_data` |
| `strategy` | ✅ | — | Strategy name (currently: `sma_cross`) |
| `params` | — | `{}` | Strategy-specific params |
| `start_date` | — | 180 days ago | Backtest start |
| `end_date` | — | today | Backtest end |

**Strategy: `sma_cross`**
- `short_window` (default: 20) — Short SMA period
- `long_window` (default: 50) — Long SMA period
- Logic: Buy on golden cross, sell on death cross

**Output:** JSON with `total_return_pct`, `max_drawdown_pct`, `trade_count`,
`sharpe_ratio`, `win_rate_pct`, `period`.

## Tool Calling Order

1. If user asks for data: call `get_market_data` directly
2. If user asks for backtest: call `run_backtest` (it fetches data internally)
3. Never fabricate numbers — always use tool output

## Critical Rules

- **NEVER fabricate backtest numbers.** Always call `run_backtest` and report its output.
- **NEVER invent OHLCV data.** Always call `get_market_data`.
- If a tool returns an error, report the error honestly to the user.
- Do not claim support for markets/strategies that are not implemented.

## Relationship with `stocks` Skill

| Scenario | Use this skill | Use `stocks` |
|----------|---------------|--------------|
| Historical OHLCV (A-share/crypto) | ✅ | — |
| Backtest | ✅ | — |
| Quick US stock quote (no history) | — | ✅ |
| Company search by name | — | ✅ |

## Verification

Ask: "拉 BTC-USDT 最近 30 天日 K 线"
Expected: Agent calls `get_market_data` with symbol="BTC-USDT", returns OHLCV JSON.

Ask: "回测 000001.SZ 20/50 均线策略"
Expected: Agent calls `run_backtest` with symbol="000001.SZ", strategy="sma_cross",
params={"short_window":20,"long_window":50}, returns RunCard JSON.

## Limitations

- Only `sma_cross` strategy available (P0)
- No T+1 rule enforcement for A-shares (planned for P1)
- No disk caching (planned for P1)
- No order placement capability
- Crypto data from Binance only
- A-share data from EastMoney only (may have rate limits)
