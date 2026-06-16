# Vibe-Trading Rust 重写 — TODO 进度

> **更新时间**：2026-06-15  
> **总体状态**：P0 ✅ 已完成 → 下一步 P1

---

## ✅ 已完成（P0 — MVP 可 demo）

### 工程基础
- [x] `crates/hermes-vibe/` crate 创建 + workspace 配置
- [x] 根 `Cargo.toml` 添加 workspace member + dependency
- [x] `hermes-tools/Cargo.toml` 添加 `vibe-research` feature（已包含在 `full` 中）

### 数据层
- [x] `MarketDataProvider` trait 定义
- [x] `BinanceProvider` — Binance REST `/api/v3/klines`（Crypto，无需 Key）
- [x] `EastmoneyProvider` — 东方财富 HTTP API（A 股，无需 Key）
- [x] `AutoRouter` — 根据 symbol 格式自动路由数据源

### 回测引擎
- [x] `BacktestEngine` — 模板策略回测框架
- [x] `sma_cross` 策略 — 金叉/死叉，计算 return/drawdown/sharpe/win_rate
- [x] SMA 指标自实现（无 polars/ta-lib 依赖，零外部大库）

### Tool Handler
- [x] `get_market_data` ToolHandler — 返回 OHLCV JSON
- [x] `run_backtest` ToolHandler — 返回 RunCard JSON
- [x] `register/vibe.rs` 注册 + feature gate

### Skill & 测试
- [x] `skills/finance/vibe-research/SKILL.md`
- [x] Parity fixture + runner（`vibe_market_data/ohlcv.json` + `vibe_backtest/sma_cross.json`，`cargo test -p hermes-parity-tests` 通过，MockProvider 隔离网络）
- [x] `hermes-vibe` 单元测试 27 通过，`hermes-parity-tests` 4 个 fixture case 通过；Clippy 零警告

### P0 验收
- [x] `cargo build -p hermes-cli` 自动包含 vibe tools
- [x] `hermes chat` 中可用自然语言触发拉数据 + 回测

---

## 🔲 未完成（P1 — 增强：小完整研究闭环）

**验收目标**：A/HK/US/crypto 各至少 1 symbol 回测；`run_card.json` 可复盘；可选投委会。

### Tools 增强
- [ ] `get_market_data` 增强：多市场 symbol + 可选 `source` 参数 + `VIBE_DATA_CACHE` 磁盘缓存
- [ ] `run_backtest` 增强：+ `rsi_revert` 策略模板 + A 股 **T+1** 规则 + Sharpe 改进
- [ ] `get_backtest_report`（可选）：读 `~/.hermes/vibe/runs/{id}/run_card.json`

### Skills
- [ ] 更新 `vibe-research` SKILL：T+1 说明、多模板路由、run_card 路径
- [ ] 新建 `vibe-debate` SKILL：`delegate_task` bull/bear 辩论
- [ ] 更新 `finance/stocks` SKILL：回测走 vibe-research

### Hermes 能力启用
- [ ] 多 agent：`delegate_task` 配合投委会
- [ ] 记忆：`memory` 存储风险偏好、标的池
- [ ] 历史：`session_search` 上次回测结论
- [ ] 定时：`cronjob` 收盘复盘

### 工程
- [ ] 库拆分（可选）：`hermes-vibe` → `hermes-vibe-data` + `hermes-vibe-backtest`
- [ ] `run_card.json` 持久化到 `~/.hermes/vibe/runs/`

---

## 🔲 未完成（P2 — 研究台）

**验收目标**：基准对比、券商 CSV 行为分析、因子 IC 子集、MCP 对外暴露。

### 新增 Tools
- [ ] `compare_benchmark` — 策略 vs SPY/CSI300
- [ ] `analyze_trade_journal` — 券商 CSV → 行为统计
- [ ] `run_factor_ic` — 风格因子 IC 子集（toraniko + factors）
- [ ] `get_quote` — 轻量现价查询
- [ ] `trading_account_read` — 券商只读账户/持仓（alpacars / ibapi）

### 新增 Skills
- [ ] `vibe-journal` — CSV 路径 + analyze_trade_journal
- [ ] `vibe-factor` — 何时 run_factor_ic；IC 局限
- [ ] `vibe-cron` — cronjob 收盘/周报配方
- [ ] 更新 `vibe-research` — benchmark、因子、只读账户

### MCP & 离线
- [ ] `hermes mcp serve` 对外暴露 5–8 个 vibe tools
- [ ] `hermes-mcp` client 连接 Robinhood MCP
- [ ] `rustdx` 离线 Parquet 导入

---

## 🔲 未完成（P3+ / Backlog）

### Tools 候选
- [ ] `export_pine` — TradingView Pine Script 导出
- [ ] `walk_forward_validate` — 防过拟合
- [ ] `portfolio_backtest` — 多标的组合回测
- [ ] `correlation_matrix` — 组合相关性
- [ ] `options_price` — 期权理论价（RustQuant）
- [ ] `trading_place_order` — 下单（需 mandate 栈）

### Skills 候选
- [ ] `vibe-shadow` — Shadow Account HTML 报告
- [ ] `vibe-options` — 期权研究
- [ ] `vibe-crypto-perp` — 永续合约/资金费率

---

## ❌ 明确不做（0py 路线图外）

- Alpha Zoo 452 全量 port
- Python 用户策略 `import`
- futu / baostock 官方 SDK
- PDF Shadow（WeasyPrint）
- `read_document` 全格式
- smartmoneyconcepts / pyharmonics
- Vibe React UI 全量重写
- AKTools / Python akshare / mootdx 依赖
- 挂 akshare-mcp 42 tools

---

## 关键文件索引

| 模块 | 路径 |
|------|------|
| Vibe 库 | `crates/hermes-vibe/src/` |
| 数据提供者 | `crates/hermes-vibe/src/providers/` |
| 回测引擎 | `crates/hermes-vibe/src/backtest.rs` |
| Tool Handler | `crates/hermes-tools/src/tools/vibe_market_data.rs` |
| Tool Handler | `crates/hermes-tools/src/tools/vibe_backtest.rs` |
| 注册 | `crates/hermes-tools/src/register/vibe.rs` |
| Skill | `skills/finance/vibe-research/SKILL.md` |
| Parity | `crates/hermes-parity-tests/fixtures/vibe_*/` |
| 路线图 | `docs/roadmaps/VIBE_TRADING_RUST_REWRITE.md` |
