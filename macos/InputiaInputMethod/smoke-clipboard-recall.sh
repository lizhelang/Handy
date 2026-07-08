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
EVENT_LOG_PROVIDED="${INPUTIA_DEBUG_EVENTS+x}"
EVENT_LOG="${INPUTIA_DEBUG_EVENTS:-/tmp/inputia-clipboard-recall-events.$$.log}"
SELECT_LOG="/tmp/inputia-clipboard-recall-select.$$.log"
RESTORE_LOG="/tmp/inputia-clipboard-recall-restore.$$.log"
OSASCRIPT_FILE="/tmp/inputia-clipboard-recall-osascript.$$.applescript"
ORIGINAL_CLIPBOARD=""
CLIPBOARD_CHANGED=0

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

restore_clipboard() {
  if [[ "$CLIPBOARD_CHANGED" == "1" && "${INPUTIA_CLIPBOARD_SMOKE_RESTORE:-1}" == "1" ]]; then
    /usr/bin/printf '%s' "$ORIGINAL_CLIPBOARD" | /usr/bin/pbcopy || true
  fi
}

if [[ ! -x "$EXECUTABLE" ]]; then
  echo "clipboardRecallSmokeReady=false reason=missing-executable path=$EXECUTABLE"
  exit 1
fi

if [[ -d "$BUILD_APP" && "${INPUTIA_SKIP_CDHASH_CHECK:-0}" != "1" ]]; then
  expected_cdhash="$(cdhash "$BUILD_APP")"
  actual_cdhash="$(cdhash "$APP")"
  echo "expectedCDHash=$expected_cdhash"
  echo "actualCDHash=$actual_cdhash"
  if [[ "$actual_cdhash" != "$expected_cdhash" ]]; then
    echo "clipboardRecallSmokeReady=false reason=cdhash-mismatch"
    exit 2
  fi
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "clipboardRecallSmokeReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow this script to open TextEdit"
  exit 7
fi
inputia_require_gui_session "clipboardRecallSmokeReady" 7

inputia_require_textedit_idle "clipboardRecallSmokeReady" 6
if [[ "${INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK:-0}" != "1" &&
  "${INPUTIA_DEBUG_LOG_PREPARE_SELF_CHECK:-0}" != "1" &&
  "${INPUTIA_SKIP_INPUTIA_HOST_PREFLIGHT_FOR_TEST:-0}" != "1" ]]; then
  inputia_require_process_not_running \
    "InputiaInputMethod" "clipboardRecallSmokeReady" 10 \
    "inputia-host-running" "-"
fi
inputia_require_text_clipboard_restorable "clipboardRecallSmokeReady" 9

cleanup_textedit_smoke() {
  inputia_cleanup_textedit_if_started
}
cleanup_smoke() {
  local cleanup_status=0
  inputia_restore_debug_events_env || cleanup_status=1
  restore_clipboard || cleanup_status=1
  inputia_restore_previous_input_source "$TIS_TOOL" "$EXECUTABLE" "$RESTORE_LOG" || cleanup_status=1
  if [[ -n "$EVENT_LOG_PROVIDED" ]]; then
    inputia_cleanup_smoke_files "$SELECT_LOG" "$RESTORE_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  else
    inputia_cleanup_smoke_files "$SELECT_LOG" "$RESTORE_LOG" "$EVENT_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  fi
  cleanup_textedit_smoke || cleanup_status=1
  return "$cleanup_status"
}
inputia_capture_debug_events_env
trap cleanup_smoke EXIT

if [[ "${INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK:-0}" == "1" ]]; then
  ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste 2>/dev/null || true)"
  self_check_phase="after-clipboard-write"
  if inputia_try_write_clipboard_text 'inputia clipboard cleanup self-check'; then
    CLIPBOARD_CHANGED=1
  else
    self_check_phase="pasteboard-unavailable"
  fi
  /usr/bin/printf 'select-log' >"$SELECT_LOG"
  /usr/bin/printf 'restore-log' >"$RESTORE_LOG"
  /usr/bin/printf 'event-log' >"$EVENT_LOG"
  /usr/bin/printf 'osascript-log' >"$OSASCRIPT_FILE"
  if inputia_try_set_debug_events_env "$EVENT_LOG"; then
    self_check_phase="${self_check_phase}+debug-env-write"
  else
    self_check_phase="${self_check_phase}+launchctl-env-unavailable"
  fi
  echo "clipboardRecallCleanupSelfCheck=true phase=$self_check_phase"
  exit "${INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK_RC:-23}"
fi

inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"
inputia_assert_debug_event_log_clean "$EVENT_LOG" "clipboardRecallSmokeReady" 13
inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$SELECT_LOG" \
  "clipboardRecallSmokeReady" 8

inputia_set_debug_events_env_or_exit "$EVENT_LOG" "clipboardRecallSmokeReady" 14
if [[ "${INPUTIA_RESTART_HOST_FOR_DEBUG:-1}" == "1" ]]; then
  /usr/bin/killall InputiaInputMethod >/dev/null 2>&1 || true
  /bin/sleep 0.5
fi

expected="${INPUTIA_CLIPBOARD_SMOKE_TEXT:-Inputia Clipboard Smoke $(date +%s)}"
ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste 2>/dev/null || true)"
/usr/bin/printf '%s' "$expected" | /usr/bin/pbcopy
CLIPBOARD_CHANGED=1
INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1
/bin/cat >"$OSASCRIPT_FILE" <<'APPLESCRIPT'
on restoreFrontmost(previousBundleId)
  if previousBundleId is not "" and previousBundleId is not "com.apple.TextEdit" then
    try
      tell application id previousBundleId to activate
    end try
  end if
end restoreFrontmost

on cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, smokeDocument)
  set cleanupSucceeded to true
  try
    if smokeDocument is not missing value then close smokeDocument saving no
  on error
    set cleanupSucceeded to false
  end try
  if not textEditWasRunning then
    try
      tell application "TextEdit" to quit saving no
    on error
      set cleanupSucceeded to false
    end try
    my restoreFrontmost(previousBundleId)
    return cleanupSucceeded
  end if
  try
    tell application "TextEdit"
      repeat while (count of documents) > preexistingTextEditDocumentCount
        close front document saving no
      end repeat
    end tell
  on error
    set cleanupSucceeded to false
  end try
  my restoreFrontmost(previousBundleId)
  return cleanupSucceeded
end cleanupTextEdit

on waitForFrontmost(appName)
  set deadline to (current date) + 3
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

on waitForClipboardRecallShown(eventLogPath)
  set deadline to (current date) + 3
  repeat
    try
      set logText to read POSIX file eventLogPath
      if logText contains "clipboardRecallShown" then return true
    end try
    if (current date) > deadline then error "clipboard-recall-not-shown"
    delay 0.1
  end repeat
end waitForClipboardRecallShown

on assertNoClipboardRecallBeforeTrigger(eventLogPath)
  try
    set logText to read POSIX file eventLogPath
    if logText contains "clipboardRecallShown" then error "clipboard-recall-shown-before-trigger"
    if logText contains "clipboardRecallCommit" then error "clipboard-recall-commit-before-trigger"
  end try
end assertNoClipboardRecallBeforeTrigger

on assertNoClipboardRecallCommitBeforeSelection(eventLogPath)
  try
    set logText to read POSIX file eventLogPath
    if logText contains "clipboardRecallCommit" then error "clipboard-recall-commit-before-selection"
  end try
end assertNoClipboardRecallCommitBeforeSelection

on resetClipboardRecallEventLog(eventLogPath)
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
end resetClipboardRecallEventLog

on textEditDocumentCount()
  tell application "System Events" to set isRunning to exists application process "TextEdit"
  if not isRunning then return 0
  tell application "TextEdit" to return count of documents
end textEditDocumentCount

on run argv
set eventLogPath to item 1 of argv
set previousBundleId to ""
set textEditWasRunning to false
set preexistingTextEditDocumentCount to 0
set smokeDocument to missing value
tell application "System Events"
  try
    set previousBundleId to bundle identifier of first application process whose frontmost is true
  end try
  set textEditWasRunning to exists application process "TextEdit"
end tell
if textEditWasRunning then
  try
    tell application "TextEdit" to set preexistingTextEditDocumentCount to count of documents
  end try
end if

try
  tell application "TextEdit"
    activate
    set smokeDocument to make new document
    set text of smokeDocument to ""
  end tell
  my waitForFrontmost("TextEdit")
	  my clearInputiaState()
	  my assertStillFrontmost("TextEdit")
	  tell application "TextEdit" to set clearedText to text of smokeDocument
	  if clearedText is not "" then error "state-clear-leaked-text:" & clearedText
	  set eventLogResetAfterStateClear to my resetClipboardRecallEventLog(eventLogPath)
	  if not eventLogResetAfterStateClear then error "clipboard-recall-event-log-reset-failed"
	  my assertNoClipboardRecallBeforeTrigger(eventLogPath)
	  tell application "System Events"
	    key code 9 using {control down, shift down}
    delay 0.35
  end tell
  my assertStillFrontmost("TextEdit")
  my waitForClipboardRecallShown(eventLogPath)
  my assertNoClipboardRecallCommitBeforeSelection(eventLogPath)
  tell application "System Events" to key code 49
  delay 0.8
	  my assertStillFrontmost("TextEdit")
	  tell application "TextEdit" to set resultText to text of smokeDocument
	  set cleanupSucceeded to my cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, smokeDocument)
	  set docsAfterCleanup to my textEditDocumentCount()
	  return resultText & linefeed & "textEditWasRunningBefore=" & textEditWasRunning & linefeed & "textEditDocsBefore=" & preexistingTextEditDocumentCount & linefeed & "clipboardRecallClearedText=" & clearedText & linefeed & "clipboardRecallEventLogResetAfterStateClear=" & eventLogResetAfterStateClear & linefeed & "textEditCleanupSucceeded=" & cleanupSucceeded & linefeed & "textEditDocsAfter=" & docsAfterCleanup
on error errMsg number errNum
  my cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, smokeDocument)
  error errMsg number errNum
end try
end run
APPLESCRIPT
result="$(inputia_run_with_timeout clipboard-recall-osascript "${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}" /usr/bin/osascript "$OSASCRIPT_FILE" "$EVENT_LOG")"
inputia_restore_debug_events_env

value_for() {
  local name="$1"
  printf '%s\n' "$result" | /usr/bin/sed -n "s/^${name}=//p" | /usr/bin/tail -1
}

result_text="$(printf '%s\n' "$result" | /usr/bin/sed '/^textEditWasRunningBefore=/d;/^textEditDocsBefore=/d;/^clipboardRecallClearedText=/d;/^clipboardRecallEventLogResetAfterStateClear=/d;/^textEditCleanupSucceeded=/d;/^textEditDocsAfter=/d')"
textedit_was_running_before="$(value_for textEditWasRunningBefore)"
textedit_docs_before="$(value_for textEditDocsBefore)"
cleared_text="$(value_for clipboardRecallClearedText)"
event_log_reset_after_state_clear="$(value_for clipboardRecallEventLogResetAfterStateClear)"
textedit_cleanup_succeeded="$(value_for textEditCleanupSucceeded)"
textedit_docs_after="$(value_for textEditDocsAfter)"

echo "clipboardRecallExpected=$expected"
echo "clipboardRecallResult=$result_text"
echo "textEditWasRunningBefore=$textedit_was_running_before"
echo "textEditDocsBefore=$textedit_docs_before"
echo "clipboardRecallClearedText=$cleared_text"
echo "clipboardRecallEventLogResetAfterStateClear=$event_log_reset_after_state_clear"
echo "textEditCleanupSucceeded=$textedit_cleanup_succeeded"
echo "textEditDocsAfter=$textedit_docs_after"
if [[ "$result_text" != "$expected" ]]; then
  echo "clipboardRecallSmokePassed=false reason=result-mismatch"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 3
fi
if [[ "$cleared_text" != "" ]]; then
  echo "clipboardRecallSmokePassed=false reason=state-clear-leaked-text"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 11
fi
if [[ "$event_log_reset_after_state_clear" != "true" ]]; then
  echo "clipboardRecallSmokePassed=false reason=event-log-reset-after-state-clear-failed"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 15
fi
if [[ "$textedit_cleanup_succeeded" != "true" ]]; then
  echo "clipboardRecallSmokePassed=false reason=textedit-cleanup-failed"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 12
fi
if [[ -n "$textedit_docs_before" && -n "$textedit_docs_after" && "$textedit_docs_after" -gt "$textedit_docs_before" ]]; then
  echo "clipboardRecallSmokePassed=false reason=textedit-cleanup before=$textedit_docs_before after=$textedit_docs_after"
  [[ -f "$EVENT_LOG" ]] && /usr/bin/tail -n 120 "$EVENT_LOG"
  exit 10
fi

if [[ -f "$EVENT_LOG" ]]; then
  if ! /usr/bin/grep -q 'clipboardRecallShown' "$EVENT_LOG"; then
    echo "clipboardRecallSmokePassed=false reason=missing-shown-event"
    /usr/bin/tail -n 120 "$EVENT_LOG"
    exit 4
  fi
  if ! /usr/bin/grep -q 'clipboardRecallCommit index=0' "$EVENT_LOG"; then
    echo "clipboardRecallSmokePassed=false reason=missing-commit-event"
    /usr/bin/tail -n 120 "$EVENT_LOG"
    exit 5
  fi
  if ! /usr/bin/grep -Fq "clipboardRecallCommit index=0 text=$expected" "$EVENT_LOG"; then
    echo "clipboardRecallSmokePassed=false reason=commit-event-mismatch"
    /usr/bin/tail -n 120 "$EVENT_LOG"
    exit 9
  fi
fi

echo "clipboardRecallSmokePassed=true"
