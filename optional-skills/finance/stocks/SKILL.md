---
name: stocks
description: Stock search, compare, history via Yahoo (optional). Spot quotes → use bundled spot-quote + get_quote.
version: 0.2.1
author: Mibay (Mibayy), Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [Stocks, Finance, Market, Crypto, Investing]
    category: finance
    related_skills: [trading-research, trading-debate, spot-quote, dcf-model, comps-analysis, lbo-model]
    requires_toolsets: [terminal]
---

# Stocks Skill (optional)

Read-only market data via Yahoo Finance. Five commands: `quote`, `search`,
`history`, `compare`, `crypto`. Python stdlib only — no API key, no pip
installs. Yahoo's endpoint is unofficial and may rate-limit or change.

**Spot quotes:** Use the built-in **`get_quote`** tool for live prices (no Python).
Install this optional skill only for **search**, **compare**, or **history** browse.

**Optional:** Install with `hermes skills install stocks`. Requires `terminal` toolset
and Python 3 on the host.

## When to Use

- User wants to **look up a ticker by company name** (`search`)
- User wants to **compare several tickers** side by side (`compare`)
- User asks for a **crypto spot price** via Yahoo (`crypto` — pass `BTC`, script appends `-USD`)
- User wants a **quick price history browse** (`history` — light Yahoo chart only; see routing table)

## When NOT to Use

- User asks for a **current stock price** (AAPL, TSLA, MSFT, ...) → use built-in **`get_quote`**
- User asks **多少钱 / 股价 / 现价** for a ticker → use **`get_quote`**

- User wants **historical OHLCV for backtesting**, Sharpe, drawdown, or T+1 rules → use **`trading-research`** (`get_market_data` / `run_backtest`)
- User wants **A-share research with Eastmoney live data** → **`trading-research`**
- User wants **HK/US/A-share/crypto unified backtest pipeline** → **`trading-research`**
- User wants **investment-committee bull/bear debate** after a backtest → **`trading-debate`**
- User wants **news, fundamentals, or research reports** → `web_search`

## Skill routing (stocks vs trading-research)

| User intent | Skill | Tool / command |
|-------------|-------|----------------|
| 「苹果现在多少钱」 | **`get_quote`** (built-in) | `get_quote(symbol="AAPL")` |
| 「特斯拉代码是什么」 | **stocks** | `search "Tesla"` |
| 「比一下 AAPL MSFT GOOGL」 | **stocks** | `compare` |
| 「拉 000001.SZ 180 天日 K 并回测 RSI」 | **trading-research** | `run_backtest` |
| 「0700.HK / AAPL 历史回测」 | **not supported** | use **`get_quote`** for spot; backtest A-share/crypto only |
| 「BTC-USDT 最近 30 天 K 线」 | **trading-research** | `get_market_data` |

If `terminal` or Yahoo fails, fall back to `web_search` — **not** `execute_code` with ad-hoc yfinance.

## Prerequisites

Python 3.8+ stdlib only. Optional: set `ALPHA_VANTAGE_KEY` to enrich
`market_cap`, `pe_ratio`, and 52-week levels when Yahoo's crumb-protected
fields come back null. Free key: https://www.alphavantage.co/support/#api-key

Script path (after sync): `{HERMES_HOME}/skills/finance/stocks/scripts/stocks_client.py`

## How to Run

Invoke through the **`terminal`** tool only. Do **not** use `execute_code` to
reimplement Yahoo/yfinance.

**Unix / macOS:**

```bash
python3 "$HERMES_HOME/skills/finance/stocks/scripts/stocks_client.py" quote AAPL
```

**Windows (cmd):**

```bat
python "%HERMES_HOME%\skills\finance\stocks\scripts\stocks_client.py" quote AAPL
```

Use `python` first on Windows; try `python3` on Unix. `HERMES_HOME` is set by
Hermes at runtime (e.g. `%LOCALAPPDATA%\hermes-agent-ultra` on Windows).

All output is JSON on stdout.

## Quick Reference

```
quote AAPL
quote AAPL MSFT GOOGL TSLA
search "Tesla"
history NVDA --range 6mo
compare AAPL MSFT GOOGL
crypto BTC ETH SOL
```

(Prepend the script path from **How to Run** to each command.)

## Commands

### `quote SYMBOL [SYMBOL2 ...]`

Current price, change, change%, volume, 52-week high/low.

### `search QUERY`

Find tickers by company name. Returns top 5: symbol, name, exchange, type.

### `history SYMBOL [--range RANGE]`

Daily OHLCV plus stats (min, max, avg, total return %). Ranges: `1mo`,
`3mo`, `6mo`, `1y`, `5y`. Default: `1mo`.

**Note:** For backtests or multi-market research, use **`trading-research`** instead.

### `compare SYMBOL1 SYMBOL2 [...]`

Side-by-side: price, change%, 52-week performance.

### `crypto SYMBOL [SYMBOL2 ...]`

Crypto prices. Pass `BTC` (the script appends `-USD` automatically).

For crypto **backtests**, use **`trading-research`** with `BTC-USDT`.

## Pitfalls

- Yahoo Finance's API is unofficial. Endpoints can change or rate-limit
  without notice — if requests start failing, that's why.
- `market_cap` and `pe_ratio` may return null on `quote` when Yahoo's
  crumb session isn't established. Set `ALPHA_VANTAGE_KEY` to backfill.
- Add a small delay between bulk requests to avoid rate-limiting.
- This is read-only — no order placement, no account integration.

## Verification

```bash
python3 "$HERMES_HOME/skills/finance/stocks/scripts/stocks_client.py" quote AAPL
```

Returns a JSON object with `symbol: "AAPL"` and a numeric `price` field.

Ask: "AAPL 现在多少钱"
Expected: Agent calls `get_quote(symbol="AAPL")` — **not** `run_backtest`, `execute_code`, or this optional skill.
