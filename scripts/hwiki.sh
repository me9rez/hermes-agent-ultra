#!/usr/bin/env bash
# hwiki — LLM Wiki CLI wrapper for Hermes Agent
#
# Usage:  hwiki <command> [args]
#
# Commands: init, lint, search, hash, stats, watch
#
# If called as `hermes wiki`, delegates to the hwiki binary.
# Respects $WIKI_PATH and $HERMES_WIKI_PATH.

set -euo pipefail

# Find the hwiki binary relative to this script or in PATH
HWIKI=""
if command -v hwiki &>/dev/null; then
    HWIKI="hwiki"
elif [ -f "$(dirname "$0")/../target/release/hwiki" ]; then
    HWIKI="$(dirname "$0")/../target/release/hwiki"
elif [ -f "$(dirname "$0")/../target/debug/hwiki" ]; then
    HWIKI="$(dirname "$0")/../target/debug/hwiki"
else
    echo "Error: hwiki binary not found. Build it with: cargo build -p hermes-wiki"
    exit 1
fi

exec "$HWIKI" "$@"
