# Vibe-Trading → Hermes 0py Rust 重写规划

> **状态**: 规划草案（0py 优先）  
> **上游**: [HKUDS/Vibe-Trading](https://github.com/HKUDS/Vibe-Trading) v0.1.9  
> **本仓**: `hermes-agent-ultra` — 复用 Hermes Rust 栈，不引入 Python 运行时  
> **接入排期**: §3 Tools & Skills（P0 / P1 / P2）  
> **最后更新**: 2026-06-15

---

## 0. 硬约束：什么叫 0py

| 允许 | 禁止 |
|------|------|
| Rust crate、`reqwest` 调公开 HTTP API | `python3`、PyO3、子进程调 Python |
| Hermes 已有 Rust 能力（agent/MCP/skills） | AKTools 等 **背后依赖 Python 的服务** 作为产品依赖 |
| 外部券商/数据 **HTTP/WebSocket/MCP** | 用户/LLM 生成代码的 **Python `import` 回测** |
| 本地文件（TDX day、Parquet cache） | WeasyPrint / pypdfium2 等 Python 文档栈 |

**产品名建议**：**Hermes Vibe Research (0py)** — 非 Vibe-Trading 完整 parity。

**Python 参考仓路径**：`C:\Users\15059\hermes-agent`（仅做行为参照，非编译依赖；本地路径可按实际 clone 位置调整）。

---

## 1. 总览：Vibe 能力 × 0py 可实现性

图例：**✅ 0py 可做** · **🟡 部分/需自研补全** · **❌ 0py 暂不可做** · **—** 用 Hermes 已有能力

### 1.1 Agent / 平台层

| Vibe 能力 | 0py | Rust / Hermes 实现 | 说明 |
|-----------|-----|-------------------|------|
| Agent 循环（48 tools） | — | `hermes-agent` `AgentLoop` | **不移植** LangChain；只挂工具 |
| MCP server（36 tools） | ✅ | `hermes-mcp` / `rmcp` | 小 MVP 只暴露 2–5 个 vibe tool；整包 36+42 详见 §10 反模式 |
| MCP client（Robinhood 等） | ✅ | `hermes-mcp` client | 远程 MCP 无 Python |
| Session / JSONL | — | `hermes-agent/session_persistence` | SQLite + FTS |
| 跨会话 Memory | 🟡 | 自写 `~/.hermes/vibe/memory/` 或扩 session | Vibe markdown 文件模式可仿 |
| Skills 77 条（markdown） | — | `hermes-skills` | 内容可迁入；**执行**靠 tools |
| skill_writer 自进化 | 🟡 | `hermes-skills` curator | 非 MVP |
| Web UI (React) | ❌ MVP | 保留 Vibe 前端或 Hermes TUI | API 契约另议 |
| REST API + SSE | ✅ | `hermes-http` / `axum` | Phase 2+ |
| Provider 13+（reasoning 等） | — | `hermes-intelligence` | 已覆盖 |
| `web_search` | — | Hermes 内置 | 不新写 |
| `read_url` | — | Hermes `web_extract` | SSRF 走现有策略 |
| `bash` / shell tools | — | Hermes `terminal` + `approval` | 默认严格门控 |
| Research Goal | 🟡 | 新 `hermes-vibe-research` | Goal store + evidence |
| Hypothesis Registry | 🟡 | SQLite + tool | Phase 3+ |
| Swarm 29 preset | 🟡 | `sub_agent_orchestrator` + YAML | ≠ Vibe DAG 全量 |
| CLI TUI | — | `hermes-cli` / Ratatui | 不移植 Vibe Rich UI |

### 1.2 行情数据（Vibe 9 loaders + auto）

| Vibe loader / 数据 | 0py | 首选 Rust crate | 备选 | 可实现功能 |
|--------------------|-----|-----------------|------|------------|
| **akshare** | ✅ | [`akshare`](https://crates.io/crates/akshare) [akshare-rs](https://github.com/Cricle/akshare-rs) v0.1 | [`tenk`](https://crates.io/crates/tenk) v0.2 | A/HK/US 股、ETF、债、期货、期权、外汇、crypto、宏观、指数 |
| **yfinance** | ✅ | 同上 `akshare`（含 Yahoo 源） | [`yfinance-rs`](https://crates.io/crates/yfinance-rs) v0.8、`yahoo_finance_api` v4.1 | OHLCV、quote、部分基本面 |
| **tushare** | ✅ | [`tushare-api`](https://crates.io/crates/tushare-api) v1.2 | `tushare-rs-pro`、`akshare` 内 Tushare 源 | 日线、财务、宏观；**PIT 需自测** |
| **mootdx** 在线 TCP | 🟡 | [`akshare`](https://crates.io/crates/akshare) / `tenk` 东财源 | — | 在线 K 线；非协议级兼容 |
| **mootdx** 本地 TDX | ✅ | [`rustdx`](https://github.com/zjp-CN/rustdx) + `rustdx-cmd` | → Parquet 缓存 | 全市场历史 day、复权、东财增量 |
| **okx** | ✅ | `akshare` crypto / [`ccxt-rust`](https://crates.io/crates/ccxt-rust) | `digdigdig3`、手写 REST | 现货/合约 OHLCV、 ticker |
| **ccxt** 多所 | ✅ | `ccxt-rust`、`digdigdig3`（47 所）、[lotusx](https://github.com/createMonster/lotusx) | — | Binance/OKX/Bybit 等 |
| **baostock** | 🟡 | `akshare` / `tushare-api` 覆盖部分场景 | — | 无专用 crate |
| **futu** OpenD | ❌ | — | — | 无官方 Rust SDK |
| **tencent** HTTP | ✅ | `akshare` / `tenk` | — | A 股 HTTP 源已含 |
| **auto fallback** | 🟡 | 自研 `MarketDataProvider` trait | `akshare → yfinance-rs → error` | 需显式降级表，非 Vibe 开箱 |
| Disk cache | ✅ | 自写 `~/.hermes/vibe/cache` | Parquet / duckdb | settled bar 缓存 |
| `get_market_data` tool | ✅ | `hermes-vibe-data` | — | MVP 核心 |

**非 0py（勿作产品依赖）**：Python akshare、AKTools HTTP、mootdx Python、baostock Python。

### 1.3 回测（Vibe 7 engines + composite + options）

| Vibe 引擎 / 能力 | 0py | Rust 实现 | 可实现 | 缺口 |
|------------------|-----|-----------|--------|------|
| `global_equity` 美股/HK | ✅ | 自写 `hermes-vibe-backtest` + Polars | SMA/RSI 模板、日频撮合、指标 | 涨跌停可选 |
| `crypto` | ✅ | 同上 | 24/7、无 T+1 | — |
| `china_a` A 股 | 🟡 | 同上 + **T+1** 规则 | 日线回测、涨跌停简化版 | 规则需自写 |
| `china_futures` / `global_futures` | 🟡 | Polars + 保证金模型 | Phase 3+ | 无开箱引擎 |
| `forex` | 🟡 | Polars | Phase 3+ | — |
| `options_portfolio` | ❌ MVP | `RustQuant` 定价参考 | 期权理论价 | 组合回测自研 |
| `composite` 跨市场资金池 | 🟡 | 自研 | Phase 4+ | 汇率/保证金 |
| 用户 Python `signal_engine` | ❌ | — | — | 改 **Rust 模板** 或未来 WASM/DSL |
| 动态 `import` 策略 | ❌ | — | — | 0py 明确不做 |
| `backtest` / `run_backtest` tool | ✅ | `hermes-vibe-backtest` | MVP 核心 | — |
| Monte Carlo / walk-forward | 🟡 | [`nt-backtesting`](https://crates.io/crates/nt-backtesting) 或自写 | Phase 3+ | — |
| Benchmark（SPY/CSI300） | 🟡 | `akshare` / `yfinance-rs` 拉基准 | Phase 2+ | — |
| `run_card.json` / md 报告 | ✅ | `serde` + 模板 | HTML/md 原生 | PDF 另议 |
| Pine/MT5/TDX 导出 | 🟡 | 自写模板字符串 | Phase 4+ | 无 crate |
| `factor_analysis` | 🟡 | `toraniko` + `factors` | 风格因子 IC | ≠ Alpha101 |
| `pattern_recognition` | ❌ | — | — | 无 SMC 等价库 |
| `analyze_options` | 🟡 | `RustQuant` | 理论定价 | 非 Vibe 全量 |

**回测框架可选（非 MVP）**：

| Crate | 用途 | 0py | MVP |
|-------|------|-----|-----|
| 自写 Polars + [`ta-lib-in-rust`](https://crates.io/crates/ta-lib-in-rust) | 日线模板回测 | ✅ | **推荐** |
| [`nyxs_owl`](https://crates.io/crates/nyxs_owl) | 带回测的 Polars 框架 | ✅ | 可选 |
| [`nt-backtesting`](https://crates.io/crates/nt-backtesting) | 事件驱动、40+ 指标 | ✅ | 偏重 |
| [barter-rs](https://github.com/barter-rs/barter-rs) | 回测+实盘一体 | ✅ | v2+ |
| [rustrade](https://github.com/Niqnil/rustrade) | barter 分支 + IBKR/Alpaca | ✅ | v2+ |
| [`hftbacktest`](https://crates.io/crates/hftbacktest) | 高频 tick | ✅ | overkill |
| [`alfars`](https://lib.rs/crates/alfars) | 因子 GP + 回测 | ❌ | **含 PyO3** |

### 1.4 因子 / Alpha Zoo

| Vibe 能力 | 0py | Rust 实现 | 说明 |
|-----------|-----|-----------|------|
| Alpha Zoo 452（qlib158/101/gtja191） | ❌ | — | 数年项目；勿 MVP |
| `alpha bench` / `compare` | 🟡 | [`toraniko`](https://crates.io/crates/toraniko) + [`factors`](https://crates.io/crates/factors) | Barra 风格子集 IC |
| 滚动算子（rank/corr/decay） | 🟡 | 自写 Polars / `ndarray` | [py-alpha-lib](https://github.com/msd-rs/py-alpha-lib) 有 PyO3，**不用** |
| `alpha_zoo` browse | 🟡 | 静态 YAML manifest | 无执行 |

### 1.5 交易 / 券商（Vibe 10 connectors）

| Vibe connector | 0py 只读 | 0py 下单 | Rust 实现 | 说明 |
|----------------|----------|----------|-----------|------|
| **IBKR** | ✅ | 🟡 | [`ibapi`](https://github.com/wboayue/rust-ibapi) / [`ibkr`](https://docs.rs/ibkr) | 连本地 TWS/Gateway |
| **Alpaca** | ✅ | 🟡 | [`alpacars`](https://github.com/kfuangsung/alpacars) | paper/live |
| **OKX / Binance** | ✅ | 🟡 | `ccxt-rust` / barter / digdigdig3 | 需 mandate |
| **Robinhood** MCP | ✅ | 🟡 | `hermes-mcp` client | OAuth 远程 MCP |
| Tiger / Futu / Longbridge / Dhan / Shoonya | ❌/🟡 | ❌ | 无成熟 Rust SDK | 长期缺口 |
| mandate / order_guard / kill switch | ✅ | ✅ | **自研 `hermes-vibe-trading`** | 必须原生 Rust |
| `trading_*` tools | 🟡 | 🟡 | 上表 + 安全栈 | **MVP 全禁写** |

### 1.6 研究附属

| Vibe 能力 | 0py | Rust 实现 | 说明 |
|-----------|-----|-----------|------|
| Shadow Account 报告 | 🟡 | HTML/md + 自写图表 | PDF 用 `printpdf` 或放弃 |
| `analyze_trade_journal` | 🟡 | `csv` crate 解析 + Polars | 同花顺/东财/富途 CSV |
| `read_document` PDF/DOCX/XLSX | ❌ MVP | — | 后续：`pdf-extract`、`calamine` 等分批 |
| Trade journal PDF | ❌ | — | — |
| 相关性热力图 | 🟡 | Polars corr + 前端或 CLI | Phase 3+ |
| 持久化 runs / swarm | ✅ | SQLite / 文件树 | `~/.hermes/vibe/runs/` |

### 1.7 技术指标

| Python 库 | 0py | Rust crate | 覆盖 |
|-----------|-----|------------|------|
| pandas TA / TA-Lib | ✅ | [`ta-lib-in-rust`](https://crates.io/crates/ta-lib-in-rust) (`rustalib`) | SMA, EMA, RSI, MACD, BB, ATR, OBV… |
| smartmoneyconcepts | ❌ | — | 形态识别仍缺 |
| pyharmonics | ❌ | — | 谐波仍缺 |
| sklearn ML skill | 🟡 | [`linfa`](https://crates.io/crates/linfa) | Phase 4+；非 MVP |

---

## 2. Rust 生态项目清单（按类别）

> **成熟度**：🟢 生产倾向 · 🟡 早期可用 · 🔴 无等价 · ⚠️ 含 Python/非 0py

### 2.1 中国市场 & 全球行情

| 项目 | crates.io / 仓库 | 成熟度 | 0py | 主要能力 |
|------|------------------|--------|-----|----------|
| **akshare-rs** | `akshare` · [Cricle/akshare-rs](https://github.com/Cricle/akshare-rs) | 🟡 v0.1 | ✅ | **主库**；A/HK/US/基金/期货/期权/外汇/crypto/宏观 |
| akshare-mcp | 同仓 `akshare-mcp` | 🟡 | ✅ | ~42 MCP tools；MVP **库直调**，勿整包挂 |
| **tenk** | `tenk` · [peitaosu/tenk](https://github.com/peitaosu/tenk) | 🟡 v0.2 | ✅ | A 股/ETF/债；东财/新浪/THS；CLI + `tenk --mcp` |
| **rustdx** | `rustdx` · [zjp-CN/rustdx](https://github.com/zjp-CN/rustdx) | 🟡 | ✅ | 本地通达信 day、东财增量、复权、导出 DB |
| **tushare-api** | `tushare-api` · [rock117/tushare-api](https://github.com/rock117/tushare-api) | 🟢 v1.2 | ✅ | Tushare Pro 全 API；限流重试 |
| tushare-rs-pro | `tushare-rs-pro` | 🟡 | ✅ | 77 预定义数据模型 |
| tushare → Polars | `tushare` v0.1 | 🟡 | ✅ | 轻量 DataFrame 输出 |
| **yfinance-rs** | `yfinance-rs` · [gramistella/yfinance-rs](https://github.com/gramistella/yfinance-rs) | 🟢 v0.8 | ✅ | Yahoo 全功能 port；2026 活跃 |
| yahoo_finance_api | `yahoo_finance_api` v4.1 | 🟢 | ✅ | 轻量 OHLCV；17 万+ 下载 |
| yahoo-finance-rs | `yahoo-finance-rs` | 🟡 | ✅ | 另一 Python yfinance port |
| baostock | — | 🔴 | ❌ | 用 akshare / tushare-api |
| futu OpenD | — | 🔴 | ❌ | — |
| AKTools | HTTP 包装 Python | ⚠️ | ❌ | 仅开发对比 |

### 2.2 加密货币 / 交易所

| 项目 | 成熟度 | 0py | 主要能力 |
|------|--------|-----|----------|
| **ccxt-rust** | 🟡 | ✅ | Binance, OKX, Bybit, Bitget, Hyperliquid… |
| **digdigdig3** | 🟡 | ✅ | 47 连接器；18 TRUSTED CEX；WS+REST |
| **lotusx** | 🟡 | ✅ | 7 所统一 API（Binance/Bybit/Hyperliquid…） |
| **barter-data** | 🟢 | ✅ | 行情流；与 barter 生态配套 |
| **rustrade-data** | 🟡 | ✅ | Hyperliquid、IBKR、Alpaca、Databento… |

### 2.3 回测 & 交易框架

| 项目 | 成熟度 | 0py | 主要能力 |
|------|--------|-----|----------|
| **hermes-vibe-backtest**（自研） | 计划 | ✅ | 模板策略、T+1、run_card |
| **ta-lib-in-rust** | 🟢 | ✅ | 技术指标 |
| **nyxs_owl** | 🟡 | ✅ | Polars 回测 + 预测模块 |
| **nt-backtesting** | 🟡 | ✅ | 事件驱动、MC、walk-forward |
| **barter-rs** | 🟢 | ✅ | 引擎 + 数据 + 执行 + 回测 |
| **rustrade** | 🟡 | ✅ | barter 式；多交易所 |
| **hftbacktest** | 🟢 | ✅ | 订单簿级 HF；MVP 不需要 |
| **alfars** | ⚠️ PyO3 | ❌ | 因子挖掘；**排除** |

### 2.4 因子 & 量化库

| 项目 | 成熟度 | 0py | 主要能力 |
|------|--------|-----|----------|
| **factors** | 🟡 | ✅ | 动量/价值/质量/规模等风格因子 |
| **toraniko** | 🟡 | ✅ | Barra 式因子收益 WLS |
| **RustQuant** | 🟡 hobby | ✅ | 期权定价、Monte Carlo（非回测主线） |
| py-alpha-lib | ⚠️ PyO3 | ❌ | 滚动算子；**排除** |
| Alpha Zoo 452 | 🔴 | ❌ | — |

### 2.5 券商 SDK

| 项目 | 成熟度 | 0py | 主要能力 |
|------|--------|-----|----------|
| **ibapi** / **ibkr** | 🟢 | ✅ | IBKR TWS/Gateway |
| **alpacars** | 🟢 | ✅ | Alpaca 交易+行情 |
| Robinhood | — | ✅ | 远程 MCP via `hermes-mcp` |
| Dhan/Shoonya/Longbridge/Futu | 🔴 | ❌ | — |

### 2.6 Hermes 本仓（直接复用）

| Crate | 0py 可覆盖的 Vibe 面 |
|-------|---------------------|
| `hermes-agent` | Agent loop、子 agent、session |
| `hermes-tools` | Tool registry、approval、terminal、web |
| `hermes-mcp` | MCP client/server |
| `hermes-skills` | Skill markdown、guard、hub |
| `hermes-intelligence` | 多 Provider、routing、pricing |
| `hermes-http` | API server |
| `hermes-auth` | OAuth |
| `hermes-config` | MCP 配置、路径 |
| `hermes-parity-tests` | Golden fixture 框架 |
| `hermes-cli` / `alpha_runtime` | Mission Control、风险看板（启发式） |

### 2.7 基础依赖（workspace 统一）

| 用途 | Crate |
|------|-------|
| DataFrame | `polars` |
| 数组 | `ndarray` |
| HTTP | `reqwest` |
| 异步 | `tokio` |
| 序列化 | `serde` / `serde_json` |
| 时间 | `chrono` |
| 本地分析缓存 | `duckdb` 或 Parquet |
| MCP SDK | `rmcp`（经 `hermes-mcp`） |

---

## 3. Tools & Skills 接入排期（P0 / P1 / P2 / 后续）

> **接入原则**：**Tool 干活，Skill 指路，Hermes 编排**。  
> - **Tool** = `hermes-tools` 新注册 + `crates/hermes-vibe` 库  
> - **Skill** = `optional-skills/finance/` 下 `SKILL.md`（无 Python 执行层）  
> - **Hermes 复用** = 不新写，仅在 skill 里写清何时调用  

### 3.0 排期总览

| 优先级 | 周期（估） | 目标 | 新增 Tool | 新增/更新 Skill |
|--------|------------|------|-----------|-----------------|
| **P0** | 2–3 人周 | 对话内可演示「拉数 + 回测」 | 2 | 1 |
| **P1** | +3–4 人周 | 多市场 + 持久化 + 投委会 | +0~1 | 2 |
| **P2** | +4–6 人周 | 研究台：基准、流水、因子、定时 | +3~5 | 2~3 |
| **后续** | 按需 | 交易只读、Alpha 子集、Shadow | +N | 若干 |

### 3.1 P0 — 必须交付（MVP 可 demo）

**验收**：`hermes chat` 完成「拉 BTC 或 A 股 K 线 → 20/50 均线回测 → JSON 指标」，且 agent **未编造**数字。

#### 3.1.1 Tools（新建）

| ID | Tool 名 | toolset | 实现 | 依赖 crate | 验收 |
|----|---------|---------|------|------------|------|
| T-P0-01 | `get_market_data` | `vibe` | `hermes-tools/tools/vibe_market_data.rs` | `hermes-vibe` → `akshare` | 返回 OHLCV JSON；symbol+日期范围 |
| T-P0-02 | `run_backtest` | `vibe` | `hermes-tools/tools/vibe_backtest.rs` | `hermes-vibe` → Polars + `ta-lib-in-rust` | `sma_cross(20,50)`；return/dd/trade_count |

**工程任务**：

| 任务 | 路径 | 说明 |
|------|------|------|
| 库 crate | `crates/hermes-vibe/` | `MarketDataProvider`、`BacktestEngine` |
| 注册模块 | `crates/hermes-tools/src/register/vibe.rs` | `pub fn register(ctx)` — 每个 tool 用 `reg(ctx, "vibe", ...)` 注册；在 `register/mod.rs` 的 `register_all` 中加 `#[cfg(feature = "vibe-research")] vibe::register(ctx)` |
| feature | `crates/hermes-tools/Cargo.toml` | `vibe-research = ["dep:hermes-vibe"]`；`full = [..., "vibe-research"]` |
| workspace | `Cargo.toml` | `members` 加 `"crates/hermes-vibe"`；`workspace.dependencies` 加 `hermes-vibe` |
| golden | `crates/hermes-parity-tests/fixtures/vibe_*` | mock JSON parity |

> **注意**：toolset 名称是注册时传入 `reg()` 的字符串参数（如 `"web"`、`"skills"`），不存在 `toolset.rs` 常量文件。vibe toolset 统一使用 `"vibe"` 字符串。

**P0 市场范围**：`BTC-USDT`（必测）+ `000001.SZ` 或 `600519.SH`（二选一先通）。

**P0 明确不做**：独立 `get_backtest_report`、T+1、disk cache、第三策略。

```toml
# P0 依赖（hermes-vibe/Cargo.toml）
akshare = "0.1"
ta-lib-in-rust = "1.0"
polars = { version = "0.46", features = ["lazy", "temporal"] }
```

> ⚠️ `akshare` v0.1 较年轻，P0 W1 需做可行性 spike：验证 A 股 + crypto 核心接口可用，否则需手写 `reqwest` HTTP 回退。

#### 3.1.2 Skills（新建）

| ID | Skill | 路径 | 作用 |
|----|-------|------|------|
| S-P0-01 | `vibe-research` | `skills/finance/vibe-research/SKILL.md` | 路由：何时 `get_market_data` / `run_backtest` vs `web_search`；**禁止编造回测数字** |

**SKILL.md 必含**：When to Use / When NOT to Use · 工具调用顺序 · 与 `stocks` skill 关系 · Verification 示例 prompt。**不新建** Python `scripts/`。

#### 3.1.3 Hermes 复用（P0 仅配置）

| 能力 | 组件 | P0 配置 |
|------|------|---------|
| Agent 循环 | `AgentLoop` | 默认 |
| 联网研究 | `web_search`, `web_extract` | Cargo feature `web` |
| 读写报告 | `read_file`, `write_file` | 默认 |
| 技能加载 | `skills_list`, `skill_view` | 默认 |
| 门控 | `approval` | 默认 |

```yaml
# config.yaml — Hermes 现有配置机制
# tools 字段控制运行时启用的 tool 名列表（默认含 bash/read/write/edit/glob/grep/web_search/web_fetch）
# platform_toolsets 按平台映射 toolset 名称（如 cli → hermes-cli）
# P0 场景：编译时启用 vibe-research feature 即可注册 get_market_data / run_backtest
# tools:
#   - bash
#   - read
#   - write
#   - web_search
#   - web_fetch
#   - get_market_data    # vibe tool
#   - run_backtest       # vibe tool
# P0 不启：delegation, cronjob, terminal, execute_code
```

#### 3.1.4 P0 里程碑

| 周 | 交付 |
|----|------|
| W1 | `hermes-vibe` crate 搭建 + akshare-rs 可行性 spike + `get_market_data` + unit test |
| W2 | `run_backtest` + parity fixture |
| W3 | `vibe-research` SKILL + `hermes chat` 端到端 |

---

### 3.2 P1 — 增强（小完整研究闭环）

**验收**：A/HK/US/crypto 各至少 1 symbol 回测；`run_card.json` 可复盘；可选投委会 `delegate_task`。

#### 3.2.1 Tools（扩展）

| ID | Tool 名 | 变更 | 说明 |
|----|---------|------|------|
| T-P1-01 | `get_market_data` | **增强** | 多市场 symbol；可选 `source`；`VIBE_DATA_CACHE` |
| T-P1-02 | `run_backtest` | **增强** | + `rsi_revert`；+ Sharpe；A 股 **T+1** |
| T-P1-03 | `get_backtest_report` | **可选** | 读 `~/.hermes/vibe/runs/{id}/run_card.json` |

**库拆分（可选）**：`hermes-vibe` → `hermes-vibe-data` + `hermes-vibe-backtest`。

#### 3.2.2 Skills（新建/更新）

| ID | Skill | 类型 | 说明 |
|----|-------|------|------|
| S-P1-01 | `vibe-research` | 更新 | T+1、多模板、run_card 路径 |
| S-P1-02 | `vibe-debate` | 新建 | `delegate_task` bull/bear；子 agent toolset=`vibe,web` |
| S-P1-03 | `finance/stocks` | 更新说明 | 回测走 `vibe-research`；quote-only 仍可用 terminal（非 0py） |

#### 3.2.3 Hermes 复用（P1 开启）

| 能力 | 组件 | P1 用途 |
|------|------|---------|
| 多 agent | `delegate_task` | 配合 `vibe-debate` |
| 记忆 | `memory` | 风险偏好、标的池 |
| 历史 | `session_search` | 上次回测结论 |
| 定时 | `cronjob` | 可选收盘复盘 |

```yaml
active_toolsets: [web, file, skills, memory, delegation, vibe]
```

#### 3.2.4 P1 里程碑

| 周 | 交付 |
|----|------|
| W4 | T+1 + RSI + 多市场 smoke |
| W5 | `run_card.json` + cache |
| W6 | `vibe-debate` + delegate 端到端 |

---

### 3.3 P2 — 研究台（仍 0py，仍 Hermes 编排）

**验收**：基准对比、券商 CSV 行为分析、因子 IC 子集、MCP 对外 5 个 tool。

#### 3.3.1 Tools（新建）

| ID | Tool 名 | 说明 | 底层 |
|----|---------|------|------|
| T-P2-01 | `compare_benchmark` | 策略 vs SPY/CSI300 | `get_market_data` ×2 |
| T-P2-02 | `analyze_trade_journal` | 券商 CSV → 行为统计 | `csv` + Polars |
| T-P2-03 | `run_factor_ic` | 风格因子 IC 子集 | `toraniko` + `factors` |
| T-P2-04 | `get_quote` | 轻量现价（可选） | `akshare` |
| T-P2-05 | `trading_account_read` | 券商只读账户/持仓 | `alpacars` / `ibapi` |

**Research Goal（可选 P2 末）**：`research_goal_create` + `research_goal_evidence`（SQLite store）。

#### 3.3.2 Skills（新建/更新）

| ID | Skill | 说明 |
|----|-------|------|
| S-P2-01 | `vibe-journal` | CSV 路径 + `analyze_trade_journal` |
| S-P2-02 | `vibe-factor` | 何时 `run_factor_ic`；IC 局限 |
| S-P2-03 | `vibe-cron` | `cronjob` 收盘/周报配方 |
| S-P2-04 | `vibe-research` | 更新 benchmark、因子、只读账户 |

#### 3.3.3 Hermes / MCP（P2）

| 项 | 说明 |
|----|------|
| `hermes mcp serve` | 对外 filter 5–8 个 vibe tools |
| `hermes-mcp` client | 可选 Robinhood MCP 只读 |
| `rustdx` 离线 | Parquet 导入；`get_market_data` 读本地 |

#### 3.3.4 P2 里程碑

| 周 | 交付 |
|----|------|
| W7–8 | benchmark + trade journal |
| W9–10 | factor IC + MCP serve |
| W11–12 | 券商只读 + research goal（可选） |

---

### 3.4 后续新增功能建议（P3+ / backlog）

#### Tools 候选

| 优先级 | Tool | 价值 | 依赖 |
|--------|------|------|------|
| 高 | `export_pine` | TradingView 导出 | 模板字符串 |
| 高 | `walk_forward_validate` | 防过拟合 | `nt-backtesting` |
| 中 | `portfolio_backtest` | 多标的组合 | composite 引擎 |
| 中 | `correlation_matrix` | 组合相关性 | Polars |
| 中 | `options_price` | 期权理论价 | `RustQuant` |
| 低 | `trading_place_order` | 下单 | mandate 栈先于 tool |
| 低 | `alpha_bench` | 452 因子子集 | 长期或永不 |

#### Skills 候选

| Skill | 说明 |
|-------|------|
| `vibe-shadow` | Shadow Account HTML 报告 |
| `vibe-options` | 期权研究 + `options_price` |
| `vibe-crypto-perp` | 永续/资金费率 + ccxt |
| Vibe 77 条迁移 | 仅 markdown 路由改写 |

#### 明确不在 0py 路线图内

- Alpha Zoo 452 全量 port  
- Python 用户策略 `import`  
- futu / baostock 官方 SDK  
- PDF Shadow（WeasyPrint）  
- `read_document` 全格式  
- smartmoneyconcepts / pyharmonics  
- Vibe React UI 全量重写  
- AKTools / Python akshare / mootdx 依赖  
- 挂 `akshare-mcp` 42 tools  

---

### 3.5 Tools vs Skills 分工（速查）

| 用户需求 | Tool | Skill |
|----------|------|-------|
| 拉 K 线 | `get_market_data` | `vibe-research` |
| 回测 | `run_backtest` | 模板说明、禁止幻觉 |
| 新闻/宏观 | `web_search` | `vibe-research` 分流 |
| 多空辩论 | `delegate_task` | `vibe-debate` |
| 定时复盘 | `cronjob` | `vibe-cron` |
| 券商 CSV | `analyze_trade_journal` | `vibe-journal` |
| 因子 IC | `run_factor_ic` | `vibe-factor` |
| 写 md 报告 | `write_file` | `vibe-research` 输出结构 |

### 3.6 旧档位对应（历史参考）

| 旧档位 | 新排期 | 说明 |
|--------|--------|------|
| 档 S | **P0** | MVP demo |
| 档 M | **P0 尾 + P1 前半** | 多市场增强 |
| 档 L | **P1 尾 + P2** | 研究台 |
| 档 XL | **§3.4 后续** | 交易/因子 |

> 此表仅为历史参考，外部读者可忽略。

---

## 4. Hermes 原生集成（优先复用，不另起炉灶）

> **原则**：只新增 **量化专用 Rust tool**；编排、搜索、记忆、多 agent、会话、MCP 外壳 **全部走 Hermes 现成能力**。

### 4.1 能力对照：Vibe → 用 Hermes 什么

| Vibe 能力 | 集成方式 | Hermes 现成组件 | 是否新写 |
|-----------|----------|-----------------|----------|
| Agent 主循环 | **直接用** | `hermes-agent::AgentLoop` | ❌ |
| 自然语言研究对话 | **直接用** | `hermes chat` / gateway | ❌ |
| 联网搜新闻/研报 | **直接用** | `web_search`（Exa / DDG） | ❌ |
| 读网页/公告 | **直接用** | `web_extract` | ❌ |
| 读本地 md/csv/json | **直接用** | `read_file` / `search_files` | ❌ |
| 写 run 报告、笔记 | **直接用** | `write_file` / `patch` | ❌ |
| 技能说明（何时回测） | **直接用** | `skills_list` + `skill_view` + `optional-skills/finance/` | ❌（可加 1 页 SKILL.md） |
| 跨会话偏好 | **直接用** | `memory` | ❌ |
| 搜历史对话 | **直接用** | `session_search` | ❌ |
| 多分析师「投委会」 | **直接用** | `delegate_task` → 子 `AgentLoop` | ❌ |
| 定时盯盘/复盘 | **直接用** | `cronjob` | ❌ |
| 任务拆解 | **直接用** | `todo` | ❌ |
| 用户澄清 | **直接用** | `clarify` | ❌ |
| 接外部 MCP（Robinhood 等） | **直接用** | `hermes-mcp` **client** + `mcp_servers` 配置 | ❌ |
| 对外暴露工具 | **直接用** | `hermes mcp serve`（注册新 tool 即可） | ❌ |
| LLM 多模型 | **直接用** | `hermes-intelligence` | ❌ |
| 危险命令门控 | **直接用** | `approval` + tool policy | ❌ |
| Mission / 交易看板 | **可选** | `hermes-cli` `alpha_runtime` | ❌ |
| **拉 OHLCV 行情** | **新增 tool** | `get_market_data` → `akshare` crate | ✅ 唯一核心新能力 |
| **模板策略回测** | **新增 tool** | `run_backtest` → Polars + `ta-lib-in-rust` | ✅ 唯一核心新能力 |
| **回测 run 元数据** | **新增 tool**（可选） | `get_backtest_report` 或合并进 `run_backtest` | ✅ 小 |
| Research Goal 状态机 | 后期 | SQLite + tool | 🟡 P2 |
| 券商只读 | 后期 | `alpacars`/`ibapi` wrapper | 🟡 P2 |

### 4.2 推荐 toolset 配置（finance 研究 profile）

Hermes 工具启用分两层：**编译时 Cargo feature** + **运行时 `config.yaml` 的 `tools` 列表**。

编译时：在 `hermes-tools/Cargo.toml` 中启用 feature：
```toml
# 编译时启用 vibe（full 默认已含）
hermes-tools = { workspace = true, features = ["vibe-research"] }
```

运行时：在 `config.yaml` 的 `tools` 列表追加 vibe tool 名：
```yaml
# config.yaml — 量化研究 profile
tools:
  - bash
  - read
  - write
  - web_search
  - web_fetch
  - skills_list
  - skill_view
  - memory
  - delegate_task
  - get_market_data     # vibe
  - run_backtest        # vibe
# 默认不启：browser, execute_code（除非用户明确要）
```

### 4.3 典型用户任务 → 工具链（全 Hermes 编排）

| 用户意图 | Agent 应调用的工具（顺序） |
|----------|---------------------------|
| 「NVDA 最近有什么新闻」 | `web_search` → 可选 `web_extract` |
| 「拉 NVDA 2024 日线」 | `get_market_data` |
| 「回测 NVDA 20/50 均线」 | `get_market_data` → `run_backtest` |
| 「写一份 md 复盘到 runs/」 | `run_backtest` → `write_file` |
| 「多空双方辩论是否买入 BTC」 | `delegate_task` ×2（bull/bear 子 agent，各带 `get_market_data`） |
| 「记住我只做日线、最大回撤 10%」 | `memory` |
| 「每天收盘跑一遍策略」 | `cronjob` |
| 「查上次回测说了什么」 | `session_search` |
| 复杂研报 PDF | ❌ MVP 不做；可用 `web_search` 替代 |

### 4.4 只新增什么代码

```
crates/hermes-vibe/          # 仅：akshare 适配 + Polars 回测（纯库）
crates/hermes-tools/
  register/vibe.rs             # reg() 注册 2 个 ToolHandler
  tools/vibe_market_data.rs
  tools/vibe_backtest.rs
skills/finance/
  vibe-research/SKILL.md       # 教 agent：何时用 get_market_data vs web_search vs stocks skill
```

**不要新建**：`VibeAgentLoop`、独立 CLI loop、独立 MCP server 二进制、独立 web_search。

### 4.5 与 `optional-skills/finance/stocks` 的关系

| 场景 | 用谁 |
|------|------|
| 快速 quote、无回测 | 0py 路线：**`get_market_data`**（Rust）；或保留 stocks skill + `terminal`（有 Python） |
| 回测、A 股、多市场 OHLCV | **必须** `get_market_data` + `run_backtest` |
| 研报、新闻 | **`web_search`**，不要用行情 tool |

### 4.6 架构图（Hermes 原生）

```
用户 → hermes chat / gateway / mcp serve
         │
         ▼
    AgentLoop（唯一 loop）
         │
    ┌────┴────────────────────────────────────────────┐
    │ Hermes 已有                                      │ 新增（薄）
    │ web_search · web_extract · read/write_file      │ get_market_data
    │ skills_* · memory · session_search              │ run_backtest
    │ delegate_task · cronjob · clarify · approval    │
    │ hermes-mcp client（可选券商 MCP）                │
    └─────────────────────────────────────────────────┘
         │
    skill_view("vibe-research")  ← 可选 SKILL.md 路由
```

---

## 5. 推荐架构（0py + 数据层）

| 排期 | Crate | 职责 |
|------|-------|------|
| P0 | `hermes-vibe` | data + backtest 合一 |
| P1 | `hermes-vibe-data` + `hermes-vibe-backtest` | 可选拆分 |
| P2 | `hermes-vibe-research` | Goal、evidence（可选） |
| 后续 | `hermes-vibe-trading` / `hermes-vibe-factors` | 下单 / 因子 |

工具注册：`hermes-tools/src/register/vibe.rs` → `register_all` + feature `vibe-research`。

---

## 6. Vibe MCP 36 tools → Hermes 映射速查

> 详细集成方式见 §4.1 能力对照表。

| Vibe MCP tool | 用 Hermes 什么 | 备注 |
|---------------|----------------|------|
| `get_market_data` | **新增** `get_market_data` | 唯一核心新 tool |
| `backtest` | **新增** `run_backtest` | 唯一核心新 tool |
| `list_skills` / `load_skill` | `skills_list` / `skill_view` | 已有 |
| `web_search` | `web_search` | 已有 |
| `read_url` | `web_extract` | 已有 |
| `read_file` / `write_file` | `read_file` / `write_file` | 已有 |
| `read_document` | ❌ MVP | 后续 `calamine` / `pdf-extract` |
| `run_swarm` | `delegate_task` | 已有 |
| `start_research_goal` 等 | P2 新 tool 或 `todo` + `memory` | 待定 |
| `trading_*` | 后续 + `hermes-mcp` client | P0–P2 禁写 |
| `alpha_bench` 等 | P2 `toraniko` 或 ❌ | 长期 |

---

## 7. 困难点（0py 语境，精简）

| 严重度 | 困难点 |
|--------|--------|
| 🔴 | `akshare-rs` v0.1 年轻；爬虫源可能同时失效；需 P0 W1 spike 确认 |
| 🔴 | 无用户 Python 策略；Alpha Zoo 452 无捷径 |
| 🔴 | 工具幻觉 — 需 skill 强制走 `run_backtest`，禁止编造数字 |
| 🔴 | `ccxt-rust` 为社区项目，非 ccxt 官方 Rust port；API 覆盖度需验证 |
| 🟡 | T+1/涨跌停自写；长任务超时 |
| 🟡 | 行情 API 测试需网络 mock（wiremock / fixture），CI 环境无法访问真实数据源 |
| 🟢 | 其余编排交给 Hermes，不重复造轮子 |

---

## 8. 测试与工程

```bash
# 单元 + 集成
cargo test -p hermes-vibe
cargo test -p hermes-vibe-backtest   # P1 拆分后
cargo test -p hermes-vibe-data

# 0py 守门：确保无 Python 依赖混入
rg -i 'python|pyo3' crates/hermes-vibe*

# Parity golden（遵循 AGENTS.md）
cargo test -p hermes-parity-tests

# Clippy
clippy -p hermes-vibe -- -D warnings

# 网络隔离测试：使用 wiremock mock 行情 API
cargo test -p hermes-vibe --features test-mock

# 端到端验收
hermes chat  # 回测验收
```

遵循 `AGENTS.md`：单模块 PR、parity golden、clippy。

---

## 9. 风险矩阵

| 风险 | 缓解 |
|------|------|
| akshare-rs 变更 / API 缺失 | pin 版本 + golden fixture；P0 W1 spike 做 go/no-go |
| akshare-rs 不可用时的 Plan B | `MarketDataProvider` trait 设计预留 provider 切换；备选 `yfinance-rs` / `tushare-api` / 手写 `reqwest` |
| 工具过多 | P0 只加 2 个 vibe tool |
| 实盘事故 | P0–P2 禁写；`hermes-vibe-trading` 需 mandate 栈先于 tool |
| ccxt-rust 社区维护 | 仅 crypto 场景依赖；A 股/美股不经过 ccxt |

---

## 10. 反模式

| 不要 | 原因 |
|------|------|
| 另写 `VibeAgentLoop` | 用 `AgentLoop` |
| 自写 web_search / session / skills | Hermes 已有 |
| 挂 `akshare-mcp` 42 tools | 工具淹没 |
| AKTools / Python sidecar | 违反 0py |

---

## 11. 待决问题

1. P0 先 crypto only 还是首版含 A 股？（§3.1 建议二选一先通）
2. P1 是否拆 `get_backtest_report` 独立 tool？
3. finance profile 默认 toolset 是否含 `delegation`（P0 否 / P1 是）？
4. akshare-rs 不满足 P0 需求时的 Plan B：切 `yfinance-rs` / `tushare-api` 还是手写 `reqwest` HTTP？
5. 是否需要 `hermes-vibe` 的 `test-mock` feature 来隔离网络测试？

---

## 12. 参考链接

| 类别 | 链接 |
|------|------|
| 上游 | https://github.com/HKUDS/Vibe-Trading |
| 行情 | https://github.com/Cricle/akshare-rs |
| Hermes MCP | `docs/mcp.md` |
| Hermes SOP | `AGENTS.md` |

---

*参考上游 Vibe-Trading v0.1.9（Python）行为；编排以 Hermes `AgentLoop` 为唯一入口；接入排期见 **§3**。*
