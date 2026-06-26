# Hermes 终态 · 只剩项 Checklist

> **终态定义**：A 股单票 `symbol → collect → 模型 → 评分 → 66 人格 → JSON（+ 可选精简 HTML）` 闭环可跑、可测、可复现；`data_confidence` / `used_fallback` 可信；约 **11 维硬数据够深 + 7 维 LLM/web**；不追 UZI 600KB / Python / Playwright。
>
> 对照：[`EQUITY_RESEARCH_TODO.md`](EQUITY_RESEARCH_TODO.md)（分期 TODO）、[`docs/sop/equity_research_data.md`](docs/sop/equity_research_data.md)（取数 SOP）。
>
> **粗估剩余量（相对 Hermes 终态）**：约 15～20%（J 节 UZI 全量硬编码、collect 并行、9 路 capital_flow 等刻意延后）。

---

## A. 端到端产品（用户能稳定用起来）

- [x] **`use_providers` 默认开启**：`analyze_stock` schema 或 tool 默认值改为 `true`（深度研报路径）
- [x] **Skill 强制顺序**：`resolve_a_share_symbol` → `get_quote` → `analyze_stock(use_providers=true)` →（低 confidence 时）`web_search` → LLM 叙事
- [x] **低置信度规则**：`data_confidence.score < 0.5` 时 skill 禁止 institutional-grade 话术，并强制 web 补 FCF/债务/同业
- [x] **中文名 → 代码**：`resolve_a_share_symbol` 工具 + akshare `stock_info_a_code_name` 缓存
- [x] **实测验收用例**：3 股 offline fetcher golden（茅台 / 亏损 / 小市值）`trading_research_fetch/fetcher_golden.json`
- [x] **Slash 命令**：`/quick-scan`（lite）、`/analyze-stock` 与 `/equity-research`（medium）经 skill frontmatter `commands:` 注册
- [ ] **WeCom / 关键词**（若需要）：`/equity-research` 或「分析一下」触发更稳定（可选）

---

## B. 22 维取数（硬数据加深）

### 已较完整 · 仅需加深 / 稳态

- [x] **1_financials**：`fcf_yi`、debt 历史进 mapper；bridge 消费 `fcf_yi`
- [x] **0_basic**：弱网 push2 失败仍 Partial+error；`cached_basic` 预填 + `needs_push2_merge`
- [x] **2_kline**：评分消费 `stage` / `ma_align` / `max_drawdown`
- [x] **10_valuation**：A 股分位失败 → `missing`（非静默 50 分）；复用 `cached_basic`
- [x] **4_peers**：medium 空 `peer_table` → `4_peers` missing
- [x] **6_fund_holders**：gdhs+基金表；`holder_change_ratio` 经 capital_flow 消费
- [x] **7_industry**：`industry_pe` / `growth` 进 scoring
- [x] **12_capital_flow**：北向/主力/户数/融资融券 UZI 子集 + golden
- [ ] **16_lhb** / **6_research** / **15_events**：fetcher 字段与 scoring 消费 grep 对齐

### 故意 LLM/web · 终态 = 规则清晰即可

- [x] **3_macro / 5_chain / … / 19_contests**：medium 缺数进 `missing_dims`；skill 绑定 `missing_dims` + `dim_summary`
- [ ] **18_trap**：规则部分移植（可选）；判断仍交 LLM

### 工程

- [x] **collect 并行**：`depends_on` 拓扑分层 + `join_all`
- [x] **跨维缓存**：`cached_quote` + `cached_basic` 预填/复用
- [x] **dim_summary** 写入 `analyze_stock` JSON

---

## C. snapshot / bridge / 可信度

- [x] **bridge 扩维**：`4_peers`、`6_fund_holders`、`12_capital_flow`、`15_events`、`6_research` merge + provenance
- [x] **DataConfidence 权重**：扩展 `CONFIDENCE_FIELDS`（含 roe / pe_quantile / industry / debt_ratio）
- [x] **缺维进 `missing_dims`**：dim_key 级去重；medium 不用 `neutral_dim` 伪装缺数维
- [x] **provenance 端到端**：bridge 标记 capital_flow / valuation 关键字段

---

## D. 19 维评分（消费 fetcher 真数据）

- [x] **逐维对照 UZI `score_fns.py`**：medium web 维缺数不再 `neutral_dim` 伪装
- [x] **7_industry**：用 `industry_pe` / `growth` 算分
- [x] **6_research**：用研报条数
- [x] **6_fund_holders**：经 capital_flow 维消费户数变化
- [x] **18_trap**：读 `15_events` 新闻标题关键词，非写死 9 分

---

## E. 估值模型（输入质量，非新模型）

- [x] **3 股模型 golden**：`models_golden.json` 茅台 / 亏损 / 小市值 DCF
- [x] **修复 parity**：`trading_parity` 跳过 `trading_research*`；`equity_research_parity` 处理 `compute_wacc`
- [x] **used_fallback 清单**：DCF fcf + shares fallback 单测

---

## F. 66 人格 panel

- [ ] **规则 vs UZI YAML 抽样对照**（Buffett / 成长 / 周期各 1）
- [ ] **缺 `pe_quantile_5y` / FCF 时 vote 行为**：不默认「通过」
- [x] **panel golden**：`panel_buffett_smoke` 确定性 + `persona_panel` op

---

## G. 报告（Hermes 终态 = 精简，非 600KB）

- [x] **HTML**：DCF 敏感性中心格、19 维分数表、panel、`used_fallback` 与置信度 gauge
- [x] **SVG**：置信度 gauge + PE 5 年分位条
- [x] **`format=html` 在 skill 里推广**为「发研报」可选格式

---

## H. 测试与回归（终态门禁）

- [x] **Fetcher golden**：3 股 offline mapper golden（`trading_research_fetch/`）
- [x] **录入脚本**：`record_equity_research_fixtures.py` 引用 fetcher golden 路径
- [ ] **CI**：`cargo test -p hermes-trading` + `cargo test -p hermes-parity-tests` 全绿
- [ ] **akshare 升级策略**：版本 pin + golden 失败即阻断合并

---

## I. 文档与维护

- [x] **更新 `EQUITY_RESEARCH_TODO.md`** / 本 checklist 勾选
- [x] **`docs/sop/equity_research_data.md`**：labels + symbol_resolve
- [x] **API 字面量常量文件**：`providers/akshare/labels.rs`

---

## J. 明确不在 Hermes 终态（不必勾选）

- Python akshare 运行时
- Playwright / 雪球 token / iwencai
- 22 维全硬编码
- 600KB HTML 体积目标
- 66 人格评语规则化（交 LLM）

---

## 建议执行顺序

1. **A** — 产品能稳定走 pipeline
2. **H** — 3 股 golden，防后面改坏
3. **B + C + D** — 取数 → bridge → 评分（绑在一起做）
4. **E + F** — 模型与人格校准
5. **G** — 报告体验
6. **I** — 文档与常量收口

---

## 当前完成度快照（2026-06-18，4 波终态计划后）

| 模块 | 约完成度 |
|------|----------|
| 估值模型 DCF/Comps/LBO/三表 | ~90% |
| 22 维取数 | ~65% |
| 19 维评分 | ~75% |
| bridge / 可信度 | ~80% |
| 66 人格 | ~80% |
| 报告 HTML/SVG | ~70% |
| Agent 编排 | ~85% |
| 测试回归 | ~75% |
| **整体（Hermes 终态）** | **~80～85%** |
