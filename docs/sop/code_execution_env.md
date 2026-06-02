# SOP: `code_execution_env`

| 字段 | 值 |
|------|-----|
| registry `id` | `code_execution_env` |
| Python | `tools/code_execution_tool.py::_scrub_child_env` |
| Rust | `crates/hermes-tools/src/code_execution_env.rs` (`scrub_child_env` + `prepare_child_env`) |
| Crate | `hermes-tools` |

## 验证

```bash
cargo build -p hermes-tools
cargo test -p hermes-parity-tests code_execution_env
cargo test -p hermes-tools code_execution_env
```

## 提交

```
parity(code_execution_env): port scrub_child_env from python
```
