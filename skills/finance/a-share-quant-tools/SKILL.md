---
name: a-share-quant-tools
description: "A股量化工具集：数据获取、因子计算、选股筛选，基于 hermes-vibe 0py 架构。"
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [quant, a-share, china, stock, factor, screening, finance]
    category: finance
---

# A-Share Quant Tools

A股量化工具集，提供从数据获取到选股筛选的完整工作流。

## 能力

- **数据获取**：通过 `hermes-vibe` 的 `MarketDataProvider` 获取 A 股 OHLCV 数据（000001.SZ / 600519.SH 等）
- **因子计算**：使用 `hermes-strategies` 内置指标（SMA、EMA、RSI、MACD、Bollinger）计算量化因子
- **选股筛选**：基于多因子条件过滤股票池，输出候选列表
- **实时行情**：通过 `hermes-market-watch` 监控 A 股实时报价和涨跌停预警

## 使用方式

```
使用 a-share-quant-tools 帮我筛选最近 30 天 RSI 低于 30 且 MACD 金叉的沪深 300 成分股
```

```
获取贵州茅台（600519.SH）最近半年的日线数据，计算布林带并画出通道
```

## 依赖

- `hermes-vibe` — OHLCV 数据获取
- `hermes-strategies` — 技术指标计算
- `hermes-market-watch` — 实时行情监控

## 0py 约束

本 skill 完全基于 Rust 原生 crate，不依赖任何 Python 运行时。
