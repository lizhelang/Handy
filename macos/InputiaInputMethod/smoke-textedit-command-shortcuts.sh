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
SELECT_LOG="/tmp/inputia-textedit-command-select.$$.log"
RESTORE_LOG="/tmp/inputia-textedit-command-restore.$$.log"
OSASCRIPT_FILE="/tmp/inputia-textedit-command-osascript.$$.applescript"
ORIGINAL_CLIPBOARD=""
CLIPBOARD_CHANGED=0

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

restore_clipboard() {
  if [[ "$CLIPBOARD_CHANGED" == "1" && "${INPUTIA_TEXTEDIT_COMMAND_RESTORE_CLIPBOARD:-1}" == "1" ]]; then
    /usr/bin/printf '%s' "$ORIGINAL_CLIPBOARD" | /usr/bin/pbcopy || true
  fi
}

if [[ ! -x "$EXECUTABLE" ]]; then
  echo "textEditCommandShortcutSmokeReady=false reason=missing-executable path=$EXECUTABLE"
  exit 1
fi

if [[ -d "$BUILD_APP" && "${INPUTIA_SKIP_CDHASH_CHECK:-0}" != "1" ]]; then
  expected_cdhash="$(cdhash "$BUILD_APP")"
  actual_cdhash="$(cdhash "$APP")"
  echo "expectedCDHash=$expected_cdhash"
  echo "actualCDHash=$actual_cdhash"
  if [[ "$actual_cdhash" != "$expected_cdhash" ]]; then
    echo "textEditCommandShortcutSmokeReady=false reason=cdhash-mismatch"
    exit 2
  fi
fi

if [[ ! -f "$APP/Contents/Resources/inputia.pdf" ]]; then
  echo "textEditCommandShortcutSmokeReady=false reason=missing-mode-icon"
  exit 3
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "textEditCommandShortcutSmokeReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow this script to open TextEdit"
  exit 16
fi
inputia_require_gui_session "textEditCommandShortcutSmokeReady" 16

inputia_require_textedit_idle "textEditCommandShortcutSmokeReady" 13
inputia_require_text_clipboard_restorable "textEditCommandShortcutSmokeReady" 18

cleanup_textedit_command_smoke() {
  local cleanup_status=0
  restore_clipboard || cleanup_status=1
  inputia_restore_previous_input_source "$TIS_TOOL" "$EXECUTABLE" "$RESTORE_LOG" || cleanup_status=1
  inputia_cleanup_smoke_files "$SELECT_LOG" "$RESTORE_LOG" "$OSASCRIPT_FILE" || cleanup_status=1
  inputia_cleanup_textedit_if_started || cleanup_status=1
  return "$cleanup_status"
}
trap cleanup_textedit_command_smoke EXIT

if [[ "${INPUTIA_TEXTEDIT_COMMAND_CLEANUP_SELF_CHECK:-0}" == "1" ]]; then
  ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste 2>/dev/null || true)"
  self_check_phase="after-clipboard-write"
  if inputia_try_write_clipboard_text 'inputia textedit command cleanup self-check'; then
    CLIPBOARD_CHANGED=1
  else
    self_check_phase="pasteboard-unavailable"
  fi
  /usr/bin/printf 'select-log' >"$SELECT_LOG"
  /usr/bin/printf 'restore-log' >"$RESTORE_LOG"
  /usr/bin/printf 'osascript-log' >"$OSASCRIPT_FILE"
  echo "textEditCommandCleanupSelfCheck=true phase=$self_check_phase"
  exit "${INPUTIA_TEXTEDIT_COMMAND_CLEANUP_SELF_CHECK_RC:-25}"
fi

inputia_select_input_source_or_exit \
  "$APP" "$EXECUTABLE" "$TIS_TOOL" "$SELECT_LOG" \
  "textEditCommandShortcutSmokeReady" 17

ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste 2>/dev/null || true)"
/usr/bin/printf '' | /usr/bin/pbcopy
CLIPBOARD_CHANGED=1
INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1

/bin/cat >"$OSASCRIPT_FILE" <<'APPLESCRIPT'
set previousBundleId to ""
set textEditWasRunning to false
set preexistingTextEditDocumentCount to 0
set sourceText to "Inputia Command Shortcut Source"
set docRef to missing value

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

on cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, docRef)
  set cleanupSucceeded to my cleanupDoc(docRef)
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

on clearInputiaState()
  tell application "System Events"
    key code 53
    delay 0.15
    key code 53
  end tell
  delay 0.2
end clearInputiaState

on assertSourceBeforeCopy(docRef, sourceText)
  tell application "TextEdit" to set currentText to text of docRef
  if currentText is not sourceText then error "textedit-command-state-clear-leaked-text:" & currentText
end assertSourceBeforeCopy

try
  tell application "TextEdit"
    activate
    set docRef to make new document
    set text of docRef to sourceText
  end tell
  delay 0.8
  my waitForFrontmost("TextEdit")
  my clearInputiaState()
  my assertStillFrontmost("TextEdit")
  tell application "TextEdit" to set text of docRef to sourceText
  delay 0.2
  my assertSourceBeforeCopy(docRef, sourceText)
  tell application "System Events"
    key code 0 using {command down}
    delay 0.2
    key code 8 using {command down}
    delay 0.2
  end tell
  my assertStillFrontmost("TextEdit")
  set copiedText to the clipboard
  tell application "TextEdit"
    set text of docRef to ""
    activate
  end tell
  delay 0.2
  tell application "System Events"
    key code 9 using {command down}
  end tell
  delay 0.8
  my assertStillFrontmost("TextEdit")
  tell application "TextEdit" to set pastedText to text of docRef
  set cleanupSucceeded to my cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, docRef)
  set docsAfterCleanup to my textEditDocumentCount()
on error errMsg number errNum
  my cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, docRef)
  error errMsg number errNum
end try

return "textEditWasRunningBefore=" & textEditWasRunning & linefeed & "textEditDocsBefore=" & preexistingTextEditDocumentCount & linefeed & "commandSelectAllCopiedText=" & copiedText & linefeed & "commandPasteResult=" & pastedText & linefeed & "textEditCleanupSucceeded=" & cleanupSucceeded & linefeed & "textEditDocsAfter=" & docsAfterCleanup
APPLESCRIPT
results="$(inputia_run_with_timeout textedit-command-osascript "${INPUTIA_SMOKE_OSASCRIPT_TIMEOUT:-90}" /usr/bin/osascript "$OSASCRIPT_FILE")"

printf '%s\n' "$results"

value_for() {
  local name="$1"
  printf '%s\n' "$results" | /usr/bin/sed -n "s/^${name}=//p" | /usr/bin/tail -1
}

copied_text="$(value_for commandSelectAllCopiedText)"
pasted_text="$(value_for commandPasteResult)"
textedit_cleanup_succeeded="$(value_for textEditCleanupSucceeded)"
textedit_docs_before="$(value_for textEditDocsBefore)"
textedit_docs_after="$(value_for textEditDocsAfter)"

if [[ "$copied_text" != "Inputia Command Shortcut Source" ]]; then
  echo "textEditCommandShortcutSmokePassed=false step=command-copy"
  exit 4
fi
if [[ "$pasted_text" != "Inputia Command Shortcut Source" ]]; then
  echo "textEditCommandShortcutSmokePassed=false step=command-paste"
  exit 5
fi
if [[ "$textedit_cleanup_succeeded" != "true" ]]; then
  echo "textEditCommandShortcutSmokePassed=false step=textedit-cleanup-failed"
  exit 14
fi
if [[ -n "$textedit_docs_before" && -n "$textedit_docs_after" && "$textedit_docs_after" -gt "$textedit_docs_before" ]]; then
  echo "textEditCommandShortcutSmokePassed=false step=textedit-cleanup before=$textedit_docs_before after=$textedit_docs_after"
  exit 12
fi

echo "textEditCommandShortcutSmokePassed=true"
