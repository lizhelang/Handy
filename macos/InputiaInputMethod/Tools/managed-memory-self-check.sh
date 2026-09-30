#!/bin/bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPO_DIR="$(cd "$ROOT_DIR/../.." && pwd)"
CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/inputia-managed-memory-check.XXXXXX")"
trap 'rm -rf "$CHECK_DIR"' EXIT
swiftc "$ROOT_DIR/Sources/InputiaInputMethod/InputiaManagedMemory.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPersonalization.swift" \
  "$ROOT_DIR/Tools/InputiaManagedMemorySelfCheck.swift" -o "$CHECK_DIR/check"
"$CHECK_DIR/check" "$REPO_DIR/crates/inputia-handy-runtime/tests/fixtures/memory-query-digest-v1.json"
swiftc "$ROOT_DIR/Sources/InputiaInputMethod/InputiaManagedMemory.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaMemoryImport.swift" \
  "$ROOT_DIR/Tools/InputiaMemoryImportSelfCheck.swift" -o "$CHECK_DIR/import-check"
"$CHECK_DIR/import-check"
