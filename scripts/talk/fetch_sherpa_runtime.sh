#!/usr/bin/env bash
# Download sherpa-onnx native runtime for hermes-talk.
#
# Usage:
#   ./scripts/talk/fetch_sherpa_runtime.sh [cpu|cuda|directml|macos|auto]
#
# `auto` (default) picks the platform pack used by `make release-talk`:
#   macOS → macos (CoreML static)
#   Windows/Linux x86_64 → cuda
#   else → cpu
#
# Build with:
#   SHERPA_ONNX_PACK=auto make release-talk
# or set SHERPA_ONNX_LIB_DIR if you already extracted libs.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EP="${1:-auto}"
VERSION="1.13.3"
BASE="https://github.com/k2-fsa/sherpa-onnx/releases/download/v${VERSION}"
CACHE="${SHERPA_ONNX_CACHE:-$ROOT/.cross-cache/sherpa-onnx}"
OS="$(uname -s)"
ARCH="$(uname -m)"

resolve_auto() {
  case "$OS:$ARCH" in
    Darwin:*)     echo "macos" ;;
    Linux:x86_64) echo "cuda" ;;
    MINGW*|MSYS*|CYGWIN*) echo "cuda" ;;
    *)            echo "cpu" ;;
  esac
}

if [[ "$EP" == "auto" ]]; then
  EP="$(resolve_auto)"
fi

archive_for() {
  case "$EP" in
    cpu)
      case "$OS:$ARCH" in
        Linux:x86_64)  echo "sherpa-onnx-v${VERSION}-linux-x64-static-lib.tar.bz2" ;;
        Linux:aarch64) echo "sherpa-onnx-v${VERSION}-linux-aarch64-static-lib.tar.bz2" ;;
        Darwin:arm64)  echo "sherpa-onnx-v${VERSION}-osx-arm64-static-lib.tar.bz2" ;;
        Darwin:x86_64) echo "sherpa-onnx-v${VERSION}-osx-x64-static-lib.tar.bz2" ;;
        MINGW*|MSYS*|CYGWIN*) echo "sherpa-onnx-v${VERSION}-win-x64-static-MT-Release-lib.tar.bz2" ;;
        *) echo "unsupported cpu target: $OS $ARCH" >&2; return 1 ;;
      esac
      ;;
    cuda)
      case "$OS:$ARCH" in
        Linux:x86_64)  echo "sherpa-onnx-v${VERSION}-cuda-12.x-cudnn-9.x-linux-x64-gpu.tar.bz2" ;;
        MINGW*|MSYS*|CYGWIN*) echo "sherpa-onnx-v${VERSION}-cuda-12.x-cudnn-9.x-win-x64-cuda.tar.bz2" ;;
        *) echo "CUDA prebuilt unavailable for $OS $ARCH" >&2; return 1 ;;
      esac
      ;;
    macos|coreml)
      case "$OS:$ARCH" in
        Darwin:arm64)  echo "sherpa-onnx-v${VERSION}-osx-arm64-static-lib.tar.bz2" ;;
        Darwin:x86_64) echo "sherpa-onnx-v${VERSION}-osx-x64-static-lib.tar.bz2" ;;
        *) echo "macOS pack requires Darwin" >&2; return 1 ;;
      esac
      ;;
    directml)
      echo "DirectML: no official prebuilt. Build sherpa-onnx with -DSHERPA_ONNX_ENABLE_DIRECTML=ON" >&2
      echo "Then: export SHERPA_ONNX_LIB_DIR=/path/to/lib and SHERPA_ONNX_PACK=directml" >&2
      return 1
      ;;
    *)
      echo "unknown pack: $EP (cpu|cuda|directml|macos|auto)" >&2
      return 1
      ;;
  esac
}

ARCHIVE="$(archive_for)"
DEST="$CACHE/$EP"
STEM="${ARCHIVE%.tar.bz2}"
LIB_DIR="$DEST/$STEM/lib"

if [[ -d "$LIB_DIR" ]]; then
  echo "sherpa-onnx pack=$EP runtime already at $LIB_DIR"
  echo "export SHERPA_ONNX_LIB_DIR=$LIB_DIR"
  echo "export SHERPA_ONNX_PACK=$EP"
  exit 0
fi

mkdir -p "$DEST"
TMP="$DEST/$ARCHIVE"
if [[ ! -f "$TMP" ]]; then
  echo "Downloading $BASE/$ARCHIVE"
  curl -fL "$BASE/$ARCHIVE" -o "$TMP"
fi

tar -xjf "$TMP" -C "$DEST"
if [[ ! -d "$LIB_DIR" ]]; then
  echo "expected lib/ under $DEST/$STEM" >&2
  exit 1
fi

echo "sherpa-onnx pack=$EP runtime ready at $LIB_DIR"
echo "export SHERPA_ONNX_LIB_DIR=$LIB_DIR"
echo "export SHERPA_ONNX_PACK=$EP"
