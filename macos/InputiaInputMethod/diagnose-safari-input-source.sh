#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$ROOT_DIR/smoke-common.sh"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
TEST_URL_FILE="/tmp/inputia-safari-input-source-test.$$.url"
HITOOLBOX_PREF_FILE="/tmp/inputia-hitoolbox-preference.$$.txt"
SOURCE_SELECT_LOG="/tmp/inputia-safari-source-select.$$.log"
FOCUSED_SELECT_LOG="/tmp/inputia-safari-focused-select.$$.log"
RESTORE_LOG="/tmp/inputia-safari-diagnose-restore.$$.log"
OSASCRIPT_FILE="/tmp/inputia-safari-diagnose-osascript.$$.applescript"
SAFARI_DIAGNOSE_WINDOW_ID=""
PREVIOUS_BUNDLE_ID=""

section() {
  printf '\n== %s ==\n' "$1"
}

source_value() {
  local key="$1"
  if [[ -x "$TIS_TOOL" ]]; then
    "$TIS_TOOL" --dump-current-input-source
  else
    "$EXECUTABLE" --dump-current-input-source
  fi |
    awk -F= -v key="$key" '$1 == key { print $2; exit }'
}

dump_current_source() {
  if [[ -x "$TIS_TOOL" ]]; then
    "$TIS_TOOL" --dump-current-input-source
  else
    "$EXECUTABLE" --dump-current-input-source
  fi
}

close_safari_diagnose_window() {
  if [[ -n "$SAFARI_DIAGNOSE_WINDOW_ID" ]]; then
    /usr/bin/osascript "$SAFARI_DIAGNOSE_WINDOW_ID" "$PREVIOUS_BUNDLE_ID" <<'APPLESCRIPT' || true
on run argv
  set smokeWindowId to item 1 of argv as integer
  set previousBundleId to ""
  if (count of argv) > 1 then set previousBundleId to item 2 of argv

  set windowClosed to "true"
  try
    tell application "Safari" to close window id smokeWindowId
  on error
    set windowClosed to "false"
  end try
  if previousBundleId is not "" and previousBundleId is not "com.apple.Safari" then
    try
      tell application id previousBundleId to activate
    end try
  end if
  return windowClosed
end run
APPLESCRIPT
  else
    printf 'true\n'
  fi
}

cleanup_diagnosis() {
  local cleanup_status=0
  inputia_restore_previous_input_source "$TIS_TOOL" "$EXECUTABLE" "$RESTORE_LOG" || cleanup_status=1
  inputia_cleanup_smoke_files \
    "$TEST_URL_FILE" \
    "$HITOOLBOX_PREF_FILE" \
    "$SOURCE_SELECT_LOG" \
    "$FOCUSED_SELECT_LOG" \
    "$RESTORE_LOG" \
    "$OSASCRIPT_FILE" || cleanup_status=1
  close_safari_diagnose_window >/dev/null || cleanup_status=1
  inputia_cleanup_safari_if_started || cleanup_status=1
  return "$cleanup_status"
}
trap cleanup_diagnosis EXIT

if [[ ! -x "$EXECUTABLE" ]]; then
  echo "inputiaExecutableFound=false path=$EXECUTABLE" >&2
  exit 1
fi

section "installed app"
echo "path=$APP"
/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Contents/Info.plist"
/usr/bin/codesign -dv --verbose=4 "$APP" 2>&1 | /usr/bin/awk -F= '/^CDHash=/{print "cdhash="$2}'

section "HIToolbox document input source preference"
if /usr/bin/defaults read com.apple.HIToolbox AppleGlobalTextInputProperties >"$HITOOLBOX_PREF_FILE" 2>/dev/null; then
  cat "$HITOOLBOX_PREF_FILE"
  if /usr/bin/grep -q 'TextInputGlobalPropertyPerContextInput = 1' "$HITOOLBOX_PREF_FILE"; then
    echo "perContextInputSource=true"
  elif /usr/bin/grep -q 'TextInputGlobalPropertyPerContextInput = 0' "$HITOOLBOX_PREF_FILE"; then
    echo "perContextInputSource=false"
  else
    echo "perContextInputSource=unknown"
  fi
else
  echo "perContextInputSource=unknown"
fi

section "GUI diagnosis gate"
if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "safariInputSourceDiagnosisReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow this script to open Safari"
  exit 10
fi
inputia_require_gui_session "safariInputSourceDiagnosisReady" 10
inputia_require_safari_idle "safariInputSourceDiagnosisReady" 11

if [[ "${INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK:-0}" == "1" ]]; then
  /usr/bin/printf 'url-log' >"$TEST_URL_FILE"
  /usr/bin/printf 'hitoolbox-log' >"$HITOOLBOX_PREF_FILE"
  /usr/bin/printf 'source-select-log' >"$SOURCE_SELECT_LOG"
  /usr/bin/printf 'focused-select-log' >"$FOCUSED_SELECT_LOG"
  /usr/bin/printf 'restore-log' >"$RESTORE_LOG"
  /usr/bin/printf 'osascript-log' >"$OSASCRIPT_FILE"
  echo "safariDiagnoseCleanupSelfCheck=true phase=after-temp-write"
  exit "${INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK_RC:-29}"
fi

section "select Inputia before opening Safari test page"
inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$SOURCE_SELECT_LOG" \
  "safariInputSourceDiagnosisReady" 12

section "open local Safari input test page"
/usr/bin/python3 - <<'PY' >"$TEST_URL_FILE"
from urllib.parse import quote

html = """<!doctype html>
<meta charset="utf-8">
<title>Inputia Safari Input Source Test</title>
<style>
body{font:16px -apple-system, BlinkMacSystemFont, sans-serif;margin:40px}
input,textarea{display:block;width:720px;max-width:90vw;font:18px -apple-system, BlinkMacSystemFont, sans-serif;padding:10px;margin:12px 0}
</style>
<h1>Inputia Safari Input Source Test</h1>
<input id="q" autofocus placeholder="type here">
<textarea rows="6" placeholder="textarea"></textarea>
<script>window.onload=()=>document.getElementById('q').focus()</script>
"""
print("data:text/html;charset=utf-8," + quote(html))
PY
url="$(cat "$TEST_URL_FILE")"
INPUTIA_SAFARI_CLEANUP_ALLOWED=1
PREVIOUS_BUNDLE_ID="$(/usr/bin/osascript -e 'tell application "System Events" to get bundle identifier of first application process whose frontmost is true' 2>/dev/null || true)"
/bin/cat >"$OSASCRIPT_FILE" <<APPLESCRIPT
tell application "Safari"
  activate
  make new document with properties {URL:"$url"}
  return id of front window
end tell
APPLESCRIPT
SAFARI_DIAGNOSE_WINDOW_ID="$(inputia_run_with_timeout safari-diagnose-osascript "${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}" /usr/bin/osascript "$OSASCRIPT_FILE")"
echo "safariDiagnoseWindowID=$SAFARI_DIAGNOSE_WINDOW_ID"
/bin/sleep 2

front_app="$(/usr/bin/osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null || true)"
echo "frontmost=$front_app"

section "current source after Safari opens focused input"
dump_current_source
current_id="$(source_value id)"
if [[ "$current_id" == com.inputia.inputmethod.Inputia* ]]; then
  echo "safariKeptInputia=true"
else
  echo "safariKeptInputia=false"
  echo "diagnosis=Safari focused input is currently using '$current_id', so bare Shift will not enter Inputia."
fi

section "select Inputia while Safari input is focused"
inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$FOCUSED_SELECT_LOG" \
  "safariInputSourceDiagnosisReady" 13
/usr/bin/tail -n 80 "$FOCUSED_SELECT_LOG"
/bin/sleep 1
dump_current_source
current_id="$(source_value id)"
if [[ "$current_id" == com.inputia.inputmethod.Inputia* ]]; then
  echo "safariAcceptsFocusedInputiaSelection=true"
else
  echo "safariAcceptsFocusedInputiaSelection=false"
  echo "diagnosis=Focused Safari context rejected or immediately replaced Inputia with '$current_id'."
fi

section "notes"
cat <<'EOF'
This script intentionally does not type into Safari. It only checks which Text Input Source Safari keeps after a fresh local data: page focuses a plain input.
If safariKeptInputia=false but selecting again while focused succeeds, the failure is likely macOS per-document/per-context input-source state, not Inputia Shift handling.
EOF

window_closed="$(close_safari_diagnose_window)"
SAFARI_DIAGNOSE_WINDOW_ID=""
echo "safariDiagnoseWindowClosed=$window_closed"
if [[ "$window_closed" != "true" ]]; then
  echo "safariInputSourceDiagnosisPassed=false reason=diagnose-window-not-closed"
  exit 14
fi
