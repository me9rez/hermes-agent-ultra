# SOP: Equity Research Data Layer (UZI HTTP parity)

| 字段 | 值 |
|------|-----|
| Scope | `hermes-trading` A-share fetchers / providers |
| UZI repo | `wbh604/UZI-Skill` → `skills/deep-analysis/scripts/` |
| Rust | `crates/hermes-trading/src/providers/akshare/` + `eastmoney_http.rs` |

## 数据源优先级（A 股硬数据）

| 维度 | 主路 (akshare-rs 0.1.7) | Fallback |
|------|-------------------------|----------|
| basic / get_quote | `a_share_quote` | `eastmoney_http` push2 → 腾讯 qt |
| kline | `a_share_candles` + 本地 MA/RSI | push2his |
| financials | `stock_financial_abstract` + `stock_financial_analysis_indicator` | emweb F10 |
| capital_flow | `a_share_capital_flow` + `stock_hsgt_individual_em` + margin SSE/SZSE | push2 fflow |
| lhb | `stock_lhb_stock_detail_date_em` | datacenter |
| research | `stock_research_report_em` | — (非 A 股 web_search) |
| events | `a_share_announcements` + `stock_news_em` | — (非 A 股 web_search) |

技术指标（MA/RSI/stage）始终由 `kline_util` 从 OHLCV 本地计算，不依赖 akshare 指标端点。

## 必读 UZI 文件（按优先级）

1. **`lib/data_sources.py`** — A 股 basic/kline fallback 链（push2 → 腾讯 → 新浪 …）
2. **`lib/providers/direct_http_provider.py`** — 腾讯/新浪直连 UA 与解析
3. **`lib/market_router.py`** — symbol 归一化（`.SH`/`.SZ`/港股/美股）
4. **`lib/network_preflight.py`** — 国内域可达性诊断
5. **`pipeline/fetchers/fetch_*.py`** — **仅**字段/schema 参考，不含 transport 防御

## UZI → Rust 映射

| UZI 逻辑 | Rust 模块 | Transport 要求 |
|----------|-----------|----------------|
| `fetch_a_share_basic` push2 直连 | `eastmoney_http::fetch_push2_quote` | `ut` + UA + Referer |
| 腾讯 qt fallback | `eastmoney_http::fetch_tencent_qt` | UA + Referer |
| push2his kline | `eastmoney_http::fetch_push2_klines` | `ut` + Referer |
| push2 fflow | `eastmoney_http::fetch_push2_fflow_klines` | `ut` + Referer |
| `parse_ticker` | `symbol::normalize_symbol` | `.SS`→`.SH` 等 |
| 合并 snapshot | `eastmoney_http::fetch_a_share_snapshot` | push2 → 腾讯 |
| akshare 主路 | `providers/akshare/*` | akshare-rs → eastmoney fallback |
| API 字段字面量 | `providers/akshare/labels.rs` | 财务/基金列名集中维护 |
| 中文名解析 | `providers/akshare/symbol_resolve.rs` | `stock_info_a_code_name` 缓存 |
| basic 维 fallback | `research/fetchers/dims/basic.rs` | akshare → basic → QuoteRouter |

## P1a HTTP Transport Gate（blocking）

在**新增或修改**任何 `research/fetchers/dims/*.rs` 之前：

1. 所有 `push2.eastmoney.com` / `push2his.eastmoney.com` **fallback 路径**调用必须经 [`eastmoney_http.rs`](../../crates/hermes-trading/src/providers/eastmoney_http.rs)（akshare 主路失败时触发）
2. `cargo test -p hermes-trading` 通过
3. `cargo clippy -p hermes-trading -- -D warnings` 通过
4. 本地可选：`cargo test -p hermes-trading -- --ignored live_`

**禁止**：在 `eastmoney_basic.rs` / `eastmoney_quote.rs` / `eastmoney.rs` / `eastmoney_capital_flow.rs` 中直接拼 push2 URL（应走 `eastmoney_http`）。

## 移植 checklist（每个新 provider）

- [ ] Headers：`User-Agent`（`http::BROWSER_USER_AGENT`）、`Referer`、`ut`
- [ ] Symbol：`normalize_symbol` + `EastmoneyProvider::to_secid`
- [ ] Fallback：push2 失败时有独立 host 备选（至少腾讯 qt）
- [ ] 失败语义：`DimQuality::Partial` 或显式 `error`，不 silent empty
- [ ] 单测：parse fixture + merge/failover 逻辑

## 验证

```bash
cargo build -p hermes-trading
cargo test -p hermes-trading --lib research::fetchers
cargo test -p hermes-parity-tests equity_research
cargo clippy -p hermes-trading -- -D warnings
cargo fmt -p hermes-trading
```

CI 在 `.github/workflows/ci.yml` 的 **Equity research gate** step 跑上述命令子集（PR 必过）。

## akshare-rs 版本 pin 与升级策略

| 项 | 说明 |
|----|------|
| **Pin 位置** | 根 [`Cargo.toml`](../../Cargo.toml) `[workspace.dependencies] akshare = "0.1.7"`；`hermes-trading` 用 `{ workspace = true }` |
| **升级流程** | 1) bump workspace 版本 → 2) `cargo build -p hermes-trading` → 3) `cargo test -p hermes-parity-tests equity_research` → 4) 本地 `--ignored live_` 冒烟 |
| **阻断规则** | golden / parity 任一失败 **禁止合并**；不得 silent 改 `expected` 骗过测试 |
| **Eastmoney fallback** | akshare 升级失败时仍依赖 `eastmoney_http`；主路 API 变更优先修 provider，再考虑 bump |

## collect 并行 benchmark

`CollectOptions.parallel`（默认 `true`）在同一 dependency layer 内用 `join_all` 并发拉维；`parallel: false` 为层内串行（benchmark 对照）。

```bash
cargo test -p hermes-trading benchmark_collect_parallel_vs_serial_600519_medium -- --ignored --nocapture
```

结果记录见 [`docs/insights/COLLECT_BENCHMARK_2026-06-25.md`](../insights/COLLECT_BENCHMARK_2026-06-25.md)。

## P2 未实现（刻意跳过）

新浪 hq、百度估值、baostock、雪球、Playwright — 见 [`EQUITY_RESEARCH_TODO.md`](../../EQUITY_RESEARCH_TODO.md) §4。
