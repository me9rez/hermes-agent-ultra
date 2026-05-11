#!/usr/bin/env python3
"""Fail when required upstream surface files are missing locally."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import subprocess
from typing import Any


DEFAULT_PREFIXES = [
    "skills",
    "optional-skills",
    "plugins",
    "website",
    "ui-tui",
    "docs",
]


def run_git(repo_root: pathlib.Path, args: list[str], check: bool = True) -> str:
    proc = subprocess.run(
        ["git", *args],
        cwd=str(repo_root),
        text=True,
        capture_output=True,
        check=False,
    )
    if check and proc.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed: {proc.stderr.strip()}")
    return proc.stdout


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", default=".", help="Repository root")
    parser.add_argument("--local-ref", default="HEAD", help="Local ref to validate")
    parser.add_argument(
        "--local-mode",
        choices=["ref", "worktree"],
        default="ref",
        help="Check files against a git ref or current worktree paths",
    )
    parser.add_argument(
        "--upstream-ref",
        default="upstream/main",
        help="Upstream ref to compare against",
    )
    parser.add_argument(
        "--prefix",
        action="append",
        default=[],
        help="Required prefix to validate (repeatable). Defaults to core upstream surfaces.",
    )
    parser.add_argument(
        "--report-path",
        default="",
        help="Optional explicit report output path",
    )
    parser.add_argument("--json", action="store_true", help="Print JSON report")
    return parser.parse_args()


def default_report_path(repo_root: pathlib.Path) -> pathlib.Path:
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%d-%H%M%S")
    return repo_root / ".sync-reports" / f"upstream-surface-coverage-gate-{stamp}.json"


def list_files(repo_root: pathlib.Path, ref: str, prefix: str) -> list[str]:
    output = run_git(repo_root, ["ls-tree", "-r", "--name-only", ref, prefix], check=False)
    if not output:
        return []
    return [line.strip() for line in output.splitlines() if line.strip()]


def ref_has_path(repo_root: pathlib.Path, ref: str, path: str) -> bool:
    proc = subprocess.run(
        ["git", "cat-file", "-e", f"{ref}:{path}"],
        cwd=str(repo_root),
        text=True,
        capture_output=True,
        check=False,
    )
    return proc.returncode == 0


def build_report(
    repo_root: pathlib.Path,
    local_ref: str,
    local_mode: str,
    upstream_ref: str,
    prefixes: list[str],
) -> dict[str, Any]:
    by_prefix: dict[str, dict[str, Any]] = {}
    missing_total: list[str] = []

    for prefix in prefixes:
        upstream_files = list_files(repo_root, upstream_ref, prefix)
        if local_mode == "worktree":
            missing = [path for path in upstream_files if not (repo_root / path).exists()]
        else:
            missing = [path for path in upstream_files if not ref_has_path(repo_root, local_ref, path)]
        present = len(upstream_files) - len(missing)
        coverage = 1.0 if not upstream_files else present / len(upstream_files)
        by_prefix[prefix] = {
            "upstream_file_count": len(upstream_files),
            "present_locally": present,
            "missing_count": len(missing),
            "coverage_ratio": coverage,
            "missing_sample": missing[:50],
        }
        missing_total.extend(missing)

    summary = {
        "prefixes_checked": prefixes,
        "upstream_file_count": sum(v["upstream_file_count"] for v in by_prefix.values()),
        "present_locally": sum(v["present_locally"] for v in by_prefix.values()),
        "missing_total": len(missing_total),
    }

    return {
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "repo_root": str(repo_root),
        "local_ref": local_ref,
        "local_mode": local_mode,
        "upstream_ref": upstream_ref,
        "ok": len(missing_total) == 0,
        "summary": summary,
        "by_prefix": by_prefix,
        "missing_paths": sorted(missing_total),
    }


def main() -> int:
    args = parse_args()
    repo_root = pathlib.Path(args.repo_root).expanduser().resolve()
    if not repo_root.exists():
        raise SystemExit(f"repo root does not exist: {repo_root}")

    prefixes = args.prefix if args.prefix else list(DEFAULT_PREFIXES)
    report = build_report(repo_root, args.local_ref, args.local_mode, args.upstream_ref, prefixes)

    report_path = (
        pathlib.Path(args.report_path).expanduser().resolve()
        if args.report_path
        else default_report_path(repo_root)
    )
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    report["report_path"] = str(report_path)

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        status = "PASSED" if report["ok"] else "FAILED"
        print(
            f"[upstream-surface-coverage] {status} "
            f"(missing={report['summary']['missing_total']} "
            f"checked={report['summary']['upstream_file_count']})"
        )
        print(f"[upstream-surface-coverage] Report: {report_path}")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
