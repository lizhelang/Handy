#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$ROOT_DIR/../.." && pwd)"
RUST_TOOLCHAIN="${INPUTIA_RUST_TOOLCHAIN:-1.96.0}"
HOST_APP="$ROOT_DIR/build/InputiaInputMethod.app"
HOST_EXEC="$HOST_APP/Contents/MacOS/InputiaInputMethod"
RIME_USER_DATA_DIR=""
export INPUTIA_VERIFICATION_OWNER_PID="${INPUTIA_VERIFICATION_OWNER_PID:-$$}"

section() {
  printf '\n== %s ==\n' "$1"
}

run_rust() {
  if command -v rustup >/dev/null 2>&1; then
    rustup run "$RUST_TOOLCHAIN" "$@"
  else
    "$@"
  fi
}

cleanup() {
  if [[ -n "$RIME_USER_DATA_DIR" ]]; then
    /bin/rm -rf "$RIME_USER_DATA_DIR" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

section "policy"
echo "validationTier=dev-fast"
echo "touchesMenuBar=false"
echo "opensGUI=false"
echo "changesSystemInputSource=false"
echo "checksNotarization=false"
echo "rustToolchain=$RUST_TOOLCHAIN"

section "build artifact lock self-check"
"$ROOT_DIR/build-artifact-lock-self-check.sh"

section "validation policy self-check"
"$ROOT_DIR/validation-policy-self-check.sh"

section "install check self-check"
INPUTIA_INSTALL_CHECK_SELF_CHECK=1 "$ROOT_DIR/install-check.sh"

section "build"
run_rust "$ROOT_DIR/build.sh"

section "settings launcher build self-check"
"$ROOT_DIR/settings-launcher-build-self-check.sh"

section "schema catalog self-check"
"$ROOT_DIR/schema-catalog-self-check.sh"

section "settings contract self-check"
"$ROOT_DIR/settings-contract-self-check.sh"

section "rust tests"
run_rust cargo test --manifest-path "$REPO_ROOT/crates/inputia-core/Cargo.toml"
run_rust cargo test --manifest-path "$REPO_ROOT/crates/inputia-rime/Cargo.toml" --test core_flow
run_rust cargo test --manifest-path "$REPO_ROOT/crates/inputia-rime/Cargo.toml" --test schema_smoke
run_rust cargo test --manifest-path "$REPO_ROOT/crates/inputia-capi/Cargo.toml"
run_rust cargo test --manifest-path "$REPO_ROOT/crates/inputia-settings/Cargo.toml"

section "swift and bridge self-check"
"$HOST_EXEC" --self-check
"$HOST_EXEC" --bridge-self-check
"$HOST_EXEC" --bridge-memory-self-check
"$HOST_EXEC" --bridge-settings-self-check
"$HOST_EXEC" --bridge-default-chinese-self-check
"$HOST_EXEC" --bridge-direct-session-self-check
"$HOST_EXEC" --host-shortcut-self-check
"$ROOT_DIR/build/inputia-shortcut-self-check"
"$ROOT_DIR/build/inputia-input-text-router-self-check"
"$ROOT_DIR/build/inputia-host-text-policy-self-check"
"$ROOT_DIR/build/inputia-candidate-panel-layout-self-check"
"$ROOT_DIR/build/inputia-settings-window-self-check"
"$ROOT_DIR/build/inputia-bridge-privacy-self-check"
"$ROOT_DIR/build/inputia-handy-memory-sync-self-check"
"$ROOT_DIR/build/inputia-voice-input-launcher-self-check"

section "rime probe"
RIME_USER_DATA_DIR="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-dev-fast-rime.XXXXXX")"
INPUTIA_RIME_SHARED_DATA_DIR="$HOST_APP/Contents/Resources/RimeData" \
  INPUTIA_RIME_USER_DATA_DIR="$RIME_USER_DATA_DIR" \
  run_rust cargo run --manifest-path "$REPO_ROOT/crates/inputia-rime/Cargo.toml" \
    --example rime_probe -- double_pinyin nillem

section "rime latency self-check"
"$ROOT_DIR/rime-latency-self-check.sh"

section "result"
echo "devFastPassed=true"
