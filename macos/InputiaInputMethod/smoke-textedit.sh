#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$ROOT_DIR/smoke-common.sh"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
SELECT_LOG="/tmp/inputia-textedit-select.$$.log"
RESTORE_LOG="/tmp/inputia-textedit-restore.$$.log"
OSASCRIPT_FILE="/tmp/inputia-textedit-osascript.$$.applescript"

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

if [[ ! -x "$EXECUTABLE" ]]; then
  echo "textEditSmokeReady=false reason=missing-executable path=$EXECUTABLE"
  exit 1
fi

if [[ -d "$BUILD_APP" && "${INPUTIA_SKIP_CDHASH_CHECK:-0}" != "1" ]]; then
  expected_cdhash="$(cdhash "$BUILD_APP")"
  actual_cdhash="$(cdhash "$APP")"
  echo "expectedCDHash=$expected_cdhash"
  echo "actualCDHash=$actual_cdhash"
  if [[ "$actual_cdhash" != "$expected_cdhash" ]]; then
    echo "textEditSmokeReady=false reason=cdhash-mismatch"
    exit 2
  fi
fi

if [[ ! -f "$APP/Contents/Resources/inputia.pdf" ]]; then
  echo "textEditSmokeReady=false reason=missing-mode-icon"
  exit 3
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "textEditSmokeReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow this script to open TextEdit"
  exit 14
fi
inputia_require_gui_session "textEditSmokeReady" 14

inputia_require_textedit_idle "textEditSmokeReady" 13

cleanup_textedit_smoke() {
  local cleanup_status=0
  inputia_restore_previous_input_source "$TIS_TOOL" "$EXECUTABLE" "$RESTORE_LOG" || cleanup_status=1
  inputia_cleanup_smoke_files "$SELECT_LOG" "$RESTORE_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  inputia_cleanup_textedit_if_started || cleanup_status=1
  return "$cleanup_status"
}
trap cleanup_textedit_smoke EXIT

if [[ "${INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK:-0}" == "1" ]]; then
  /usr/bin/printf 'select-log' >"$SELECT_LOG"
  /usr/bin/printf 'restore-log' >"$RESTORE_LOG"
  /usr/bin/printf 'osascript-log' >"$OSASCRIPT_FILE"
  echo "textEditCleanupSelfCheck=true phase=after-temp-write"
  exit "${INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK_RC:-27}"
fi

inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$SELECT_LOG" \
  "textEditSmokeReady" 15

INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1
/bin/cat >"$OSASCRIPT_FILE" <<'APPLESCRIPT'
set previousBundleId to ""
set textEditWasRunning to false
set preexistingTextEditDocumentCount to 0
set stateClearPasses to 0
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

on restoreFrontmost(previousBundleId)
  if previousBundleId is not "" and previousBundleId is not "com.apple.TextEdit" then
    try
      tell application id previousBundleId to activate
    end try
  end if
end restoreFrontmost

on cleanupDoc(docRef)
  try
    if docRef is not missing value then close docRef saving no
    return true
  on error
    return false
  end try
  return true
end cleanupDoc

on cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId)
  set cleanupSucceeded to true
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
  if textEditWasRunning then
    try
      tell application "TextEdit"
        repeat while (count of documents) > preexistingTextEditDocumentCount
          close document 1 saving no
        end repeat
      end tell
    on error
      set cleanupSucceeded to false
    end try
  end if
  my restoreFrontmost(previousBundleId)
  return cleanupSucceeded
end cleanupTextEdit

on textEditDocumentCount()
  tell application "System Events" to set isRunning to exists application process "TextEdit"
  if not isRunning then return 0
  tell application "TextEdit" to return count of documents
end textEditDocumentCount

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

on typeNI()
  tell application "System Events"
    key code 45
    delay 0.05
    key code 34
    delay 0.2
  end tell
end typeNI

on clearInputiaState()
  tell application "System Events"
    key code 53
    delay 0.15
    key code 53
  end tell
  delay 0.2
end clearInputiaState

on assertDocumentCleared(docRef, labelName)
  global stateClearPasses
  tell application "TextEdit" to set clearedText to text of docRef
  if clearedText is not "" then error labelName & "-state-clear-leaked-text:" & clearedText
  set stateClearPasses to stateClearPasses + 1
end assertDocumentCleared

on runCase(caseName, caseKind)
  global stateClearPasses
  my clearInputiaState()
  set docRef to missing value
  try
    tell application "TextEdit"
      activate
      set docRef to make new document
      set text of docRef to ""
    end tell
    delay 0.8
    my waitForFrontmost("TextEdit")
    my clearInputiaState()
    my assertStillFrontmost("TextEdit")
    my assertDocumentCleared(docRef, caseName)
    my typeNI()
    tell application "System Events"
      if caseKind is "space" then
        key code 49
      else if caseKind is "return" then
        key code 36
      else if caseKind is "keypad-return" then
        key code 76
      else if caseKind is "digit-three" then
        key code 20
      else if caseKind is "page-down" then
        key code 121
        delay 0.2
        key code 49
      else if caseKind is "arrow-down" then
        key code 125
        delay 0.2
        key code 125
        delay 0.2
        key code 49
      end if
    end tell
    delay 0.8
    my assertStillFrontmost("TextEdit")
    tell application "TextEdit" to set resultText to text of docRef
    if not (my cleanupDoc(docRef)) then error "case-cleanup-failed"
    return resultText
  on error errMsg number errNum
    if docRef is not missing value then my cleanupDoc(docRef)
    error caseName & ":" & errMsg number errNum
  end try
end runCase

on runShiftCase()
  global stateClearPasses
  my clearInputiaState()
  set docRef to missing value
  try
    tell application "TextEdit"
      activate
      set docRef to make new document
      set text of docRef to ""
    end tell
    delay 0.8
    my waitForFrontmost("TextEdit")
    my clearInputiaState()
    my assertStillFrontmost("TextEdit")
    my assertDocumentCleared(docRef, "shift-english")
    tell application "System Events"
      key down shift
      delay 0.1
      key up shift
      delay 0.2
    end tell
    my typeNI()
    tell application "System Events" to key code 49
    delay 0.8
    my assertStillFrontmost("TextEdit")
    tell application "TextEdit"
      set englishText to text of docRef
      set text of docRef to ""
      activate
    end tell
    delay 0.8
    my waitForFrontmost("TextEdit")
    my clearInputiaState()
    my assertStillFrontmost("TextEdit")
    my assertDocumentCleared(docRef, "shift-chinese")
    tell application "System Events"
      key down shift
      delay 0.1
      key up shift
      delay 0.2
    end tell
    my typeNI()
    tell application "System Events" to key code 49
    delay 0.8
    my assertStillFrontmost("TextEdit")
    tell application "TextEdit" to set chineseText to text of docRef
    if not (my cleanupDoc(docRef)) then error "shift-cleanup-failed"
    return englishText & linefeed & chineseText
  on error errMsg number errNum
    if docRef is not missing value then my cleanupDoc(docRef)
    error "shift:" & errMsg number errNum
  end try
end runShiftCase

set outputLines to {}
try
  set end of outputLines to "textEditWasRunningBefore=" & textEditWasRunning
  set end of outputLines to "textEditDocsBefore=" & preexistingTextEditDocumentCount
  set end of outputLines to "defaultChineseResult=" & my runCase("default-chinese", "space")
  set end of outputLines to "enterRawResult=" & my runCase("enter-raw-composition", "return")
  set end of outputLines to "keypadEnterRawResult=" & my runCase("keypad-enter-raw-composition", "keypad-return")
  set end of outputLines to "digitCandidateResult=" & my runCase("digit-select-candidate", "digit-three")
  set end of outputLines to "pageCandidateResult=" & my runCase("page-down-commit", "page-down")
  set end of outputLines to "arrowCandidateResult=" & my runCase("arrow-down-commit", "arrow-down")
  set shiftResults to my runShiftCase()
  set oldDelimiters to AppleScript's text item delimiters
  set AppleScript's text item delimiters to linefeed
  set shiftItems to text items of shiftResults
  set AppleScript's text item delimiters to oldDelimiters
	  set end of outputLines to "shiftEnglishResult=" & item 1 of shiftItems
	  set end of outputLines to "shiftChineseResult=" & item 2 of shiftItems
	  set end of outputLines to "textEditStateClearPasses=" & stateClearPasses
	  set cleanupSucceeded to my cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId)
  set end of outputLines to "textEditCleanupSucceeded=" & cleanupSucceeded
  set end of outputLines to "textEditDocsAfter=" & my textEditDocumentCount()
on error errMsg number errNum
  my cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId)
  error errMsg number errNum
end try

set oldDelimiters to AppleScript's text item delimiters
set AppleScript's text item delimiters to linefeed
set outputText to outputLines as text
set AppleScript's text item delimiters to oldDelimiters
return outputText
APPLESCRIPT
results="$(inputia_run_with_timeout textedit-osascript "${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}" /usr/bin/osascript "$OSASCRIPT_FILE")"

printf '%s\n' "$results"

value_for() {
  local name="$1"
  printf '%s\n' "$results" | /usr/bin/sed -n "s/^${name}=//p" | /usr/bin/tail -1
}

default_result="$(value_for defaultChineseResult)"
enter_raw_result="$(value_for enterRawResult)"
keypad_enter_raw_result="$(value_for keypadEnterRawResult)"
digit_candidate_result="$(value_for digitCandidateResult)"
page_candidate_result="$(value_for pageCandidateResult)"
arrow_candidate_result="$(value_for arrowCandidateResult)"
shift_english_result="$(value_for shiftEnglishResult)"
shift_chinese_result="$(value_for shiftChineseResult)"
textedit_docs_before="$(value_for textEditDocsBefore)"
textedit_state_clear_passes="$(value_for textEditStateClearPasses)"
textedit_cleanup_succeeded="$(value_for textEditCleanupSucceeded)"
textedit_docs_after="$(value_for textEditDocsAfter)"

if [[ "$(has_cjk "$default_result")" != "true" ]]; then
  echo "textEditSmokePassed=false step=default-chinese"
  exit 4
fi
if [[ "$enter_raw_result" != "ni" ]]; then
  echo "textEditSmokePassed=false step=enter-raw-composition"
  exit 5
fi
if [[ "$keypad_enter_raw_result" != "ni" ]]; then
  echo "textEditSmokePassed=false step=keypad-enter-raw-composition"
  exit 6
fi
if [[ "$(has_cjk "$digit_candidate_result")" != "true" || "$digit_candidate_result" == "你" ]]; then
  echo "textEditSmokePassed=false step=digit-select-candidate"
  exit 7
fi
if [[ "$(has_cjk "$page_candidate_result")" != "true" || "$page_candidate_result" == "你" ]]; then
  echo "textEditSmokePassed=false step=page-down-commit"
  exit 8
fi
if [[ "$(has_cjk "$arrow_candidate_result")" != "true" || "$arrow_candidate_result" == "你" ]]; then
  echo "textEditSmokePassed=false step=arrow-down-commit"
  exit 9
fi
if [[ "$shift_english_result" != "ni " ]]; then
  echo "textEditSmokePassed=false step=shift-to-english"
  exit 10
fi
if [[ "$(has_cjk "$shift_chinese_result")" != "true" ]]; then
  echo "textEditSmokePassed=false step=shift-back-to-chinese"
  exit 11
fi
if [[ "$textedit_state_clear_passes" != "8" ]]; then
  echo "textEditSmokePassed=false step=state-clear-count expected=8 actual=${textedit_state_clear_passes:-missing}"
  exit 14
fi
if [[ "$textedit_cleanup_succeeded" != "true" ]]; then
  echo "textEditSmokePassed=false step=textedit-cleanup-failed"
  exit 13
fi
if [[ -n "$textedit_docs_before" && -n "$textedit_docs_after" && "$textedit_docs_after" -gt "$textedit_docs_before" ]]; then
  echo "textEditSmokePassed=false step=textedit-cleanup before=$textedit_docs_before after=$textedit_docs_after"
  exit 12
fi

echo "textEditSmokePassed=true"
