# SOP: `checkpoint_manager`

| 字段 | 值 |
|------|-----|
| registry `id` | `checkpoint_manager` |
| Python | `research/hermes-agent/tools/checkpoint_manager.py` |
| Rust（当前 parity） | `crates/hermes-parity-tests/src/harness.rs` — `checkpoint_shadow_dir_id` |
| Rust（完整移植目标） | `crates/hermes-tools/src/backends/checkpoint.rs`（尚未存在则创建） |
| Crate（完整功能） | `hermes-tools` |
| Fixtures | `crates/hermes-parity-tests/fixtures/checkpoint_manager/*.json` |
| registry `note` | 完整 snapshot/rollback 需 Rust checkpoint backend + git2 测试 |

## 前置

1. 读 Python `checkpoint_manager.py`：snapshot / rollback / list / diff 与 shadow 路径逻辑。
2. 读 golden `shadow_dir_hash.json`：**16 位 hex** = `SHA256(abs_path_utf8)` 的前 16 个十六进制字符（与 Python `hexdigest()[:16]` 一致）。
3. 参考移植风格：`crates/hermes-tools/src/backends/file.rs`、`crates/hermes-tools/src/registry.rs`。
4. 依赖：`git2` — 仅在 workspace 未声明时经根 `Cargo.toml` 添加，版本与 workspace 对齐。

Shadow 目录命名（完整实现阶段）：`~/.hermes/shadow/<workspace_hash>/`，其中 hash 算法以 fixture 与 `checkpoint_shadow_dir_id` 为准。

## 分阶段

### Phase A — shadow 目录 ID（当前 registry 覆盖）

1. 逻辑位于 `harness.rs` 的 `checkpoint_shadow_dir_id`（或抽到共享 crate 若多处使用）。
2. 验证：

```bash
cargo test -p hermes-parity-tests checkpoint
```

### Phase B — 完整 checkpoint 工具（见 `PARITY_PLAN.md` Week 1）

1. 在 `crates/hermes-tools/src/backends/checkpoint.rs` 实现 git2 影子仓库操作。
2. 语义：snapshot / rollback / list / diff；turn 前 commit，message `turn-{turn_id}-{timestamp}`。
3. 注册 ToolRegistry，tool name `checkpoint`。
4. 扩展 `fixtures/checkpoint_manager/` 与 `record_fixtures.py`。

验证：

```bash
cargo build -p hermes-tools
cargo test -p hermes-tools checkpoint
cargo test -p hermes-parity-tests checkpoint
cargo clippy -p hermes-tools -- -D warnings
```

**Phase B 可选性能验收**（不阻塞 JSON parity）：10MB 工作区快照 &lt;100ms — 用 criterion 或一次性 benchmark 自证，见 `PARITY_PLAN.md`。

**手动验收**：改文件 → rollback 完整恢复。

## 防御

- golden 中 `demo_workspace_path` 的 `expected` 与 `checkpoint_shadow_dir_id("/workspace/demo")` 必须一致；改算法前先确认 Python 侧是否变更。

## 提交

```
parity(checkpoint_manager): port from python v2026.4.13
```

Phase A 与 Phase B 建议分 PR；同一 PR 仅一种阶段范围。
