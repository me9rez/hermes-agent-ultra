---
name: spot-quote
description: "Spot price for stocks and crypto only. Use get_quote for A-share, US/HK equities, and crypto pairs. Do NOT use for retail goods, shoes, rent, or tickets — use web_search instead."
version: 0.1.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [Finance, Quote, Spot-Price, A-Share, Crypto, HK, US]
    category: finance
    related_skills: [trading-research, stocks]
---

# Spot Quote Skill

Live **financial** spot prices via the built-in `get_quote` tool. No API key, no Python.

## Agent Workflow

No gateway keyword routing — **you** decide whether this skill applies:

1. Read `<available_skills>` or call `skills_list` when the user asks about a price or quote.
2. If the request is for **stocks / crypto spot prices**, call `skill_view(name="spot-quote")` and follow this skill.
3. If the request is for **retail goods, shoes, rent, tickets**, do **not** use this skill — use `web_search`.
4. When unsure, prefer `skill_view` before calling `get_quote` or `web_search`.

## When to Use

- User asks for a **stock / ETF / index / crypto spot price** (not history, not backtest)
- Examples:
  - `AAPL 现在多少钱` → `get_quote(symbol="AAPL")`
  - `0700.HK 股价` → `get_quote(symbol="0700.HK")`
  - `000001.SZ 现价` → `get_quote(symbol="000001.SZ")`
  - `BTC 多少钱` → `get_quote(symbol="BTC-USDT")`
  - `What is the current price of TSLA?` → `get_quote(symbol="TSLA")`

## When NOT to Use

- **Retail goods or services** — shoes, phones, clothes, food delivery, rent, tickets
  - `耐克鞋子什么价位` → **web_search** (NOT `get_quote`, NOT NKE stock)
  - `iPhone 16 多少钱` → **web_search**
- **Historical OHLCV / backtest / Sharpe** → use **trading-research** (`get_market_data` / `run_backtest`)
- **Company search by name** → optional **stocks** skill (`skills install stocks`) or **web_search**

## Tool Rules

1. Call **`get_quote`** with the correct symbol; always use **`source="auto"`** (never force `yahoo` / `binance`).
2. Report the numeric **`price`** from the tool JSON; include **`market_date`** and **`as_of`** when present.
3. **Crypto** must use pair format: `BTC-USDT`, `ETH-USDT` — not `BTC-USD` or `BTCUSD=X`.
4. If `get_quote` fails (e.g. Yahoo geo-blocked), fall back to **`web_search`**.
5. Do **not** use `execute_code`, `get_market_data`, or `run_backtest` for a simple spot price.

## Symbol Cheat Sheet

| User intent | symbol |
|-------------|--------|
| US stock (Apple) | `AAPL` |
| HK stock (Tencent) | `0700.HK` |
| A-share (Ping An Bank) | `000001.SZ` |
| Bitcoin | `BTC-USDT` |
| Ethereum | `ETH-USDT` |

Provider routing is automatic (`source=auto`): A-share → Eastmoney, crypto pairs → Binance, US/HK → Yahoo.

## Verification

Ask: `AAPL 现在多少钱`  
Expected: `get_quote(symbol="AAPL", source="auto")`, report `price` from JSON.

Ask: `naiike 鞋子什么价位`  
Expected: **Do not** call `get_quote`; use `web_search` for retail shoe pricing.

Ask: `BTC 多少钱`  
Expected: `get_quote(symbol="BTC-USDT", source="auto")`, report USDT price.

Ask: `回测 AAPL RSI 策略`  
Expected: **Do not** use this skill; use **trading-research**.
