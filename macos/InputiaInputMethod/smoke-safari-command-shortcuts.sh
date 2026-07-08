#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$ROOT_DIR/smoke-common.sh"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
TEST_URL_FILE="/tmp/inputia-safari-command-test.$$.url"
SELECT_LOG="/tmp/inputia-safari-command-select.$$.log"
RESTORE_LOG="/tmp/inputia-safari-command-restore.$$.log"
OSASCRIPT_FILE="/tmp/inputia-safari-command-osascript.$$.applescript"
ORIGINAL_CLIPBOARD=""
CLIPBOARD_CHANGED=0

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

restore_clipboard() {
  if [[ "$CLIPBOARD_CHANGED" == "1" && "${INPUTIA_SAFARI_COMMAND_RESTORE_CLIPBOARD:-1}" == "1" ]]; then
    /usr/bin/printf '%s' "$ORIGINAL_CLIPBOARD" | /usr/bin/pbcopy || true
  fi
}

if [[ ! -x "$EXECUTABLE" ]]; then
  echo "safariCommandShortcutSmokeReady=false reason=missing-executable path=$EXECUTABLE"
  exit 1
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "safariCommandShortcutSmokeReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow this script to open Safari"
  exit 12
fi
inputia_require_gui_session "safariCommandShortcutSmokeReady" 12

if [[ -d "$BUILD_APP" && "${INPUTIA_SKIP_CDHASH_CHECK:-0}" != "1" ]]; then
  expected_cdhash="$(cdhash "$BUILD_APP")"
  actual_cdhash="$(cdhash "$APP")"
  echo "expectedCDHash=$expected_cdhash"
  echo "actualCDHash=$actual_cdhash"
  if [[ "$actual_cdhash" != "$expected_cdhash" ]]; then
    echo "guiSmokeReady=false reason=cdhash-mismatch"
    echo "safariCommandShortcutSmokeReady=false reason=cdhash-mismatch"
    exit 2
  fi
fi
inputia_require_safari_idle "safariCommandShortcutSmokeReady" 13
inputia_require_text_clipboard_restorable "safariCommandShortcutSmokeReady" 15

cleanup_smoke() {
  local cleanup_status=0
  restore_clipboard || cleanup_status=1
  inputia_restore_previous_input_source "$TIS_TOOL" "$EXECUTABLE" "$RESTORE_LOG" || cleanup_status=1
  inputia_cleanup_smoke_files "$TEST_URL_FILE" "$SELECT_LOG" "$RESTORE_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  inputia_cleanup_safari_if_started || cleanup_status=1
  return "$cleanup_status"
}
trap cleanup_smoke EXIT

if [[ "${INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK:-0}" == "1" ]]; then
  ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste 2>/dev/null || true)"
  /usr/bin/printf 'inputia safari command cleanup self-check' | /usr/bin/pbcopy
  CLIPBOARD_CHANGED=1
  /usr/bin/printf 'url-log' >"$TEST_URL_FILE"
  /usr/bin/printf 'select-log' >"$SELECT_LOG"
  /usr/bin/printf 'restore-log' >"$RESTORE_LOG"
  /usr/bin/printf 'osascript-log' >"$OSASCRIPT_FILE"
  echo "safariCommandCleanupSelfCheck=true phase=after-clipboard-write"
  exit "${INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK_RC:-24}"
fi

inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$SELECT_LOG" \
  "safariCommandShortcutSmokeReady" 14

/usr/bin/python3 - <<'PY' >"$TEST_URL_FILE"
from urllib.parse import quote

html = """<!doctype html>
<meta charset="utf-8">
<title>VALUE:</title>
<style>
body{font:16px -apple-system,BlinkMacSystemFont,sans-serif;margin:40px;background:white;color:#111}
input{display:block;width:720px;max-width:90vw;font:20px -apple-system,BlinkMacSystemFont,sans-serif;padding:12px;margin:16px 0;border:1px solid #999;border-radius:6px}
</style>
<h1>Inputia Safari Command Shortcut Smoke</h1>
<input id="q" autofocus autocomplete="off" value="Inputia Safari Command Source">
<script>
const q = document.getElementById('q');
function sync(){ document.title = 'VALUE:' + q.value; }
q.addEventListener('input', sync);
window.onload = () => { q.focus(); q.select(); sync(); };
</script>"""
print("data:text/html;charset=utf-8," + quote(html))
PY

url="$(cat "$TEST_URL_FILE")"
ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste 2>/dev/null || true)"
/usr/bin/printf '' | /usr/bin/pbcopy
CLIPBOARD_CHANGED=1
INPUTIA_SAFARI_CLEANUP_ALLOWED=1
/bin/cat >"$OSASCRIPT_FILE" <<APPLESCRIPT
set testURL to "$url"
set previousBundleId to ""
set smokeWindowId to missing value

tell application "System Events"
  try
    set previousBundleId to bundle identifier of first application process whose frontmost is true
  end try
end tell

on restoreFrontmost(previousBundleId)
  if previousBundleId is not "" and previousBundleId is not "com.apple.Safari" then
    try
      tell application id previousBundleId to activate
    end try
  end if
end restoreFrontmost

on closeSmokeWindow(smokeWindowId)
  if smokeWindowId is not missing value then
    try
      tell application "Safari" to close window id smokeWindowId
      return "true"
    on error
      return "false"
    end try
  end if
  return "true"
end closeSmokeWindow

on waitForFrontmost(appName)
  set deadline to (current date) + 4
  repeat
    tell application "System Events" to set frontApp to name of first application process whose frontmost is true
    if frontApp is appName then return true
    if (current date) > deadline then error "focus-not-frontmost:" & frontApp
    delay 0.1
  end repeat
end waitForFrontmost

on assertStillFrontmost(appName)
  tell application "System Events" to set frontApp to name of first application process whose frontmost is true
  if frontApp is not appName then error "focus-lost:" & frontApp
end assertStillFrontmost

on clearInputiaState()
  tell application "System Events"
    key code 53
    delay 0.15
    key code 53
  end tell
  delay 0.2
end clearInputiaState

on assertCommandSourceBeforeCopy(smokeWindowId)
  tell application "Safari" to set currentTitle to name of current tab of window id smokeWindowId
  if currentTitle is not "VALUE:Inputia Safari Command Source" then error "safari-command-state-clear-leaked-title:" & currentTitle
end assertCommandSourceBeforeCopy

try
  tell application "Safari"
    activate
    make new document with properties {URL:testURL}
    set smokeWindowId to id of front window
  end tell
  delay 2
  my waitForFrontmost("Safari")
  my clearInputiaState()
  my assertStillFrontmost("Safari")
  my assertCommandSourceBeforeCopy(smokeWindowId)
  tell application "System Events"
    key code 0 using {command down}
    delay 0.2
    key code 8 using {command down}
    delay 0.2
  end tell
  set copiedText to the clipboard
  tell application "System Events"
    key code 51
    delay 0.2
    key code 9 using {command down}
    delay 0.8
  end tell
  my assertStillFrontmost("Safari")
	  tell application "Safari" to set resultTitle to name of current tab of window id smokeWindowId
	  set smokeWindowClosed to my closeSmokeWindow(smokeWindowId)
	  my restoreFrontmost(previousBundleId)
	  return "safariCommandStateClearBeforeCopy=true" & linefeed & "commandSelectAllCopiedText=" & copiedText & linefeed & "commandPasteTitle=" & resultTitle & linefeed & "safariSmokeWindowClosed=" & smokeWindowClosed
on error errMsg number errNum
  my closeSmokeWindow(smokeWindowId)
  my restoreFrontmost(previousBundleId)
  error errMsg number errNum
end try
APPLESCRIPT
results="$(inputia_run_with_timeout safari-command-osascript "${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}" /usr/bin/osascript "$OSASCRIPT_FILE")"

printf '%s\n' "$results"

value_for() {
  local name="$1"
  printf '%s\n' "$results" | /usr/bin/sed -n "s/^${name}=//p" | /usr/bin/tail -1
}

copied_text="$(value_for commandSelectAllCopiedText)"
state_clear_before_copy="$(value_for safariCommandStateClearBeforeCopy)"
title="$(value_for commandPasteTitle)"
result="${title#VALUE:}"
smoke_window_closed="$(value_for safariSmokeWindowClosed)"

if [[ "$state_clear_before_copy" != "true" ]]; then
  echo "safariCommandShortcutSmokePassed=false step=state-clear-before-copy"
  exit 6
fi
if [[ "$copied_text" != "Inputia Safari Command Source" ]]; then
  echo "safariCommandShortcutSmokePassed=false step=command-copy"
  exit 3
fi
if [[ "$result" != "Inputia Safari Command Source" ]]; then
  echo "safariCommandShortcutSmokePassed=false step=command-paste"
  exit 4
fi
if [[ "$smoke_window_closed" != "true" ]]; then
  echo "safariCommandShortcutSmokePassed=false step=smoke-window-close"
  exit 5
fi

echo "safariCommandShortcutSmokePassed=true"
