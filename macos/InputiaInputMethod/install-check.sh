#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
SYSTEM_APP="${INPUTIA_SYSTEM_APP_FOR_TEST:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_SETTINGS_APP="$ROOT_DIR/build/Inputia 设置.app"
SYSTEM_SETTINGS_APP="${INPUTIA_SYSTEM_SETTINGS_APP_FOR_TEST:-/Applications/Inputia 设置.app}"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Hans}"
export INPUTIA_VERIFICATION_OWNER_PID="${INPUTIA_VERIFICATION_OWNER_PID:-$$}"

section() {
  printf '\n== %s ==\n' "$1"
}

plist_value() {
  local plist="$1"
  local key="$2"
  if [[ -f "$plist" ]]; then
    /usr/libexec/PlistBuddy -c "Print :$key" "$plist" 2>/dev/null || true
  fi
}

app_version() {
  plist_value "$1/Contents/Info.plist" CFBundleVersion
}

app_cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
}

process_pids() {
  /usr/bin/pgrep -x InputiaInputMethod 2>/dev/null || true
}

section "policy"
echo "validationTier=install-check"
echo "touchesMenuBar=false"
echo "opensGUI=false"
echo "changesSystemInputSource=false"
echo "checksNotarization=false"

section "system host"
build_version="$(app_version "$BUILD_APP")"
system_version="$(app_version "$SYSTEM_APP")"
build_cdhash="$(app_cdhash "$BUILD_APP")"
system_cdhash="$(app_cdhash "$SYSTEM_APP")"
echo "buildApp=$BUILD_APP"
echo "buildVersion=${build_version:-unknown}"
echo "buildCDHash=${build_cdhash:-unknown}"
echo "systemApp=$SYSTEM_APP"
echo "systemExists=$([[ -d "$SYSTEM_APP" ]] && echo true || echo false)"
echo "systemVersion=${system_version:-unknown}"
echo "systemCDHash=${system_cdhash:-unknown}"
if [[ -n "${build_cdhash:-}" && "$system_cdhash" == "$build_cdhash" ]]; then
  system_matches_build=true
else
  system_matches_build=false
fi
echo "systemMatchesBuild=$system_matches_build"

section "settings app"
build_settings_version="$(app_version "$BUILD_SETTINGS_APP")"
system_settings_version="$(app_version "$SYSTEM_SETTINGS_APP")"
echo "buildSettingsVersion=${build_settings_version:-unknown}"
echo "systemSettingsApp=$SYSTEM_SETTINGS_APP"
echo "systemSettingsExists=$([[ -d "$SYSTEM_SETTINGS_APP" ]] && echo true || echo false)"
echo "systemSettingsVersion=${system_settings_version:-unknown}"
if [[ -n "${build_settings_version:-}" && "$system_settings_version" == "$build_settings_version" ]]; then
  settings_matches_build=true
else
  settings_matches_build=false
fi
echo "settingsMatchesBuild=$settings_matches_build"

section "tis"
tis_output="$(INPUTIA_TIS_INCLUDE_MENU_READINESS=0 "$ROOT_DIR/tis-readiness.sh" "$SYSTEM_APP" 2>&1 || true)"
printf '%s\n' "$tis_output" | /usr/bin/awk '
  /^app=|^appExists=|^buildCDHash=|^appCDHash=|^appMatchesBuild=|^expectedTISModeID=|^tis.targetEnabledMatches=|^tis.targetInstalledMatches=|^tis.hansIconMatchesApp=|^tis.hansEnabled=|^tis.hansSelectable=|^tis.hansSelected=|^tis.currentID=|^tis.currentMatchesTarget=|^tis.menuReadiness=|^tis.menuBlockReason=|^tis.readinessBlockReason=|^tisReadiness=/ { print }
'
tis_ready=false
if /usr/bin/grep -q '^tisReadiness=true$' <<<"$tis_output"; then
  tis_ready=true
fi
echo "installCheckTISReady=$tis_ready"

section "running host"
running_matches_build=false
running_found=false
while IFS= read -r pid; do
  [[ -z "$pid" ]] && continue
  running_found=true
  command="$(/bin/ps -p "$pid" -o command= 2>/dev/null || true)"
  echo "runningPID=$pid"
  echo "runningCommand=$command"
  if [[ "$command" == "$SYSTEM_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
    running_version="$(app_version "$SYSTEM_APP")"
    running_cdhash="$(app_cdhash "$SYSTEM_APP")"
    echo "runningApp=$SYSTEM_APP"
    echo "runningVersion=${running_version:-unknown}"
    echo "runningCDHash=${running_cdhash:-unknown}"
    if [[ -n "${build_cdhash:-}" && "$running_cdhash" == "$build_cdhash" ]]; then
      running_matches_build=true
    fi
  fi
done <<<"$(process_pids)"
echo "runningHostFound=$running_found"
echo "runningMatchesBuild=$running_matches_build"

section "result"
if [[ "$system_matches_build" == "true" &&
  "$settings_matches_build" == "true" &&
  "$tis_ready" == "true" &&
  "$running_matches_build" == "true" ]]; then
  echo "installCheckPassed=true"
else
  echo "installCheckPassed=false"
  exit 1
fi
