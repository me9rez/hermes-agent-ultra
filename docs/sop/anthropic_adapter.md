# SOP: `anthropic_adapter`

| 字段 | 值 |
|------|-----|
| registry `id` | `anthropic_adapter` |
| Python | `research/hermes-agent/agent/anthropic_adapter.py` |
| Rust | `crates/hermes-intelligence/src/anthropic_adapter.rs` |
| Crate | `hermes-intelligence` |
| Fixtures | `crates/hermes-parity-tests/fixtures/anthropic_adapter/*.json` |

## 前置

1. 读 Python 源，列出本次改动涉及的 **public 函数**（与 fixture `op` 对应）。
2. 读 Rust 现有实现与 `crates/hermes-parity-tests/src/harness.rs` 中 `dispatch_case` 的 `op` 分支。
3. 读现有 golden：`model_tools.json`、`oauth_betas.json`。

## 实现

1. 在 `anthropic_adapter.rs` 修改逻辑；保持 `snake_case` 与 Python 语义一致。
2. 若新增 `op`：在 `harness.rs` 增加分支 + 新 fixture 文件 + `scripts/record_fixtures.py`（需 Python 仓库）。
3. 错误与日志：遵循根 [`AGENTS.md`](../../AGENTS.md) 编码约定。

## 验证（顺序执行，失败即停）

```bash
cargo build -p hermes-intelligence
cargo test -p hermes-parity-tests anthropic
cargo clippy -p hermes-intelligence -- -D warnings
```

可选单元测试（若 crate 内有针对 adapter 的 tests）：

```bash
cargo test -p hermes-intelligence anthropic
```

## 提交

```
parity(anthropic_adapter): port from python v2026.4.13
```

PR 中不得包含其它 registry 模块的实现改动。
