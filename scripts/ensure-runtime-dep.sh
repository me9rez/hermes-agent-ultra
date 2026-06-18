#!/usr/bin/env bash
# Ensure a single Hermes runtime dependency via the Rust installer core.
set -euo pipefail

dep="${1:-}"
if [[ -z "${dep}" ]]; then
  echo "usage: ensure-runtime-dep.sh <ffmpeg|node|browser|ripgrep>" >&2
  exit 1
fi

find_hermes_bin() {
  local name candidate
  if [[ -n "${HERMES_BIN:-}" && -x "${HERMES_BIN}" ]]; then
    echo "${HERMES_BIN}"
    return 0
  fi
  for name in hermes-agent-ultra hermes-ultra hermes; do
    if candidate="$(command -v "${name}" 2>/dev/null)"; then
      echo "${candidate}"
      return 0
    fi
  done
  local script_dir repo_root
  script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  repo_root="$(cd "${script_dir}/.." && pwd)"
  for candidate in \
    "${repo_root}/target/release/hermes-agent-ultra" \
    "${repo_root}/target/debug/hermes-agent-ultra" \
    "${repo_root}/target/release/hermes-ultra" \
    "${repo_root}/target/debug/hermes-ultra"; do
    if [[ -x "${candidate}" ]]; then
      echo "${candidate}"
      return 0
    fi
  done
  return 1
}

bin="$(find_hermes_bin)" || {
  echo "Hermes binary not found on PATH; build or install hermes-agent-ultra first." >&2
  exit 1
}

exec "${bin}" _ensure-dep "${dep}" --quiet
