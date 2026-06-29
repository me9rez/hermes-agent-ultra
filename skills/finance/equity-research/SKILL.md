---
name: equity-research
description: "A-share equity research: DCF, 19-dim scoring, 66-investor panel via analyze_stock. Slash: /quick-scan, /analyze-stock, /equity-research"
version: 0.1.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
commands:
  - name: quick-scan
    description: "30秒速判：8维+Top10评委+杀猪盘"
    template: |
      [MODE: quick-scan / depth=lite] Analyze: {args}

      Workflow:
      1. resolve_a_share_symbol if needed
      2. analyze_stock(symbol, depth=lite, use_providers=true) — paste quick-scan markdown verbatim
      3. Do NOT web_search or format=html
      4. Add ≤2 sentence one-liner (must cite score); do not expand to 66-judge table
  - name: analyze-stock
    description: "完整深度分析：22维+66评委+DCF"
    template: |
      [MODE: analyze-stock / depth=medium] Analyze: {args}

      Workflow:
      1. resolve_a_share_symbol if needed
      2. analyze_stock(symbol, depth=medium, use_providers=true) — system auto-delivers brief + HTML (web dims filled automatically)
      3. Optional: user explicitly requests refresh → web_search then merge_external_only
      4. Do not write long analysis in chat; one tool call is enough
metadata:
  hermes:
    tags: [Finance, Equity, Research, DCF, Valuation, A-Share]
    category: finance
    related_skills: [trading-research, spot-quote, dcf-model, comps-analysis]
    requires_toolsets: [trading, web]
---

# Equity Research Skill

Pure Rust institutional-style equity research — DCF, comps, 19-dimension scoring,
and 66-investor persona panel. **No Python runtime.**

Complements optional `dcf-model` (Excel) and `trading-research` (OHLCV/backtest).

## When to Use

- User wants **fundamentals, valuation, DCF, investment merit, or peer comparison** on a listed stock (name or code)
- User invokes **`/quick-scan`**, **`/analyze-stock`**, or **`/equity-research`** (optional args: stock name or symbol)
- User wants structured JSON with `data_confidence`, `used_fallback`, persona votes
- A-share pipeline (600519.SH, 000001.SZ, etc.)

## Agent Workflow

No gateway keyword routing — decide from **intent** (valuation vs spot price vs backtest) and **slash mode**:

| Mode | Tool | web_search | Output |
|------|------|------------|--------|
| `/quick-scan` | `analyze_stock(depth=lite)` | **自动**（治理 web 补数） | quick-scan markdown + ≤2句 one-liner |
| `/analyze-stock` / `/equity-research` | `analyze_stock(depth=medium)` | **自动**（宏观/政策/舆情/产业链等） | 短摘要 MD + HTML 附件（自动） |
| 用户要 **研报 / HTML / 发报告** | 同 `/analyze-stock`（已自动附 HTML）；可加 `narrative=…` 写入 HTML | 自动 | institutional HTML 附件；`write_report` 由系统自动处理 |
| 只要 **结构化结论 JSON** | `analyze_stock(format=synthesis)` | 自动 | `synthesis` + 核心指标，无 66 评委全表 |

1. If the request is **fundamental/valuation research**, call `skill_view(name="equity-research")` when unsure of the workflow.
2. **`resolve_a_share_symbol`** when the user gives a Chinese name or bare 6-digit code.
3. **`analyze_stock(symbol, depth=…, use_providers=true)`** — always **before** `web_search` for medium-depth research.
4. **`web_search`** — handled **inside** `analyze_stock` for web-only dims (macro/policy/sentiment/chain/moat…). For manual refresh after delivery: `analyze_stock(symbol, depth=medium, merge_external_only=true, external_context={...})`.
5. If user typed **`/quick-scan`**, **`/analyze-stock`**, or **`/equity-research …`**, treat the skill as loaded and run the matching row above.

## When NOT to Use

- User wants **only spot price** → `get_quote` + `spot-quote`
- User wants **K-line backtest** → `trading-research`
- User wants **Excel DCF workbook** → optional `dcf-model` skill
- User wants **news only** → `web_search`

## Slash commands

- **`/quick-scan 688126`** — lite: 8 core dims + Top 10 judges + trap; governance web-filled automatically
- **`/analyze-stock 沪硅产业`** — medium: HTTP 采集 + 自动 web 补数 + 66 judges + DCF + 自动 HTML
- **`/equity-research 山西汾酒`** — same as `/analyze-stock` (medium)
- **`/equity-research 600809.SH`** — same, with explicit symbol

## Workflow (mandatory order)

**Symbol format:** A-shares use `.SH` / `.SZ` (e.g. `600519.SH`). Do **not** use Yahoo suffix `.SS` — Hermes normalizes it, but prefer `.SH` in tool calls.

1. **`resolve_a_share_symbol(query)`** — when user gives Chinese name (e.g. `牧原股份`, `山西汾酒`), resolve to canonical symbol
2. **`analyze_stock(symbol, depth=medium, use_providers=true)`** — **next** (before web). Runs 22-dim HTTP fetchers + DCF/scoring/panel; returns `raw_dims`, `data_confidence`, `used_fallback`
   - **`depth=lite`** for `/quick-scan` only (Top 10 judges, quick-scan markdown)
   - Only pass manual `fundamentals` / `peers` when providers failed or user supplied research notes
3. **`get_quote(symbol)`** — optional spot check; not a substitute for `analyze_stock` on research requests
4. **`web_search`** — normally **not needed** (auto inside `analyze_stock`). Manual refresh only when user asks to update macro/policy/sentiment after delivery:
   - `analyze_stock(symbol, depth=medium, merge_external_only=true, external_context={...})`
5. **LLM narrative** — after pasting **`summary_markdown`** from `analyze_stock` (full 19 dims + 66 judges; do not shorten to 9 rows), add conclusion citing:
   - `data_confidence.score`, `missing_dims`, and `dim_summary`
   - `used_fallback` (never hide proxy/Fallback paths)
   - DCF `verdict` + persona `panel_consensus`
6. **Optional report delivery** — when user asks for 研报 / HTML / 发报告:
   - `analyze_stock(symbol, depth=medium, format=html, narrative=…)` — institutional one-page HTML
   - Add `write_report=true` to save under `{HERMES_HOME}/reports/{symbol}_{date}/`:
     - `full-report-standalone.html`
     - `analysis.json`
   - Response includes `report_paths` when `write_report=true`
7. **Optional `format=synthesis`** — slim JSON (`synthesis`, `data_confidence`, `missing_dims`, scores) when chat does not need full markdown tables
8. **Default medium output** remains `summary_markdown` + full JSON (no `format` param)

### Eastmoney API fallback

Tool layer (`get_quote`, `analyze_stock` basic/kline/financials dims) tries **akshare-rs → push2 → Tencent qt** automatically.

If both fail (push2.eastmoney.com unreachable):

1. **`get_market_data(symbol, source="eastmoney")`** — uses push2his endpoint, often works when quote endpoint is blocked. Latest `close` ≈ current price proxy.
2. **Web-extract financial pages** — search English: `"600519.SS stock price"`, `"Kweichow Moutai market cap"`. Check snippets from Investing.com, SimplyWallSt, companiesmarketcap.com, Yahoo Finance.
3. **Extract price from Sina snippet** — search `"贵州茅台" "最新价格"` and check snippet for today's price e.g. `"贵州茅台 1240.00 (-1.25%)"`.
4. **Manually estimate PE** from web-marketcap / web-earnings. Market cap from companiesmarketcap.com, net income from tradingeconomics.com.
5. **Deliver with data-availability warning** — label non-real-time data as estimated. Never claim institutional-grade when live quote was unavailable.

### Rules

- If `data_confidence.score < 0.5`: **do not** claim "institutional-grade" — say data is partial; run `web_search` for gaps before final narrative
- Always surface `used_fallback` fields in the user-facing summary
- Persona **commentary** is LLM-generated; Rust output is `{id, vote, score, cited_rule}` for all **66** investors in `personas.investors`
- **`summary_markdown`** in tool JSON is the canonical chat table — paste verbatim before your narrative (default medium path)
- `format=synthesis` for structured verdict only; `format=html` + `narrative` when user asks for 研报 / readable report
- `write_report=true` (medium only) saves HTML + JSON to `{HERMES_HOME}/reports/` and returns paths
- **`full-report-standalone.html` structure**: Hero（公司/现价/市值/PE/PB）→ 公司基本面 → 板块与同业 → 政策/宏观/舆情 → 资金与事件 → 19维评分 → 66评委 → DCF附录
- `use_providers` defaults **true**; set `false` only for quote-only smoke tests

## Example

```json
analyze_stock({
  "symbol": "600519.SH",
  "fundamentals": {
    "revenue_latest_yi": 1500,
    "fcf_latest_yi": 600,
    "net_margin": 52,
    "market_cap_yi": 21000,
    "shares_outstanding_yi": 12.56,
    "total_debt_yi": 30,
    "cash_yi": 1500,
    "roe_latest": 30,
    "moat_total": 35
  },
  "peers": [
    {"name": "五粮液", "pe": 18, "pb": 4.2},
    {"name": "泸州老窖", "pe": 16, "pb": 3.8}
  ]
})
```

## Toolsets

- **`trading`** — `resolve_a_share_symbol`, `get_quote`, `analyze_stock`
- **`web`** — `web_search` for fundamentals gap-fill (macro, policy, moat when not in `raw_dims`)
