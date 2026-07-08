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
host_source_commit="$(plist_value "$BUILD_APP/Contents/Info.plist" InputiaSourceCommit)"
settings_source_commit="$(plist_value "$SETTINGS_INFO" InputiaSourceCommit)"
host_source_branch="$(plist_value "$BUILD_APP/Contents/Info.plist" InputiaSourceBranch)"
settings_source_branch="$(plist_value "$SETTINGS_INFO" InputiaSourceBranch)"
host_source_dirty="$(plist_value "$BUILD_APP/Contents/Info.plist" InputiaSourceDirty)"
settings_source_dirty="$(plist_value "$SETTINGS_INFO" InputiaSourceDirty)"

echo "settingsLauncherExpectedHostCDHash=${expected_host_cdhash:-unknown}"
echo "settingsLauncherBuildHostCDHash=${host_cdhash:-unknown}"
echo "settingsLauncherHostSourceCommit=${host_source_commit:-unknown}"
echo "settingsLauncherSourceCommit=${settings_source_commit:-unknown}"
echo "settingsLauncherHostSourceBranch=${host_source_branch:-unknown}"
echo "settingsLauncherSourceBranch=${settings_source_branch:-unknown}"
echo "settingsLauncherHostSourceDirty=${host_source_dirty:-unknown}"
echo "settingsLauncherSourceDirty=${settings_source_dirty:-unknown}"

[[ -n "$host_cdhash" ]] || fail "missing-build-host-cdhash"
[[ "$expected_host_cdhash" == "$host_cdhash" ]] || fail "expected-host-cdhash-mismatch"
[[ -n "$host_source_commit" ]] || fail "missing-host-source-commit"
[[ "$settings_source_commit" == "$host_source_commit" ]] || fail "settings-source-commit-mismatch"
[[ "$settings_source_branch" == "$host_source_branch" ]] || fail "settings-source-branch-mismatch"
[[ "$settings_source_dirty" == "$host_source_dirty" ]] || fail "settings-source-dirty-mismatch"

/usr/bin/grep -q 'InputiaExpectedHostCDHash' "$LAUNCHER_SOURCE" ||
  fail "launcher-source-missing-expected-host-cdhash"
/usr/bin/grep -q 'bundleCDHash(at:' "$LAUNCHER_SOURCE" ||
  fail "launcher-source-missing-cdhash-check"
/usr/bin/grep -q 'codesign' "$LAUNCHER_SOURCE" ||
  fail "launcher-source-missing-codesign-inspection"

echo "settingsLauncherBuildSelfCheck=true"
