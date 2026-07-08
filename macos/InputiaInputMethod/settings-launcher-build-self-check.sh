#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
BUILD_SETTINGS_APP="$ROOT_DIR/build/Inputia 设置.app"
SETTINGS_INFO="$BUILD_SETTINGS_APP/Contents/Info.plist"
LAUNCHER_SOURCE="$ROOT_DIR/SettingsLauncher/main.swift"

fail() {
  echo "settingsLauncherBuildSelfCheck=false reason=$1"
  exit 1
}

app_cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

plist_value() {
  /usr/libexec/PlistBuddy -c "Print :$2" "$1" 2>/dev/null || true
}

[[ -d "$BUILD_APP" ]] || fail "missing-build-host"
[[ -d "$BUILD_SETTINGS_APP" ]] || fail "missing-build-settings-launcher"
[[ -f "$SETTINGS_INFO" ]] || fail "missing-settings-info-plist"

host_cdhash="$(app_cdhash "$BUILD_APP")"
expected_host_cdhash="$(plist_value "$SETTINGS_INFO" InputiaExpectedHostCDHash)"

echo "settingsLauncherExpectedHostCDHash=${expected_host_cdhash:-unknown}"
echo "settingsLauncherBuildHostCDHash=${host_cdhash:-unknown}"

[[ -n "$host_cdhash" ]] || fail "missing-build-host-cdhash"
[[ "$expected_host_cdhash" == "$host_cdhash" ]] || fail "expected-host-cdhash-mismatch"

/usr/bin/grep -q 'InputiaExpectedHostCDHash' "$LAUNCHER_SOURCE" ||
  fail "launcher-source-missing-expected-host-cdhash"
/usr/bin/grep -q 'bundleCDHash(at:' "$LAUNCHER_SOURCE" ||
  fail "launcher-source-missing-cdhash-check"
/usr/bin/grep -q 'codesign' "$LAUNCHER_SOURCE" ||
  fail "launcher-source-missing-codesign-inspection"

echo "settingsLauncherBuildSelfCheck=true"
