---
name: global-market-watch
description: "Global market observer: real-time watchlist, price alerts, and multi-market quote aggregation."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [market, watchlist, alert, quote, real-time, finance, global]
    category: finance
---

# Global Market Watch

Global market observer with real-time watchlist management, price alerts, and multi-market quote aggregation.

## Capabilities

- **Watchlist Management**: Add/remove symbols across markets (US equities, crypto, A-shares, forex)
- **Real-Time Quotes**: Fetch live price snapshots via `hermes-market-watch` `QuoteProvider`
- **Price Alerts**: Configure conditions (price above/below, change %, volume spike) and receive triggers
- **Multi-Market Coverage**: Supports any symbol understood by the configured data provider

## Usage

```
Add BTC-USDT and AAPL to my watchlist, alert me when BTC drops below $60,000
```

```
Show me the latest quotes for all symbols in my watchlist
```

```
Set a 5% daily change alert for TSLA
```

## Architecture

This skill delegates to the `hermes-market-watch` crate:
- `Watchlist` — symbol management
- `QuoteProvider` — quote fetching (trait, provider-agnostic)
- `AlertEngine` — condition evaluation and trigger generation

## 0py Constraint

Fully Rust-native. No Python runtime, PyO3, or subprocess dependencies.
