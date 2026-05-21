# SOP: `code_execution_stubs`

| 字段 | 值 |
|------|-----|
| registry `id` | `code_execution_stubs` |
| Python | `tools/code_execution_tool.py::generate_hermes_tools_module` |
| Rust | `crates/hermes-tools/src/code_execution_stubs.rs` |
| Crate | `hermes-tools` |

## 验证

```bash
cargo build -p hermes-tools
cargo test -p hermes-parity-tests code_execution_stubs
cargo test -p hermes-tools matches_python_web_search_only
```

Golden 来自 Python（`C:\Users\15059\hermes-agent`）：

```bash
python3 -c "import sys; sys.path.insert(0,'../hermes-agent'); ..."
```

见 `crates/hermes-parity-tests/fixtures/code_execution_stubs/generate.json`。

## 提交

```
parity(code_execution_stubs): port generate_hermes_tools_module from python
```
