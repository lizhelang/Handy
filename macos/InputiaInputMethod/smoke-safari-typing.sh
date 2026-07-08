#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$ROOT_DIR/smoke-common.sh"
USER_DEFAULT_APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
DEFAULT_APP="/Library/Input Methods/InputiaInputMethod.app"
if [[ -d "$USER_DEFAULT_APP" ]]; then
  DEFAULT_APP="$USER_DEFAULT_APP"
fi
APP="${1:-$DEFAULT_APP}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
TEXT="${INPUTIA_SAFARI_SMOKE_TEXT:-ni}"
EXPECT="${INPUTIA_SAFARI_SMOKE_EXPECT:-cjk}"
TEST_URL_FILE="/tmp/inputia-safari-typing-test.$$.url"
SELECT_LOG="/tmp/inputia-safari-typing-select.$$.log"
RESTORE_LOG="/tmp/inputia-safari-typing-restore.$$.log"
OSASCRIPT_FILE="/tmp/inputia-safari-typing-osascript.$$.applescript"

cleanup_smoke() {
  local cleanup_status=0
  inputia_restore_previous_input_source "$TIS_TOOL" "$EXECUTABLE" "$RESTORE_LOG" || cleanup_status=1
  inputia_cleanup_smoke_files "$TEST_URL_FILE" "$SELECT_LOG" "$RESTORE_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  inputia_cleanup_safari_if_started || cleanup_status=1
  return "$cleanup_status"
}
trap cleanup_smoke EXIT

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

has_cjk() {
  /usr/bin/python3 - "$1" <<'PY'
import sys

text = sys.argv[1]
print("true" if any("\u4e00" <= ch <= "\u9fff" for ch in text) else "false")
PY
}

key_script() {
  /usr/bin/python3 - "$1" <<'PY'
import sys

codes = {
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6,
    "x": 7, "c": 8, "v": 9, "b": 11, "q": 12, "w": 13,
    "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19,
    "3": 20, "4": 21, "6": 22, "5": 23, "9": 25, "7": 26,
    "8": 28, "0": 29, "o": 31, "u": 32, "i": 34, "p": 35,
    "l": 37, "j": 38, "k": 40, "n": 45, "m": 46, " ": 49,
}

for ch in sys.argv[1].lower():
    if ch not in codes:
        raise SystemExit(f"unsupported smoke character: {ch!r}")
    print(f"  key code {codes[ch]}")
    print("  delay 0.06")
print("  key code 49")
PY
}

if [[ ! -x "$EXECUTABLE" ]]; then
  echo "safariTypingSmokeReady=false reason=missing-executable path=$EXECUTABLE"
  exit 1
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "safariTypingSmokeReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow this script to open Safari"
  exit 7
fi
inputia_require_gui_session "safariTypingSmokeReady" 7

if [[ -d "$BUILD_APP" && "${INPUTIA_SKIP_CDHASH_CHECK:-0}" != "1" ]]; then
  expected_cdhash="$(cdhash "$BUILD_APP")"
  actual_cdhash="$(cdhash "$APP")"
  echo "expectedCDHash=$expected_cdhash"
  echo "actualCDHash=$actual_cdhash"
  if [[ "$actual_cdhash" != "$expected_cdhash" ]]; then
    echo "guiSmokeReady=false reason=cdhash-mismatch"
    echo "safariTypingSmokeReady=false reason=cdhash-mismatch"
    exit 2
  fi
fi
inputia_require_safari_idle "safariTypingSmokeReady" 9

if [[ "${INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK:-0}" == "1" ]]; then
  /usr/bin/printf 'url-log' >"$TEST_URL_FILE"
  /usr/bin/printf 'select-log' >"$SELECT_LOG"
  /usr/bin/printf 'restore-log' >"$RESTORE_LOG"
  /usr/bin/printf 'osascript-log' >"$OSASCRIPT_FILE"
  echo "safariTypingCleanupSelfCheck=true phase=after-temp-write"
  exit "${INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK_RC:-28}"
fi

inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$SELECT_LOG" \
  "safariTypingSmokeReady" 8

/usr/bin/python3 - <<'PY' >"$TEST_URL_FILE"
from urllib.parse import quote

html = """<!doctype html>
<meta charset="utf-8">
<title>VALUE:</title>
<style>
body{font:16px -apple-system,BlinkMacSystemFont,sans-serif;margin:40px;background:white;color:#111}
input{display:block;width:720px;max-width:90vw;font:20px -apple-system,BlinkMacSystemFont,sans-serif;padding:12px;margin:16px 0;border:1px solid #999;border-radius:6px}
</style>
<h1>Inputia Safari Typing Smoke</h1>
<input id="q" autofocus placeholder="type here">
<script>
const q = document.getElementById('q');
function sync(){ document.title = 'VALUE:' + q.value; }
q.addEventListener('input', sync);
q.addEventListener('compositionend', sync);
window.onload = () => { q.focus(); sync(); };
</script>"""
print("data:text/html;charset=utf-8," + quote(html))
PY

url="$(cat "$TEST_URL_FILE")"
keys="$(key_script "$TEXT")"
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

on assertEmptyBeforeTyping(smokeWindowId)
  tell application "Safari" to set currentTitle to name of current tab of window id smokeWindowId
  if currentTitle is not "VALUE:" then error "safari-typing-state-clear-leaked-title:" & currentTitle
end assertEmptyBeforeTyping

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
  my assertEmptyBeforeTyping(smokeWindowId)
  tell application "System Events"
$keys
  end tell
  delay 1
	  my assertStillFrontmost("Safari")
	  tell application "Safari" to set resultTitle to name of current tab of window id smokeWindowId
	  set smokeWindowClosed to my closeSmokeWindow(smokeWindowId)
	  my restoreFrontmost(previousBundleId)
	  return "safariTypingStateClearBeforeTyping=true" & linefeed & "safariTypingTitle=" & resultTitle & linefeed & "safariSmokeWindowClosed=" & smokeWindowClosed
on error errMsg number errNum
  my closeSmokeWindow(smokeWindowId)
  my restoreFrontmost(previousBundleId)
  error errMsg number errNum
end try
APPLESCRIPT
results="$(inputia_run_with_timeout safari-typing-osascript "${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}" /usr/bin/osascript "$OSASCRIPT_FILE")"
printf '%s\n' "$results"

value_for() {
  local name="$1"
  printf '%s\n' "$results" | /usr/bin/sed -n "s/^${name}=//p" | /usr/bin/tail -1
}

title="$(value_for safariTypingTitle)"
state_clear_before_typing="$(value_for safariTypingStateClearBeforeTyping)"
result="${title#VALUE:}"
echo "safariTypingResult=$result"
echo "safariTypingExpectation=$EXPECT"

if [[ "$state_clear_before_typing" != "true" ]]; then
  echo "safariTypingSmokePassed=false reason=state-clear-before-typing-missing"
  exit 11
fi

if [[ "$(value_for safariSmokeWindowClosed)" != "true" ]]; then
  echo "safariTypingSmokePassed=false reason=smoke-window-not-closed"
  exit 10
fi

case "$EXPECT" in
  cjk)
    if [[ "$(has_cjk "$result")" != "true" ]]; then
      echo "safariTypingSmokePassed=false reason=no-cjk"
      exit 3
    fi
    ;;
  raw)
    if [[ "$result" != "$TEXT" ]]; then
      echo "safariTypingSmokePassed=false reason=raw-mismatch expected=$TEXT"
      exit 4
    fi
    ;;
  nonempty)
    if [[ -z "$result" ]]; then
      echo "safariTypingSmokePassed=false reason=empty-result"
      exit 5
    fi
    ;;
  *)
    echo "safariTypingSmokePassed=false reason=unknown-expectation value=$EXPECT"
    exit 6
    ;;
esac

echo "safariTypingSmokePassed=true"
