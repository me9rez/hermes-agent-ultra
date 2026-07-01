#!/bin/sh
# Launch bundled ELF with cross-built glibc (2.38). Direct execution of the .bin
# file uses the board system loader (e.g. Ubuntu 20.04 glibc 2.31) and segfaults.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec "${ROOT}/lib/ld-linux-aarch64.so.1" \
    --library-path "${ROOT}/lib" \
    "${ROOT}/bin/.hermes-agent-ultra.bin" "$@"
