#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$ROOT_DIR/smoke-common.sh"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
EVENT_LOG_PROVIDED="${INPUTIA_DEBUG_EVENTS+x}"
EVENT_LOG="${INPUTIA_DEBUG_EVENTS:-/tmp/inputia-safari-enter.$$.log}"
TEST_URL_FILE="/tmp/inputia-safari-enter-test.$$.url"
SELECT_LOG="/tmp/inputia-safari-enter-select.$$.log"
RESTORE_LOG="/tmp/inputia-safari-enter-restore.$$.log"
OSASCRIPT_FILE="/tmp/inputia-safari-enter-osascript.$$.applescript"

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

if [[ ! -x "$EXECUTABLE" ]]; then
  echo "safariEnterSmokeReady=false reason=missing-executable path=$EXECUTABLE"
  exit 1
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "safariEnterSmokeReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow this script to open Safari"
  exit 5
fi
inputia_require_gui_session "safariEnterSmokeReady" 5

if [[ -d "$BUILD_APP" && "${INPUTIA_SKIP_CDHASH_CHECK:-0}" != "1" ]]; then
  expected_cdhash="$(cdhash "$BUILD_APP")"
  actual_cdhash="$(cdhash "$APP")"
  echo "expectedCDHash=$expected_cdhash"
  echo "actualCDHash=$actual_cdhash"
  if [[ "$actual_cdhash" != "$expected_cdhash" ]]; then
    echo "guiSmokeReady=false reason=cdhash-mismatch"
    echo "safariEnterSmokeReady=false reason=cdhash-mismatch"
    exit 2
  fi
fi
inputia_require_safari_idle "safariEnterSmokeReady" 7
inputia_require_process_not_running \
  "InputiaInputMethod" "safariEnterSmokeReady" 11 \
  "inputia-host-running" "-"

cleanup_smoke() {
  local cleanup_status=0
  inputia_restore_debug_events_env || cleanup_status=1
  inputia_restore_previous_input_source "$TIS_TOOL" "$EXECUTABLE" "$RESTORE_LOG" || cleanup_status=1
  if [[ -n "$EVENT_LOG_PROVIDED" ]]; then
    inputia_cleanup_smoke_files "$TEST_URL_FILE" "$SELECT_LOG" "$RESTORE_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  else
    inputia_cleanup_smoke_files "$TEST_URL_FILE" "$SELECT_LOG" "$RESTORE_LOG" "$EVENT_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  fi
  inputia_cleanup_safari_if_started || cleanup_status=1
  return "$cleanup_status"
}
inputia_capture_debug_events_env
trap cleanup_smoke EXIT

if [[ "${INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK:-0}" == "1" ]]; then
  /usr/bin/printf 'event-log' >"$EVENT_LOG"
  /usr/bin/printf 'url-log' >"$TEST_URL_FILE"
  /usr/bin/printf 'select-log' >"$SELECT_LOG"
  /usr/bin/printf 'restore-log' >"$RESTORE_LOG"
  /usr/bin/printf 'osascript-log' >"$OSASCRIPT_FILE"
  self_check_phase="after-temp-write"
  if inputia_try_set_debug_events_env "$EVENT_LOG"; then
    self_check_phase="${self_check_phase}+debug-env-write"
  else
    self_check_phase="${self_check_phase}+launchctl-env-unavailable"
  fi
  echo "safariEnterCleanupSelfCheck=true phase=$self_check_phase"
  exit "${INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK_RC:-26}"
fi

inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"
inputia_assert_debug_event_log_clean "$EVENT_LOG" "safariEnterSmokeReady" 10
inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$SELECT_LOG" \
  "safariEnterSmokeReady" 6

inputia_set_debug_events_env_or_exit "$EVENT_LOG" "safariEnterSmokeReady" 12
if [[ "${INPUTIA_RESTART_HOST_FOR_DEBUG:-1}" == "1" ]]; then
  /usr/bin/killall InputiaInputMethod >/dev/null 2>&1 || true
  /bin/sleep 0.5
fi

/usr/bin/python3 - <<'PY' >"$TEST_URL_FILE"
from urllib.parse import quote

html = """<!doctype html>
<meta charset="utf-8">
<title>READY:</title>
<form id="f"><input id="q" autofocus autocomplete="off"></form>
<script>
const q = document.getElementById('q');
const f = document.getElementById('f');
function sync(prefix='VALUE:'){ document.title = prefix + q.value; }
q.addEventListener('input', () => sync());
q.addEventListener('compositionend', () => sync());
f.addEventListener('submit', (event) => { event.preventDefault(); sync('SUBMITTED:'); });
window.onload = () => { q.focus(); sync(); };
</script>"""
print("data:text/html;charset=utf-8," + quote(html))
PY

url="$(cat "$TEST_URL_FILE")"
INPUTIA_SAFARI_CLEANUP_ALLOWED=1
/bin/cat >"$OSASCRIPT_FILE" <<APPLESCRIPT
set testURL to "$url"
set eventLogPath to "$EVENT_LOG"
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

on assertNoRawCommitBeforeTyping(eventLogPath)
  try
    set logText to read POSIX file eventLogPath
    if logText contains "commit=abc" then error "safari-enter-raw-commit-before-typing"
  end try
end assertNoRawCommitBeforeTyping

on resetSafariEnterEventLog(eventLogPath)
  try
    set eventLogFile to open for access POSIX file eventLogPath with write permission
    set eof eventLogFile to 0
    close access eventLogFile
    return true
  on error
    try
      close access eventLogFile
    end try
    return false
  end try
end resetSafariEnterEventLog

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
  set eventLogResetAfterStateClear to my resetSafariEnterEventLog(eventLogPath)
  if not eventLogResetAfterStateClear then error "safari-enter-event-log-reset-failed"
  my assertNoRawCommitBeforeTyping(eventLogPath)
  tell application "System Events"
    key code 0
    delay 0.05
    key code 11
    delay 0.05
    key code 8
    delay 0.05
    key code 36
  end tell
  delay 0.8
  my assertStillFrontmost("Safari")
  tell application "Safari" to set resultTitle to name of current tab of window id smokeWindowId
  set smokeWindowClosed to my closeSmokeWindow(smokeWindowId)
  my restoreFrontmost(previousBundleId)
  return "safariEnterTitle=" & resultTitle & linefeed & "safariEnterEventLogResetAfterStateClear=" & eventLogResetAfterStateClear & linefeed & "safariSmokeWindowClosed=" & smokeWindowClosed
on error errMsg number errNum
  my closeSmokeWindow(smokeWindowId)
  my restoreFrontmost(previousBundleId)
  error errMsg number errNum
end try
APPLESCRIPT
results="$(inputia_run_with_timeout safari-enter-osascript "${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}" /usr/bin/osascript "$OSASCRIPT_FILE")"
inputia_restore_debug_events_env

printf '%s\n' "$results"
value_for() {
  local name="$1"
  printf '%s\n' "$results" | /usr/bin/sed -n "s/^${name}=//p" | /usr/bin/tail -1
}

title="$(value_for safariEnterTitle)"
event_log_reset_after_state_clear="$(value_for safariEnterEventLogResetAfterStateClear)"
echo "safariEnterExpected=SUBMITTED:abc"
if [[ "$title" != "SUBMITTED:abc" ]]; then
  echo "safariEnterSmokePassed=false reason=not-submitted"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 3
fi
if [[ "$(value_for safariSmokeWindowClosed)" != "true" ]]; then
  echo "safariEnterSmokePassed=false reason=smoke-window-not-closed"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 5
fi
if [[ "$event_log_reset_after_state_clear" != "true" ]]; then
  echo "safariEnterSmokePassed=false reason=event-log-reset-after-state-clear-failed"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 12
fi

if [[ -f "$EVENT_LOG" ]]; then
  if ! /usr/bin/grep -q 'commit=abc' "$EVENT_LOG"; then
    echo "safariEnterSmokePassed=false reason=missing-raw-commit-event"
    /usr/bin/tail -n 120 "$EVENT_LOG"
    exit 4
  fi
fi

echo "safariEnterSmokePassed=true"
