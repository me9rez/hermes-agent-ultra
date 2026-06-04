#!/usr/bin/env python3
"""Upload release artifacts to ModelScope model repo."""
import argparse
import json
import os
import re
import sys
from datetime import datetime, timezone
from pathlib import Path


ALLOWED_PATTERNS = [
    "hermes-*.tar.gz",
    "hermes-*.zip",
    "checksums.sha256",
    "install.sh",
    "hermes-agent-ultra.rb",
]


def parse_checksums(dist_dir: Path) -> dict[str, str]:
    """Parse checksums.sha256 file into {filename: sha256} mapping."""
    checksums_file = dist_dir / "checksums.sha256"
    result: dict[str, str] = {}
    if checksums_file.exists():
        for line in checksums_file.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line:
                continue
            parts = line.split()
            if len(parts) >= 2:
                sha256_hash = parts[0]
                filename = parts[-1]  # last field is filename (coreutils format)
                result[filename] = sha256_hash
    return result


def derive_channel(version: str) -> str:
    """Derive release channel from version string.

    Known channels: stable (no pre-release), beta, rc, nightly.
    Unknown pre-release suffixes default to 'beta' for safety,
    matching the Rust client's Channel::from_prerelease behavior.
    """
    v_lower = version.lower()
    # Strip leading 'v' and parse pre-release
    version_clean = v_lower.lstrip('v')

    # Priority must match Rust: nightly > beta > rc > (empty=stable) > unknown=beta
    if "nightly" in v_lower:
        return "nightly"
    if "beta" in v_lower:
        return "beta"
    if "rc" in v_lower:
        return "rc"

    # Pure semver with no pre-release suffix → stable
    if re.match(r'^\d+\.\d+\.\d+$', version_clean):
        return "stable"

    # Has pre-release but doesn't match known channels → treat as beta (safe default)
    print(f"WARNING: Unknown pre-release suffix in '{version}', defaulting to 'beta' channel")
    return "beta"


def artifact_to_platform_key(filename: str) -> str | None:
    """Map artifact filename to platform key, or None if not a binary artifact."""
    m = re.match(r"^hermes-(.+?)(?:\.tar\.gz|\.zip)$", filename)
    if m:
        return m.group(1)
    return None


def collect_artifacts(dist_dir: Path) -> list[Path]:
    """Collect release artifacts from dist directory, excluding .sig/.pem/security."""
    artifacts: list[Path] = []
    for pattern in ALLOWED_PATTERNS:
        artifacts.extend(sorted(dist_dir.glob(pattern)))
    # Deduplicate while preserving order
    seen: set[Path] = set()
    unique: list[Path] = []
    for a in artifacts:
        if a not in seen:
            seen.add(a)
            unique.append(a)
    return unique


def build_latest_json(version: str, repo: str, artifacts: list[Path], dist_dir: Path) -> dict:
    """Build the manifest payload (new format with platforms + backward-compat artifacts)."""
    clean_version = version.lstrip("v")
    channel = derive_channel(version)
    pub_date = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

    checksums = parse_checksums(dist_dir)

    base_url = (
        f"https://modelscope.cn/api/v1/models/{repo}/repo"
        f"?Revision=master&FilePath=hermes-agent-ultra/{version}"
    )

    platforms: dict[str, dict] = {}
    artifact_names: list[str] = []

    for a in artifacts:
        artifact_names.append(a.name)
        platform_key = artifact_to_platform_key(a.name)
        if platform_key is not None:
            entry: dict = {
                "url": f"{base_url}/{a.name}",
            }
            if a.name in checksums:
                entry["sha256"] = checksums[a.name]
            entry["size"] = os.path.getsize(a)
            platforms[platform_key] = entry

    return {
        "version": clean_version,
        "channel": channel,
        "pub_date": pub_date,
        "forced": False,
        "notes": "",
        "platforms": platforms,
        "artifacts": artifact_names,
    }


def main():
    parser = argparse.ArgumentParser(description="Upload release to ModelScope")
    parser.add_argument(
        "--repo",
        required=True,
        help="ModelScope model repo (e.g. flowy2025/agent)",
    )
    parser.add_argument(
        "--version",
        required=True,
        help="Release version tag (e.g. v0.1.0)",
    )
    parser.add_argument(
        "--dist-dir",
        required=True,
        help="Directory containing release artifacts",
    )
    args = parser.parse_args()

    token = os.environ.get("MODELSCOPE_TOKEN")
    if not token:
        raise SystemExit("ERROR: MODELSCOPE_TOKEN environment variable not set")

    dist_dir = Path(args.dist_dir)
    if not dist_dir.is_dir():
        raise SystemExit(f"ERROR: dist directory not found: {dist_dir}")

    version: str = args.version
    repo: str = args.repo

    # Collect artifacts
    artifacts = collect_artifacts(dist_dir)
    if not artifacts:
        raise SystemExit(f"ERROR: no release artifacts found in {dist_dir}")

    print(f"Found {len(artifacts)} artifact(s) to upload:")
    for a in artifacts:
        print(f"  - {a.name} ({a.stat().st_size:,} bytes)")

    # Build latest.json
    latest = build_latest_json(version, repo, artifacts, dist_dir)
    latest_path = dist_dir / "latest.json"
    latest_path.write_text(json.dumps(latest, indent=2) + "\n", encoding="utf-8")
    print(f"\nGenerated latest.json: {json.dumps(latest)}")

    # Import ModelScope SDK
    try:
        from modelscope.hub.api import HubApi
    except ImportError:
        raise SystemExit(
            "ERROR: modelscope package not installed. Run: pip install modelscope"
        )

    # Authenticate
    api = HubApi()
    api.login(token)
    print(f"\nAuthenticated to ModelScope, uploading to model repo: {repo}")

    # Upload each artifact
    upload_prefix = f"hermes-agent-ultra/{version}"
    success_count = 0
    fail_count = 0

    # Upload artifacts
    for artifact in artifacts:
        remote_path = f"{upload_prefix}/{artifact.name}"
        try:
            api.upload_file(
                path_or_fileobj=str(artifact),
                path_in_repo=remote_path,
                repo_id=repo,
                repo_type="model",
                commit_message=f"Release {version}: {artifact.name}",
            )
            print(f"  [OK] {artifact.name} -> {remote_path}")
            success_count += 1
        except Exception as e:
            print(f"  [FAIL] {artifact.name}: {e}", file=sys.stderr)
            fail_count += 1

    # Upload latest.json
    remote_latest = "hermes-agent-ultra/latest.json"
    try:
        api.upload_file(
            path_or_fileobj=str(latest_path),
            path_in_repo=remote_latest,
            repo_id=repo,
            repo_type="model",
            commit_message=f"Release {version}: update latest.json",
        )
        print(f"  [OK] latest.json -> {remote_latest}")
        success_count += 1
    except Exception as e:
        print(f"  [FAIL] latest.json: {e}", file=sys.stderr)
        fail_count += 1

    # Upload channels/{channel}.json
    channel = latest["channel"]
    remote_channel = f"hermes-agent-ultra/channels/{channel}.json"
    try:
        api.upload_file(
            path_or_fileobj=str(latest_path),
            path_in_repo=remote_channel,
            repo_id=repo,
            repo_type="model",
            commit_message=f"Release {version}: update channels/{channel}.json",
        )
        print(f"  [OK] latest.json -> {remote_channel}")
        success_count += 1
    except Exception as e:
        print(f"  [FAIL] channels/{channel}.json: {e}", file=sys.stderr)
        fail_count += 1

    # Summary
    print(f"\nUpload complete: {success_count} succeeded, {fail_count} failed")
    if fail_count > 0:
        raise SystemExit(f"ERROR: {fail_count} file(s) failed to upload")


if __name__ == "__main__":
    main()
