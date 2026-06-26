# 66 人格校准抽样（轨 F · 2026-06-25）

对照 UZI `investor_criteria.py` 语义，在 Rust `personas/evaluator.rs` + `rules.rs` 做三条离线 golden 抽样。  
**不移植** narrative 评语；只校准确定性 vote / score / signal。

## 变更要点

| 场景 | 旧行为 | 新行为 |
|------|--------|--------|
| 缺 `pe_quantile_5y` | `unwrap_or(100)` 隐式 fail | 前置检查 + fail msg「数据缺失，规则不通过」 |
| 缺 FCF | `fcf_positive.unwrap_or(false)` | 无 `fcf_positive` 且无 `fcf_latest_yi` → 前置 fail；有 `fcf_latest_yi>0` 可推断 |
| vote confidence | 仅规则数量 | 缺 PE 分位 / FCF 各 −15，全缺失 fail 再 −10 |

## 三条 golden case

### 1 · 价值 · Buffett · 600519（低置信度）

**Fixture**: `panel_buffett_low_confidence`  
**输入**: 高 ROE / 净利率 / 低负债，但**无** `pe_quantile_5y`、**无** FCF  
**期望**:

- `buffett.signal = bearish`，`score ≤ 40`
- `fcf_positive` / `safety_margin_pe` 进入 fail（含「数据缺失」）
- 全 panel 均值仍约 77（medium 66 评委多数不依赖 PE 分位）；本 case 只断言 **Buffett 个体** fail-closed

**与 UZI 差异**: UZI YAML 对缺失字段通常不计入 pass；Rust 现显式 fail-closed，避免 silent pass。

**Golden 修正**: `panel_buffett_smoke` 的 `panel_consensus` 从错误的 `3.1` 更正为 `78.6`（修复 parity `features_from` 剥离 `raw_dims` 后的真实均值）。

### 2 · 成长 · Fisher · 300750（宁德时代类）

**Fixture**: `panel_fisher_growth_smoke`  
**输入**: `revenue_growth_latest=22%`，`roe_latest=24`，`net_margin=18%`  
**期望**: `fisher.signal = bullish`（三项成长规则权重通过）

**与 UZI 差异**: Rust 仅 3 条 Fisher 规则（rev_growth / roe_quality / margin_stable），无 YAML 里额外 narrative 权重。

### 3 · 周期 · Soros · 600157（周期下行）

**Fixture**: `panel_soros_cycle_smoke`  
**输入**: `stage=Stage 4 下跌`，`change_pct=-3.5`，亏损 PE  
**期望**: `soros.signal = bearish`（trend + momentum 均 fail）

**与 UZI 差异**: Soros 在 UZI 可能引用宏观/reflexivity 维；Rust 仅 kline stage + ma_align 代理。

## 验证

```powershell
cargo test -p hermes-trading --lib research::personas
cargo test -p hermes-parity-tests equity_research
cargo clippy -p hermes-trading -- -D warnings
```
