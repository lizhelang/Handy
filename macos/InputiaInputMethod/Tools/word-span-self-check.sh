#!/bin/bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/inputia-word-span-check.XXXXXX")"
trap 'rm -rf "$CHECK_DIR"' EXIT
swiftc "$ROOT_DIR/Sources/InputiaInputMethod/InputiaManagedMemory.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaWordSpan.swift" \
  "$ROOT_DIR/Tools/InputiaWordSpanSelfCheck.swift" -o "$CHECK_DIR/check"
"$CHECK_DIR/check"
