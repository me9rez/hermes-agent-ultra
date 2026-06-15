---
name: market-news-sentinel
description: "Market news sentinel: monitor financial news sources, detect market-moving events, and generate alerts."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [news, sentinel, alert, market, monitoring, finance, events]
    category: finance
---

# Market News Sentinel

Monitor financial news sources, detect market-moving events, and generate actionable alerts.

## Capabilities

- **News Monitoring**: Poll configured news sources for new articles (RSS, HTTP APIs)
- **Event Detection**: Classify news by relevance to watched symbols / sectors
- **Alert Generation**: Emit alerts when significant market events are detected
- **Integration with Watchlist**: Cross-reference news with `hermes-market-watch` watchlist symbols

## Usage

```
Monitor news for AAPL and TSLA, alert me on earnings announcements or major analyst upgrades
```

```
Summarize today's market-moving news for my watchlist symbols
```

```
Check if there are any regulatory filings or insider transactions for 600519.SH in the past week
```

## Architecture

This skill is a thin orchestration layer that:
1. Fetches news from configured HTTP endpoints (via `reqwest`)
2. Cross-references against `hermes-market-watch` watchlist
3. Generates structured alerts compatible with `hermes-market-watch` `AlertEngine`

## Data Sources

News sources are configured via environment or config file. Built-in support planned for:
- RSS feeds (financial news outlets)
- SEC EDGAR filings (US equities)
- Exchange announcement APIs

## 0py Constraint

All fetching and classification is Rust-native. No Python NLP libraries or web scraping frameworks.
