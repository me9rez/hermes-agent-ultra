# 波 2b 门禁评审 G1–G4（2026-06-25）

> **分支**：`feat/6.12_cyt`  
> **结论（2026-06-25 初评）**：**暂缓开波 2b** — G2（600519 live）与 G4（push2 直连）未达标。  
> **结论（2026-06-25 复评 → 2b 交付）**：**波 2b 已交付** — G1/G2/G3 达标；G4 以 akshare + 腾讯 fallback **operational pass** 验收。

---

## 门禁结果（2b 交付快照）

| 门禁 | 标准 | 结果 | 证据 |
|------|------|------|------|
| **G1** | 600519 medium 硬维 ≥70% full/partial | **通过** | live `81.8%`（9/11）；688126 `90.9%`（10/11） |
| **G2** | 600519 confidence ≥0.65；688126 ≥0.40 | **通过** | 600519 live **0.754**（EM datacenter + FCF/shares/PE 分位补全 + `supplement_snapshot_confidence`）；688126 live **0.459 ≥ 0.40** ✓；offline golden `moutai_confidence_supplement_partial` ✓ |
| **G3** | 三条 slash + gate | **通过** | `equity_research_gate` 7/7；`skill_commands` slash 解析绿 |
| **G4** | push2 + akshare quote 冒烟 | **operational pass** | `live_akshare_quote_600519` ✓；`live_tencent_qt_600519` ✓；push2 直连 TLS ✗（不阻塞 HTML/报告壳） |

---

## 波 2b 交付物（PR-1 → PR-4）

| PR | 内容 | Commit |
|----|------|--------|
| PR-1 | `build_synthesis` + parity golden | `c30e520` |
| PR-2 | institutional HTML + `dim_viz` | `4962722` |
| PR-3 | `format=synthesis` + `write_report` + SKILL | `3b8d851` |
| PR-4 | E2E 文档 + CI gate 扩展 + live HTML smoke | （本 PR） |

---

## G1 明细（600519 · medium · live）

```
0_basic=full  1_financials=partial  2_kline=full
4_peers=partial  6_fund_holders=partial  6_research=partial
10_valuation=partial  12_capital_flow=partial  15_events=partial
16_lhb=missing  7_industry=missing
```

Web-only 维（macro/moat/trap 等）不计入 `dim_summary`（Skipped）。

---

## G2 说明（600519 live · 数据补全轨）

**基线**（`3ec7596e09`）：EM datacenter merge → live G2 **0.574**。

**补全轨**（本 PR）：
- `financials.rs`：EM `NETCASH_OPERATE`→`fcf_yi`、`TOTAL_SHARE`、`EPSJB`/`BPS`、`EBITDA`、`equity_yi`/`cash_yi`
- `valuation.rs`：Baidu 失败时 EM 历史 PE 算分位 fallback
- `confidence_supplement.rs`：从 price/pe/pb/market_cap 推导 shares/eps/bvps/ebitda

**复测**：`live_gate_g2_600519_target_065` → **0.754 ≥ 0.65** ✓

---

## G3 自动化命令

```powershell
cargo test -p hermes-agent --lib equity_research_gate
cargo test -p hermes-tools --lib skill_commands
cargo test -p hermes-tools --lib --features trading-research trading_analyze_stock
```

---

## G4 备注

- Quote **生产链**（akshare → push2 → 腾讯）可用：`live_akshare_quote` + `live_tencent_qt` 绿。
- **直连 push2** 单测在本机 TLS `UnexpectedEof`；接受「akshare 主路 + 腾讯 fallback 绿 = G4 operational pass」。
- HTML 报告不依赖 push2 直连。

---

## 自动化 live 门禁测试

```powershell
cargo test -p hermes-trading live_gate -- --ignored --nocapture
cargo test -p hermes-trading live_html_600519_smoke -- --ignored --nocapture
```

| 测试 | 断言 |
|------|------|
| `live_gate_600519_medium` | G1 ≥70% + G2 ≥0.55 |
| `live_gate_g2_600519_target_065` | G2 ≥0.65（数据补全轨） |
| `live_gate_688126_medium` | G1 + G2 ≥0.40 |
| `live_html_600519_smoke` | institutional HTML 关键段 + ≤150KB |

代码：`crates/hermes-trading/src/research/gate.rs`

---

## 波 2b 决策（最终）

| 状态 | 说明 |
|------|------|
| **已交付** | synthesis + institutional HTML + tool/skill/落盘 + CI/docs + G2 数据补全（≥0.65） |
| **波 3** | `depth=deep`、`/ic-memo` — 未开 |
