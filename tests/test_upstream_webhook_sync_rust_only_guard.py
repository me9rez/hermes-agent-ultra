from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


def _load_module():
    repo_root = Path(__file__).resolve().parents[1]
    script_path = repo_root / "scripts" / "upstream_webhook_sync.py"
    spec = importlib.util.spec_from_file_location("upstream_webhook_sync", script_path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)  # type: ignore[assignment]
    return module


def test_commit_is_python_test_only():
    module = _load_module()
    assert module.commit_is_python_test_only(["tests/cli/test_commands.py"]) is True
    assert module.commit_is_python_test_only(["test/helpers.py"]) is True
    assert (
        module.commit_is_python_test_only(
            ["tests/cli/test_commands.py", "crates/hermes-cli/src/main.rs"]
        )
        is False
    )
