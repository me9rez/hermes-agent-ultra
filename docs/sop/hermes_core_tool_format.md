# SOP: `hermes_core_tool_format`

| 字段 | 值 |
|------|-----|
| registry `id` | `hermes_core_tool_format` |
| Python | N/A（Hermes XML tool-call 往返，Rust 为规范实现） |
| Rust | `crates/hermes-core/src/tool_call_parser.rs` — `format_tool_calls`, `parse_tool_calls`, `separate_text_and_calls` |
| Crate | `hermes-core` |
| Fixtures | `crates/hermes-parity-tests/fixtures/hermes_core/*.json` |

## 前置

1. 读 `tool_call_parser.rs` 与 fixture：`format_tool_calls.json`、`tool_call_parser.json`。
2. 读 `harness.rs` 中 `format_tool_calls` / `parse_tool_calls` / `separate_text_and_calls` 分支。
3. 读属性测试：`crates/hermes-core/tests/prop_tool_call_parser.rs`（round-trip 不变量）。

## 实现

1. 修改解析或格式化逻辑时，保持与 golden **字节级**一致（空白、标签顺序以 fixture 为准）。
2. 新增 `op` 或 case：同步 fixture + harness +（若需要）prop test。
3. 勿破坏对外 re-export：`crates/hermes-core/src/lib.rs`。

## 验证（顺序执行，失败即停）

```bash
cargo build -p hermes-core
cargo test -p hermes-parity-tests hermes_core
cargo test -p hermes-core tool_call
cargo clippy -p hermes-core -- -D warnings
```

## 提交

```
parity(hermes_core_tool_format): port from python v2026.4.13
```

说明：无 Python 对照时，commit body 可注明「golden-driven XML round-trip」。
