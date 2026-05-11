from __future__ import annotations

import importlib.util
from pathlib import Path


def _load_module():
    repo_root = Path(__file__).resolve().parents[1]
    script_path = repo_root / "scripts" / "generate-upstream-patch-queue.py"
    spec = importlib.util.spec_from_file_location("patch_queue", script_path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)  # type: ignore[assignment]
    return module


def test_python_test_surface_detection():
    module = _load_module()
    assert module.is_python_test_surface("tests/test_cli.py") is True
    assert module.is_python_test_surface("./test/helpers.py") is True
    assert module.is_python_test_surface("crates/hermes-cli/src/main.rs") is False


def test_commit_is_python_test_only():
    module = _load_module()
    assert module.commit_is_python_test_only(["tests/a.py", "tests/b.py"]) is True
    assert module.commit_is_python_test_only(["tests/a.py", "skills/x.md"]) is False
    assert module.commit_is_python_test_only([]) is False
