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

**P1 总体验收目标**：
- [ ] A-share / HK / US / crypto 四类市场各至少 1 个 symbol 能成功回测。
- [ ] 回测结果持久化为 `~/.hermes/vibe/runs/{id}/run_card.json`，并可通过 tool 读取复盘。
- [ ] 新增 `rsi_revert` 策略模板，A 股 T+1 规则生效。
- [ ] `cargo test -p hermes-vibe` 和 `cargo test -p hermes-parity-tests` 全部通过。
- [ ] `cargo clippy -p hermes-vibe -p hermes-parity-tests -- -D warnings` 通过。

### Tools 增强

#### `get_market_data`
- [ ] 支持显式 `source` 参数
  - 验收：`source` 可选值为 `auto|binance|eastmoney`，默认 `auto`。
  - 验收：`source=binance` 时只走 BinanceProvider，`source=eastmoney` 时只走 EastmoneyProvider。
  - 验收：新增/更新 parity fixture 覆盖 `source` 参数。
- [ ] 支持 HK / US 市场 symbol 格式（至少设计好路由规则）
  - 验收：`HK_00700` 或 `0700.HK` 格式能识别为待接入状态（可 mock）。
  - 验收：非法 market 返回清晰错误。
- [ ] 实现 `VIBE_DATA_CACHE` 磁盘缓存
  - 验收：缓存目录为 `~/.hermes/vibe/cache/`。
  - 验收：缓存 key 格式为 `{source}-{symbol}-{interval}-{start}-{end}.json`。
  - 验收：默认缓存有效期 24h；过期后重新请求网络。
  - 验收：同一请求在缓存有效期内只触发一次网络调用（单测验证）。
  - 验收：缓存可手动清空或绕过。

#### `run_backtest`
- [ ] 新增 `rsi_revert` 策略模板
  - 验收：默认参数 `rsi_period=14`, `oversold=30`, `overbought=70`。
  - 验收：在 mock 数据上产生至少 1 笔交易。
  - 验收：新增 parity fixture `vibe_backtest/rsi_revert.json`。
- [ ] A 股 T+1 规则
  - 验收：当日买入信号不成交，下一交易日开盘价成交。
  - 验收：卖出信号当日可成交（A 股 T+1 只限制买入后当日卖出）。
  - 验收：新增单测覆盖 T+1 与 T+0 的差异。
- [ ] Sharpe 改进
  - 验收：使用收益率序列（而非 equity curve 步长）计算年化 Sharpe。
  - 验收：提供 `risk_free_rate` 参数，默认 0.0。

#### `get_backtest_report`（可选）
- [ ] 读取 `~/.hermes/vibe/runs/{id}/run_card.json`
  - 验收：`{id}` 支持 UUID 或时间戳格式。
  - 验收：文件不存在时返回清晰错误。
  - 验收：返回 JSON 包含 run_card 全部字段。

### Skills

- [ ] 更新 `vibe-research` SKILL
  - 验收：When to Use 增加 `rsi_revert` 说明。
  - 验收：增加 T+1 规则说明（A-share 回测默认启用）。
  - 验收：增加 `run_card.json` 保存路径说明。
  - 验收：验证示例 prompt 覆盖 `rsi_revert`。
- [ ] 新建 `vibe-debate` SKILL
  - 验收：路径 `skills/finance/vibe-debate/SKILL.md`。
  - 验收：frontmatter `name: vibe-debate`。
  - 验收：使用 `delegate_task` 触发 bull/bear 两个子 agent。
  - 验收：输出格式为 pros/cons 结论摘要。
- [ ] 更新 `finance/stocks` SKILL
  - 验收：When to Use 明确 “历史 OHLCV / 回测” 走 `vibe-research`。
  - 验收：保留 stocks skill 对 quote/company search 的职责。

### Hermes 能力启用

- [ ] 多 agent：投委会
  - 验收：通过 `delegate_task` 并行触发 bull 和 bear agent。
  - 验收：输入包含 symbol、strategy、run_card 摘要。
  - 验收：输出统一格式（如 `{"bull": "...", "bear": "...", "consensus": "..."}`）。
- [ ] 记忆：风险偏好、标的池
  - 验收：使用 `memory` tool 存储用户风险等级（保守/稳健/积极）。
  - 验收：使用 `memory` tool 存储用户关注标的列表。
  - 验收：回测前自动读取风险偏好并提示。
- [ ] 历史：`session_search` 上次回测结论
  - 验收：通过 `session_search` 找到最近 7 天内包含 `run_backtest` 的会话。
  - 验收：能提取上次回测的 symbol、strategy、total_return_pct。
- [ ] 定时：`cronjob` 收盘复盘
  - 验收：支持 `hermes cron` 配置每日收盘后运行预设 symbol 回测。
  - 验收：输出保存到 `~/.hermes/vibe/runs/`。

### 工程

- [ ] `run_card.json` 持久化到 `~/.hermes/vibe/runs/`
  - 验收：每次 `run_backtest` 成功后将 RunCard 写入 `{id}/run_card.json`。
  - 验收：`id` 生成规则明确（建议使用 `{symbol}-{strategy}-{timestamp}` 或 UUID）。
  - 验收：目录不存在时自动创建。
- [ ] 库拆分（可选）：`hermes-vibe` → `hermes-vibe-data` + `hermes-vibe-backtest`
  - 验收：如执行拆分，`hermes-tools` 依赖保持不变或更清晰。
  - 验收：拆不拆不影响 P1 总体验收。

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
