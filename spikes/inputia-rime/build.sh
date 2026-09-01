#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="$ROOT_DIR/build"
LIBRIME_INCLUDE_DIR="${LIBRIME_INCLUDE_DIR:-/tmp/inputia-librime/src}"
SQUIRREL_APP="${SQUIRREL_APP:-/Library/Input Methods/Squirrel.app}"
LIBRIME_DYLIB="${LIBRIME_DYLIB:-$SQUIRREL_APP/Contents/Frameworks/librime.1.dylib}"
FRAMEWORKS_DIR="$(dirname "$LIBRIME_DYLIB")"

if [ ! -f "$LIBRIME_INCLUDE_DIR/rime_api.h" ]; then
  echo "missing rime_api.h: $LIBRIME_INCLUDE_DIR/rime_api.h" >&2
  echo "run: git clone --depth 1 https://github.com/rime/librime.git /tmp/inputia-librime" >&2
  exit 1
fi

if [ ! -f "$LIBRIME_DYLIB" ]; then
  echo "missing librime dylib: $LIBRIME_DYLIB" >&2
  exit 1
fi

mkdir -p "$BUILD_DIR"

/usr/bin/clang++ \
  -std=c++17 \
  -I"$LIBRIME_INCLUDE_DIR" \
  "$ROOT_DIR/rime_probe.cc" \
  "$LIBRIME_DYLIB" \
  -Wl,-rpath,"$FRAMEWORKS_DIR" \
  -o "$BUILD_DIR/rime_probe"

echo "$BUILD_DIR/rime_probe"
