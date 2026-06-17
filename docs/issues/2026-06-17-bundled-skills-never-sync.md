# Issue: 内置 Skills 未同步到用户目录，Agent 运行时「看不见」Skill 路由

> **状态**：open  
> **优先级**：P0（阻塞端到端用户场景，非开发机手工操作）  
> **发现场景**：WeCom 用户问「AAPL 现在多少钱」— 工具齐全但 Agent 未走 `stocks` / `trading-research` 路由  
> **日期**：2026-06-17

---

## 用户故事（不是开发故事）

一位用户：

1. 安装/运行 `hermes-agent-ultra`（Windows 发布包或 `cargo install`）
2. 配置 WeCom gateway，`platform_toolsets.wecom: [hermes-wecom]`
3. 在企业微信问：**「AAPL 现在多少钱」**

**用户期望**：Agent 读取 `trading-research` / `stocks` 的 SKILL 文档，走 `skill_view` → `terminal` → `stocks_client.py quote AAPL`（或提示安装 optional `stocks`）。

**实际行为**：

- `hermes skills list` → `(no skills installed)`
- `%LOCALAPPDATA%\hermes-agent-ultra\skills\` 为空
- Agent 即兴使用 `execute_code`（yfinance）+ `web_search`，Yahoo 限流后无数字报价
- 全程未调用 `skill_view`、`skills_install`、`terminal`

用户**不会**也**不应该**从 git 仓库 `Copy-Item` skills 到 home 目录——那是开发者工作流，不是产品行为。

---

## 复现步骤（用户路径）

```powershell
# 1. 使用已发布的 hermes-agent-ultra（或本地 build 的二进制，非 repo 根目录 cwd）
hermes-agent-ultra skills list
# → Installed skills (...): (no skills installed)

# 2. 检查 home
dir $env:LOCALAPPDATA\hermes-agent-ultra\skills
# → 空目录

# 3. 启动 gateway，WeCom 发送「AAPL 现在多少钱」
# → 日志可见 execute_code / web_search，无 skill_view
```

**对照**：仓库 `skills/finance/trading-research/SKILL.md` 已写明 quote → stocks，但运行时 Agent 无法 `skill_view` 到该文件。

---

## 根因分析

### 1. `sync_skills()` 存在但从未接入用户生命周期

`crates/hermes-skills/src/sync.rs` 实现了完整的 bundled sync：

- 首次复制 `skills/` → `~/.hermes/skills/`（manifest `.bundled_manifest`）
- 尊重用户修改（hash 比对，不覆盖 user-modified）
- 支持 `.no-bundled-skills` opt-out

但 **没有任何生产路径调用它**：

| 预期触发点（文档） | 实际 |
|---|---|
| 首次安装 | ❌ 未调用 |
| `hermes update` | ❌ `update/mod.rs` 只做二进制 OTA，不 sync skills |
| `gateway` 启动 | ❌ `gateway_runtime.rs` 只 `FileSkillStore::default_dir()`，不 sync |
| `hermes skills sync` | ❌ CLI 无此 action（parity 笔记声称已暴露，但未实现） |

`usage.rs` 明确注释：

```text
the sync_skills() code path that writes [manifest] is dead code
```

保护逻辑退化为编译期常量 `BUNDLED_SKILL_NAMES_FALLBACK`（curator 用），**不等于**运行时 SKILL 内容可用。

### 2. 用户文档与实现不一致

`website/docs/user-guide/features/skills.md` 写明：

> On install and on every `hermes update`, a sync pass copies those into `~/.hermes/skills/` ...

当前 Rust 实现**不满足**该承诺。

### 3. `hermes skills reset --restore` 是 stub

文档描述完整的 manifest reset + 从 bundled 恢复。  
`crates/hermes-cli/src/commands/skills/cli/lifecycle.rs::run_reset` 实际只写一个占位 `SKILL.md`，**未调用** `hermes_skills::reset_bundled_skill`。

### 4. 发布物可能未携带 `bundled_dir`

`sync_skills` 在 `bundled_dir` 不存在时直接 no-op：

```rust
if !config.bundled_dir.exists() {
    return Ok(SkillSyncResult::default());
}
```

Windows 发布包若只含 `hermes-agent-ultra.exe`、不含相邻 `skills/` 目录，即使用户手动 `skills sync`（待实现），也无法复制。  
Nix 文档提到 `HERMES_BUNDLED_SKILLS` wrapper，但普通 Windows 安装路径未设置。

### 5. Optional skills（`stocks`）是第二层问题

`stocks` 在 `optional-skills/`，设计上需 `hermes skills install official/finance/stocks`（从上游 GitHub 拉取）。  
这可以不是「首次安装自动带齐」，但：

- Agent 要先能 `skill_view("trading-research")` 才知道去 install stocks
- 若 bundled `trading-research` 本身未 sync，整条链路断裂

---

## 影响范围

| 受影响用户场景 | 说明 |
|---|---|
| WeCom / 任意 gateway 查价 | SKILL 路由文档不可见 |
| Vibe Trading P1 验收 | `trading-research`、`trading-debate` 文档改动对运行时无效 |
| 新用户 onboarding | `skills list` 空，与「Hermes 自带 80+ skills」认知冲突 |
| `skill_view` / `skills_list` 工具 | 返回空或找不到 finance skills |

---

## 建议修复方向（用户场景优先）

### A. 接入 sync 生命周期（必须）

在以下节点调用 `hermes_skills::sync_skills`（`SkillSyncConfig` 需正确解析 `bundled_dir`）：

1. **Gateway 启动**（幂等、quiet；有变更时 info 日志）
2. **`hermes update` 成功后**（与文档一致）
3. **新增 CLI**：`hermes skills sync`（手动修复 / 运维）
4. **可选**：`hermes-agent-ultra` 首次创建 `hermes_home` 时

`bundled_dir` 解析优先级建议：

```
HERMES_BUNDLED_SKILLS（目录）
→ 可执行文件同目录/skills（Windows/macOS 发布布局）
→ 开发环境：repo 根 skills/（仅 debug/dev）
```

### B. 发布物携带 skills（必须）

Windows/macOS release artifact 需包含 `skills/` 树，或 embed + 首次解压到 cache。  
验收：`bundled_dir.exists()` 在干净安装后为 true。

### C. 实现真实的 `skills reset`（应做）

将 `run_reset` 接到 `reset_bundled_skill` / `sync_skills`，支持 `--restore`、`--yes`（与文档一致）。  
在 CLI help 的 available actions 中加入 `reset`、`sync`。

### D. Spot quote without Python（已实现，2026-06）

Rust `get_quote` in `hermes-trading` + `trading-quote` toolset on `hermes-cli` / WeCom.
Gateway `prepend_finance_quote_skill_hint` requires `get_quote` in tool schemas.
Optional `stocks` skill retained for search/compare/history only.

### E. Optional skills 引导（应做，可 P1）

When user needs company search and `stocks` is not installed:

1. `skill_view("trading-research")` — **前提 A 已解决**
2. `skills_install official/finance/stocks` — 需网络，但无需用户手工复制

### F. 可观测性（应做）

Gateway 启动日志示例：

```
skills: synced 87 bundled (3 updated, 0 user-modified, bundled_dir=...)
```

或 warning：

```
skills: bundled_dir missing — no skills synced; run `hermes skills sync` after reinstall
```

---

## 验收标准（用户视角）

- [ ] 全新安装 + 启动 gateway 后，`hermes skills list` 包含 `trading-research`、`trading-debate` 等内置 skill
- [ ] **无需**从 git clone 目录手工复制任何文件
- [ ] `hermes update` 后新 bundled skill 自动出现在 home（未 user-modify 的）
- [ ] `hermes skills sync` 可手动触发且幂等
- [ ] `hermes skills reset trading-research --restore` 从 bundled 恢复真实 SKILL.md（非占位符）
- [x] WeCom「AAPL 现在多少钱」日志中出现 `get_quote`（数据源限流另论，路由须正确） — Rust quote + gateway finance quote hint

---

## Follow-up: quote 路由（2026-06-17）

Layered bootstrap 已让 `skills list` 可见；但 agent 仍可能跳过 tools 直接 `web_search`.

**补充修复（finance-only, v2 — Rust quote）：**

- `get_quote` tool in `hermes-trading` (Yahoo US/HK + Eastmoney A-share + Binance crypto)
- `trading-quote` toolset on `hermes-cli` (WeCom inherits via `hermes-wecom` → `hermes-cli`)
- Gateway `prepend_finance_quote_skill_hint` — spot price messages force `get_quote` first
- Bundled `stocks` removed; optional `optional-skills/finance/stocks` for search/compare/history

验收：日志 `tool=get_quote` → 失败时才 `web_search`；无 `skill_view(stocks)` / `terminal` / Python。


- ❌ 要求用户 `Copy-Item` 仓库 `skills/` → `%LOCALAPPDATA%`（开发 workaround，非产品方案）
- ❌ 仅靠改 SKILL 文档、不改 sync 接线（当前状态已证明无效）
- ❌ 把 optional `stocks` 全部塞进首次 bundled sync（可保留 hub install，但路由 skill 必须先可见）

---

## 相关代码与文档

| 资源 | 路径 |
|---|---|
| Sync 实现（未接线） | `crates/hermes-skills/src/sync.rs` |
| Dead code 注释 | `crates/hermes-skills/src/usage.rs` ~L516 |
| Gateway skill store | `crates/hermes-cli/src/gateway_runtime.rs` ~L230 |
| CLI reset stub | `crates/hermes-cli/src/commands/skills/cli/lifecycle.rs::run_reset` |
| CLI actions 列表缺 sync/reset | `crates/hermes-cli/src/commands/skills/cli/mod.rs` |
| 用户文档（承诺 sync） | `website/docs/user-guide/features/skills.md` § Bundled skill updates |
| Parity 声称已暴露 sync CLI | `docs/parity/queue-overrides.json` |
| Trading 受影响 TODO | `docs/roadmaps/VIBE_TRADING_TODO.md` |

---

## 关联 incident

- Session: `5f58f582-a05a-43d5-aa11-537bfcf5a066`
- WeCom: `woPMNBUgAA50oGJSjG6NXfy1WDQ5ufMg`
- 工具链：`execute_code` ×3 → `web_search` ×4 → `web_extract` ×1；无 `skill_view`
- Home: `C:\Users\38788\AppData\Local\hermes-agent-ultra\skills\` 空
