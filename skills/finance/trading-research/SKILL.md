---
name: trading-research
description: Quantitative research with real market data and backtesting. No API key required.
version: 0.5.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [Finance, Quantitative, Backtest, Market-Data, A-Share, Crypto, HK, US]
    category: finance
    related_skills: [stocks, trading-debate, spot-quote]
---

# Trading Research Skill

Pure Rust quantitative research — fetch real OHLCV market data and run template
backtests without any API key or Python dependency.

## When to Use

- User asks for historical K-line / candlestick / OHLCV data
- User wants to backtest SMA crossover or RSI mean-reversion strategies
- User wants to create a custom declarative strategy (`create_strategy`)
- User asks about A-share (沪深股票) or crypto (BTC/ETH) price history
- User wants quantitative performance metrics (return, drawdown, Sharpe)
- User wants to retrieve a previous backtest report (`get_backtest_report`)

## When NOT to Use

- User asks for **news or research reports** → use `web_search`
- User asks for **real-time quote only** (no backtest/history pipeline) → use bundled **`spot-quote`** skill + **`get_quote`**; `web_search` only on failure (e.g. Yahoo blocked without VPN) or for retail goods (shoes, rent, etc.)
- User asks for **investment-committee bull/bear debate** → use **`trading-debate`** (after `run_backtest`)
- User asks to **place orders or trade** → not supported
- User asks about **fundamentals** (PE, revenue) → use `web_search`
- User asks for **US/HK historical K-line or backtest** → use **`get_quote`** for spot only; historical OHLCV not supported yet
- User asks about markets not supported (futures, options, forex) → inform limitation

## Available Tools

### `get_market_data`

Fetch OHLCV data for a symbol over a date range.

**Parameters:**
| Param | Required | Default | Description |
|-------|----------|---------|-------------|
| `symbol` | ✅ | — | `BTC-USDT`, `000001.SZ` |
| `start_date` | — | 30 days ago | `YYYY-MM-DD` |
| `end_date` | — | today | `YYYY-MM-DD` |
| `interval` | — | `daily` | `daily` or `weekly` |
| `source` | — | `auto` | `auto`, `binance`, or `eastmoney` |
| `refresh` | — | `false` | Bypass disk cache and force network fetch |

**Disk cache:** Responses are cached at `{HERMES_HOME}/trading/cache/` for 24h (key: `{source}-{symbol}-{interval}-{dates}.json`). Delete files manually to clear cache.

**Response field `partial`:** `true` when returned rows do not fully cover the requested date range (holidays, suspensions).

**Supported Markets (auto-routing):**
- A-shares: `XXXXXX.SZ` / `XXXXXX.SH` → EastMoney (live)
- Crypto: `XXX-YYY` pairs → Binance (live)
- US/HK: **not supported** for historical OHLCV — use **`get_quote`** (`spot-quote` skill) for spot prices

**Symbol routing rules:**
| Format | Market | Provider |
|--------|--------|----------|
| `000001.SZ`, `600519.SH` | A-share | eastmoney |
| `BTC-USDT`, `ETH-USDT` | Crypto | binance |
| `0700.HK`, `AAPL` | US/HK | ❌ not supported (use `get_quote`) |

### `run_backtest`

Run a strategy backtest on historical data. Results are saved to `~/.hermes/trading/runs/{id}/run_card.json`.

**Parameters:**
| Param | Required | Default | Description |
|-------|----------|---------|-------------|
| `symbol` | ✅ | — | Same as `get_market_data` |
| `strategy` | ✅ | — | e.g. `sma_cross`, `rsi_revert`, or user-created name |
| `params` | — | `{}` | Strategy-specific params |
| `source` | — | `auto` | Data source override |
| `refresh` | — | `false` | Bypass disk cache |
| `risk_free_rate` | — | `0.0` | Annual risk-free rate for Sharpe |
| `start_date` | — | 180 days ago | Backtest start |
| `end_date` | — | today | Backtest end |

**Built-in strategies:**
- `sma_cross` — `short_window` (20), `long_window` (50); golden/death cross
- `rsi_revert` — `rsi_period` (14), `oversold` (30), `overbought` (70)

Use `list_strategies` to see all built-in and user-created strategies.

**A-share T+1 rules (auto-enabled for `.SZ`/`.SH`):**
- Buy signals fill at the **next trading day's open**
- Sell signals fill at **same-day close** (cannot sell shares bought same day)

**Output:** RunCard JSON with `id`, `total_return_pct`, `max_drawdown_pct`, `trade_count`,
`sharpe_ratio`, `win_rate_pct`, `period`.

### `get_backtest_report`

Load a previously saved RunCard by `id` from `~/.hermes/trading/runs/{id}/run_card.json`.

### `list_strategies` / `create_strategy`

- `list_strategies` — enumerate built-in + user strategies
- `create_strategy` — define a new declarative strategy from indicators and rules

## Tool Calling Order

1. Data only → `get_market_data`
2. Backtest → `run_backtest` (fetches data internally; saves run card)
3. Review past run → `get_backtest_report` with `id` from prior `run_backtest`
4. Custom strategy → `create_strategy`, then `run_backtest` with new name
5. Bull/bear debate after backtest → switch to **`trading-debate`** skill
6. Never fabricate numbers — always use tool output

## Critical Rules

- **NEVER fabricate backtest numbers.** Always call `run_backtest` and report its output.
- **NEVER invent OHLCV data.** Always call `get_market_data`.
- If a tool returns an error, report the error honestly to the user.
- Do not claim support for markets/strategies that are not implemented.

## Relationship with `spot-quote`, `get_quote`, and optional `stocks` Skill

| Scenario | Use this skill | Use `spot-quote` / `get_quote` / `stocks` |
|----------|---------------|---------------------------------------------|
| Historical OHLCV (A-share/crypto) | ✅ | — |
| Backtest / Sharpe / T+1 (A-share/crypto) | ✅ | — |
| Quick US/HK/A-share/crypto spot quote | — | ✅ **`spot-quote`** → **`get_quote`** (`source=auto`) |
| Retail goods price (shoes, phones) | — | **`web_search`** (not `get_quote`) |
| Company search by name | — | optional **`stocks`** (`skills install stocks`) |
| `get_quote` failed | — | `web_search` for spot price |

## Relationship with `trading-debate` Skill

After `run_backtest` produces a RunCard, hand off to **`trading-debate`** when the user
wants bull/bear analysis or an investment-committee style verdict. Do not run debate
logic inside this skill — delegate per `trading-debate` workflow.

## Verification

Ask: "拉 BTC-USDT 最近 30 天日 K 线"
Expected: Agent calls `get_market_data` with symbol="BTC-USDT", returns OHLCV JSON.

Ask: "回测 000001.SZ RSI 策略"
Expected: Agent calls `run_backtest` with symbol="000001.SZ", strategy="rsi_revert",
returns RunCard JSON with T+1-adjusted metrics.

Ask: "回测 000001.SZ 20/50 均线策略"
Expected: Agent calls `run_backtest` with symbol="000001.SZ", strategy="sma_cross",
params={"short_window":20,"long_window":50}, returns RunCard JSON.

Ask: "回测 AAPL SMA 策略"
Expected: **Do not** call `run_backtest`. Explain US historical OHLCV is not supported; offer `get_quote` for spot or A-share/crypto backtest.

Ask: "AAPL 现在多少钱"
Expected: Follow **`spot-quote`** — `get_quote(symbol="AAPL", source="auto")`, report `price`. Use `web_search` only if Yahoo fails — **not** `get_market_data` / `run_backtest` / `execute_code`.

Ask: "000001.SZ 现在多少钱"
Expected: Agent calls `get_quote(symbol="000001.SZ")`, reports `price` from JSON.

## Limitations

- US/HK historical OHLCV and backtest are **not supported** (use `get_quote` for spot prices)
- Disk cache TTL is 24h; use `refresh=true` to force fresh data
- No order placement capability
- Crypto data from Binance only
- A-share data from EastMoney only (may have rate limits)
