#!/bin/bash

inputia_require_gui_session() {
  local ready_var="$1"
  local exit_code="$2"

  if [[ "${INPUTIA_SKIP_GUI_SESSION_CHECK:-0}" == "1" ]]; then
    echo "guiSessionCheck=skipped"
    return 0
  fi

  local console_user
  console_user="$(/usr/bin/stat -f '%Su' /dev/console 2>/dev/null || true)"
  echo "guiConsoleUser=${console_user:-unknown}"
  if [[ -z "$console_user" || "$console_user" == "root" || "$console_user" == "_mbsetupuser" ]]; then
    echo "guiSmokeReady=false reason=no-console-user"
    echo "$ready_var=false reason=no-console-user"
    exit "$exit_code"
  fi
  local console_uid
  console_uid="$(/usr/bin/stat -f '%u' /dev/console 2>/dev/null || true)"
  echo "guiConsoleUID=${console_uid:-unknown}"
  if [[ -z "$console_uid" ]] || ! /bin/launchctl print "gui/$console_uid" >/dev/null 2>&1; then
    echo "guiSmokeReady=false reason=gui-bootstrap-unavailable"
    echo "$ready_var=false reason=gui-bootstrap-unavailable"
    exit "$exit_code"
  fi

  local session_state
  session_state="$(/usr/sbin/ioreg -n Root -d1 2>/dev/null || true)"
  if [[ "$session_state" != *"kCGSessionLoginDoneKey\"=Yes"* ]]; then
    echo "guiSmokeReady=false reason=login-not-complete"
    echo "$ready_var=false reason=login-not-complete"
    exit "$exit_code"
  fi
  if [[ "$session_state" == *"CGSSessionScreenIsLocked\"=Yes"* ||
    "$session_state" == *"kCGSSessionScreenIsLocked\"=Yes"* ]]; then
    echo "guiSmokeReady=false reason=screen-locked"
    echo "$ready_var=false reason=screen-locked"
    exit "$exit_code"
  fi

  local frontmost_app
  if ! frontmost_app="$(/usr/bin/osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null)"; then
    echo "guiSmokeReady=false reason=frontmost-unavailable"
    echo "$ready_var=false reason=frontmost-unavailable"
    exit "$exit_code"
  fi
  echo "guiFrontmostApp=$frontmost_app"
  if [[ "$frontmost_app" == "loginwindow" ]]; then
    echo "guiSmokeReady=false reason=loginwindow-frontmost"
    echo "$ready_var=false reason=loginwindow-frontmost"
    exit "$exit_code"
  fi
}

inputia_process_match_by_ps() {
  local process_name="$1"
  /bin/ps -axo pid=,comm=,command= 2>/dev/null |
    /usr/bin/awk -v process_name="$process_name" '
      {
        command = (NF >= 3) ? substr($0, index($0, $3)) : ""
        launcher = command ~ "^/bin/(zsh|bash|sh)( |$)"
        launcher = launcher || command ~ "^/usr/bin/(sudo|awk|grep|sed)( |$)"
        matched = $2 == process_name
        matched = matched || $3 == process_name
        matched = matched || $3 ~ ("/" process_name "$")
        matched = matched || command ~ ("^" process_name "([ ]|$)")
        matched = matched || command ~ ("/" process_name "([ ]|$)")
        matched = matched || command ~ (process_name "\\.app/Contents/MacOS/" process_name "([ ]|$)")
        if (!launcher && matched) {
          found = 1
        }
      }
      END { exit found ? 0 : 1 }
    '
}

inputia_process_running() {
  local process_name="$1"
  local process_check_output process_check_rc
  INPUTIA_LAST_PROCESS_CHECK_OUTPUT=""
  set +e
  process_check_output="$(/usr/bin/pgrep -x "$process_name" 2>&1 >/dev/null)"
  process_check_rc=$?
  set -e
  if [[ "$process_check_rc" -eq 0 ]]; then
    return 0
  fi
  if inputia_process_match_by_ps "$process_name"; then
    return 0
  fi
  if /bin/ps -axo pid=,comm=,command= >/dev/null 2>&1; then
    return 1
  fi
  if [[ -n "$process_check_output" ]]; then
    INPUTIA_LAST_PROCESS_CHECK_OUTPUT="$process_check_output"
    return 2
  fi
  return 1
}

inputia_require_process_not_running() {
  local process_name="$1"
  local ready_var="$2"
  local exit_code="$3"
  local reason="$4"
  local allow_var="$5"
  local allow_value="0"
  if [[ -n "$allow_var" && "$allow_var" != "-" ]]; then
    allow_value="${!allow_var:-0}"
  fi

  local process_state="not-running"
  local process_check_rc
  if [[ ",${INPUTIA_PROCESS_RUNNING_FOR_TEST:-}," == *",$process_name,"* ]]; then
    process_state="running"
  elif [[ "${INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST:-0}" != "1" ]]; then
    if inputia_process_running "$process_name"; then
      process_state="running"
    else
      process_check_rc=$?
    fi
    if [[ "${process_check_rc:-1}" -eq 2 ]]; then
      echo "${process_name}Preflight=unknown"
      echo "processListAvailable=false reason=process-list-unavailable"
      printf '%s\n' "$INPUTIA_LAST_PROCESS_CHECK_OUTPUT" | /usr/bin/sed 's/^/processListOutput: /'
      echo "guiSmokeReady=false reason=process-list-unavailable"
      echo "$ready_var=false reason=process-list-unavailable"
      exit "$exit_code"
    fi
  fi

  if [[ "$process_state" == "running" ]]; then
    echo "${process_name}Preflight=running"
    if [[ "$allow_value" != "1" ]]; then
      echo "guiSmokeReady=false reason=$reason"
      echo "$ready_var=false reason=$reason"
      exit "$exit_code"
    fi
    echo "${process_name}PreflightAllowed=true"
  else
    echo "${process_name}Preflight=not-running"
  fi
}

inputia_textedit_document_count() {
  /usr/bin/python3 <<'PY'
import subprocess

script = r'''
tell application "System Events"
  if not (exists application process "TextEdit") then return 0
  try
    return count of windows of application process "TextEdit"
  on error
    return 0
  end try
end tell
'''

try:
    result = subprocess.run(
        ["/usr/bin/osascript"],
        input=script,
        text=True,
        capture_output=True,
        timeout=2,
        check=False,
    )
except subprocess.TimeoutExpired:
    print("0")
    raise SystemExit(0)

output = result.stdout.strip()
print(output if result.returncode == 0 and output.isdigit() else "0")
PY
}

inputia_require_textedit_idle() {
  local ready_var="$1"
  local exit_code="$2"
  local allow_value="${INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING:-0}"

  local process_check_output process_check_rc
  if [[ "${INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST:-0}" == "1" ]]; then
    INPUTIA_TEXTEDIT_PREFLIGHT="not-running"
    INPUTIA_TEXTEDIT_DOCS_BEFORE="0"
  elif [[ ",${INPUTIA_PROCESS_RUNNING_FOR_TEST:-}," == *,TextEdit,* ]]; then
    INPUTIA_TEXTEDIT_PREFLIGHT="running"
    INPUTIA_TEXTEDIT_DOCS_BEFORE="0"
  else
    INPUTIA_TEXTEDIT_PREFLIGHT="not-running"
    INPUTIA_TEXTEDIT_DOCS_BEFORE="0"
    if inputia_process_running TextEdit; then
      INPUTIA_TEXTEDIT_PREFLIGHT="running"
      INPUTIA_TEXTEDIT_DOCS_BEFORE="$(inputia_textedit_document_count)"
    else
      process_check_rc=$?
    fi
    if [[ "${process_check_rc:-1}" -eq 2 ]]; then
      INPUTIA_TEXTEDIT_PREFLIGHT="unknown"
      INPUTIA_TEXTEDIT_DOCS_BEFORE="unknown"
      echo "textEditPreflight=unknown docs=unknown"
      echo "processListAvailable=false reason=process-list-unavailable"
      printf '%s\n' "$INPUTIA_LAST_PROCESS_CHECK_OUTPUT" | /usr/bin/sed 's/^/processListOutput: /'
      echo "guiSmokeReady=false reason=process-list-unavailable"
      echo "$ready_var=false reason=process-list-unavailable"
      exit "$exit_code"
    fi
  fi

  echo "textEditPreflight=$INPUTIA_TEXTEDIT_PREFLIGHT docs=$INPUTIA_TEXTEDIT_DOCS_BEFORE"
  if [[ "$INPUTIA_TEXTEDIT_PREFLIGHT" != "not-running" && "$allow_value" != "1" ]]; then
    echo "guiSmokeReady=false reason=textedit-already-running"
    echo "$ready_var=false reason=textedit-already-running"
    exit "$exit_code"
  fi
}

inputia_wait_process_exit() {
  local process_name="$1"
  local max_ticks="${2:-50}"
  local waited=0

  while ((waited < max_ticks)); do
    local process_check_rc
    if inputia_process_running "$process_name"; then
      :
    else
      process_check_rc=$?
      if [[ "$process_check_rc" -eq 2 ]]; then
        echo "${process_name}ProcessListAvailable=false reason=process-list-unavailable"
        printf '%s\n' "$INPUTIA_LAST_PROCESS_CHECK_OUTPUT" | /usr/bin/sed 's/^/processListOutput: /'
        return 2
      fi
      return 0
    fi
    /bin/sleep 0.1
    waited=$((waited + 1))
  done
  return 1
}

inputia_run_with_timeout() {
  local label="$1"
  local timeout_seconds="$2"
  shift 2

  if [[ -x /usr/bin/python3 ]]; then
    /usr/bin/python3 - "$label" "$timeout_seconds" "$@" <<'PY'
import os
import signal
import subprocess
import sys
import time

label = sys.argv[1]
timeout_seconds = float(sys.argv[2])
command = sys.argv[3:]

process = subprocess.Popen(command, preexec_fn=os.setsid)
deadline = time.monotonic() + timeout_seconds

while True:
    return_code = process.poll()
    if return_code is not None:
        if return_code < 0:
            raise SystemExit(128 + abs(return_code))
        raise SystemExit(return_code)

    if time.monotonic() >= deadline:
        print(f"inputiaSmokeTimeout={label} seconds={sys.argv[2]}", file=sys.stderr)
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            return_code = process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            return_code = process.wait()
        if return_code < 0:
            raise SystemExit(128 + abs(return_code))
        raise SystemExit(return_code if return_code != 0 else 143)

    time.sleep(0.05)
PY
    return $?
  fi

  "$@" &
  local command_pid=$!

  (
    /bin/sleep "$timeout_seconds"
    if /bin/kill -0 "$command_pid" >/dev/null 2>&1; then
      echo "inputiaSmokeTimeout=$label seconds=$timeout_seconds" >&2
      /bin/kill "$command_pid" >/dev/null 2>&1 || true
      /bin/sleep 1
      /bin/kill -9 "$command_pid" >/dev/null 2>&1 || true
    fi
  ) &
  local timer_pid=$!

  local exit_status=0
  wait "$command_pid" || exit_status=$?
  /bin/kill "$timer_pid" >/dev/null 2>&1 || true
  wait "$timer_pid" >/dev/null 2>&1 || true
  return "$exit_status"
}

inputia_cleanup_textedit_if_started() {
  if [[ "${INPUTIA_TEXTEDIT_PREFLIGHT:-}" == "not-running" &&
    "${INPUTIA_TEXTEDIT_CLEANUP_ALLOWED:-0}" == "1" ]]; then
    /usr/bin/osascript >/dev/null 2>&1 <<'APPLESCRIPT' || true
tell application "System Events" to set textEditRunning to exists application process "TextEdit"
if textEditRunning then tell application "TextEdit" to quit saving no
APPLESCRIPT
    if ! inputia_wait_process_exit TextEdit "${INPUTIA_CLEANUP_WAIT_TICKS:-50}"; then
      echo "textEditCleanupFailed=process-list-or-timeout"
      return 1
    fi
    if inputia_process_running TextEdit; then
      echo "textEditCleanupFailed=process-still-running"
      return 1
    fi
  fi
}

inputia_require_safari_idle() {
  local ready_var="$1"
  local exit_code="$2"
  local allow_value="${INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING:-0}"

  local process_check_output process_check_rc
  if [[ "${INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST:-0}" == "1" ]]; then
    INPUTIA_SAFARI_PREFLIGHT="not-running"
  elif [[ ",${INPUTIA_PROCESS_RUNNING_FOR_TEST:-}," == *,Safari,* ]]; then
    INPUTIA_SAFARI_PREFLIGHT="running"
  else
    INPUTIA_SAFARI_PREFLIGHT="not-running"
    if inputia_process_running Safari; then
      INPUTIA_SAFARI_PREFLIGHT="running"
    else
      process_check_rc=$?
    fi
    if [[ "${process_check_rc:-1}" -eq 2 ]]; then
      INPUTIA_SAFARI_PREFLIGHT="unknown"
      echo "safariPreflight=unknown"
      echo "processListAvailable=false reason=process-list-unavailable"
      printf '%s\n' "$INPUTIA_LAST_PROCESS_CHECK_OUTPUT" | /usr/bin/sed 's/^/processListOutput: /'
      echo "guiSmokeReady=false reason=process-list-unavailable"
      echo "$ready_var=false reason=process-list-unavailable"
      exit "$exit_code"
    fi
  fi

  echo "safariPreflight=$INPUTIA_SAFARI_PREFLIGHT"
  if [[ "$INPUTIA_SAFARI_PREFLIGHT" != "not-running" && "$allow_value" != "1" ]]; then
    echo "guiSmokeReady=false reason=safari-already-running"
    echo "$ready_var=false reason=safari-already-running"
    exit "$exit_code"
  fi
}

inputia_cleanup_safari_if_started() {
  if [[ "${INPUTIA_SAFARI_PREFLIGHT:-}" == "not-running" &&
    "${INPUTIA_SAFARI_CLEANUP_ALLOWED:-0}" == "1" ]]; then
    /usr/bin/osascript >/dev/null 2>&1 <<'APPLESCRIPT' || true
tell application "System Events" to set safariRunning to exists application process "Safari"
if safariRunning then tell application "Safari" to quit
APPLESCRIPT
    if ! inputia_wait_process_exit Safari "${INPUTIA_CLEANUP_WAIT_TICKS:-50}"; then
      echo "safariCleanupFailed=process-list-or-timeout"
      return 1
    fi
    if inputia_process_running Safari; then
      echo "safariCleanupFailed=process-still-running"
      return 1
    fi
  fi
}

inputia_cleanup_smoke_files() {
  if [[ "${INPUTIA_KEEP_SMOKE_LOGS:-0}" == "1" ]]; then
    echo "smokeTempCleanup=skipped"
    return 0
  fi
  for file_path in "$@"; do
    if [[ -n "$file_path" ]]; then
      /bin/rm -f "$file_path" >/dev/null 2>&1 || true
    fi
  done
}

inputia_clipboard_info_restorable_reason() {
  local clipboard_info="$1"
  if [[ "$clipboard_info" == *"Unicode text"* || "$clipboard_info" == *"string"* ||
    "$clipboard_info" == *"«class utf8»"* || "$clipboard_info" == *"«class ut16»"* ]]; then
    if [[ "$clipboard_info" == *"«class RTF"* || "$clipboard_info" == *"TIFF picture"* ||
      "$clipboard_info" == *"JPEG picture"* || "$clipboard_info" == *"GIF picture"* ||
      "$clipboard_info" == *"PDF "* || "$clipboard_info" == *"file URL"* ||
      "$clipboard_info" == *"alias"* ]]; then
      echo "non-text-clipboard"
      return 1
    fi
    echo "text-restorable"
    return 0
  fi

  echo "missing-text-clipboard"
  return 2
}

inputia_current_clipboard_info() {
  if [[ "${INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST:-0}" == "1" &&
    -n "${INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST+x}" ]]; then
    /usr/bin/printf '%s\n' "$INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST"
    return 0
  fi
  /usr/bin/osascript -e 'clipboard info' 2>/dev/null || true
}

inputia_try_write_clipboard_text() {
  local text="$1"
  if /usr/bin/printf '%s' "$text" | /usr/bin/pbcopy >/dev/null 2>&1; then
    return 0
  fi
  echo "clipboardWrite=false reason=pasteboard-unavailable"
  return 1
}

inputia_require_text_clipboard_restorable() {
  local ready_var="$1"
  local exit_code="$2"

  if [[ "${INPUTIA_ALLOW_NON_TEXT_CLIPBOARD_SMOKE:-0}" == "1" ]]; then
    echo "clipboardRestorable=skipped reason=override"
    return 0
  fi

  local clipboard_info
  local reason
  clipboard_info="$(inputia_current_clipboard_info)"
  set +e
  reason="$(inputia_clipboard_info_restorable_reason "$clipboard_info")"
  set -e
  if [[ "$reason" == "text-restorable" ]]; then
    echo "clipboardRestorable=true"
    return 0
  fi

  echo "clipboardRestorable=false reason=$reason info=${clipboard_info:-unknown}"
  echo "$ready_var=false reason=$reason"
  exit "$exit_code"
}

inputia_capture_debug_events_env() {
  INPUTIA_PREVIOUS_DEBUG_EVENTS_VALUE="$(/bin/launchctl getenv INPUTIA_DEBUG_EVENTS 2>/dev/null || true)"
  if [[ -n "$INPUTIA_PREVIOUS_DEBUG_EVENTS_VALUE" ]]; then
    INPUTIA_PREVIOUS_DEBUG_EVENTS_WAS_SET=1
  else
    INPUTIA_PREVIOUS_DEBUG_EVENTS_WAS_SET=0
  fi
}

inputia_restore_debug_events_env() {
  if [[ "${INPUTIA_PREVIOUS_DEBUG_EVENTS_WAS_SET:-0}" == "1" ]]; then
    /bin/launchctl setenv INPUTIA_DEBUG_EVENTS "$INPUTIA_PREVIOUS_DEBUG_EVENTS_VALUE" >/dev/null 2>&1 || true
  else
    /bin/launchctl unsetenv INPUTIA_DEBUG_EVENTS >/dev/null 2>&1 || true
  fi
}

inputia_try_set_debug_events_env() {
  local event_log="$1"
  local setenv_output
  if setenv_output="$(/bin/launchctl setenv INPUTIA_DEBUG_EVENTS "$event_log" 2>&1)"; then
    return 0
  fi
  echo "debugEnvSet=false reason=launchctl-env-unavailable"
  if [[ -n "$setenv_output" ]]; then
    printf '%s\n' "$setenv_output" | /usr/bin/sed 's/^/debugEnvSetOutput: /'
  fi
  return 1
}

inputia_set_debug_events_env_or_exit() {
  local event_log="$1"
  local ready_var="$2"
  local exit_code="$3"
  if inputia_try_set_debug_events_env "$event_log"; then
    return 0
  fi
  echo "guiSmokeReady=false reason=launchctl-env-unavailable"
  echo "$ready_var=false reason=launchctl-env-unavailable"
  exit "$exit_code"
}

inputia_prepare_debug_event_log() {
  local event_log="$1"
  local event_log_provided="$2"
  if [[ -n "$event_log_provided" && -e "$event_log" && ! -f "$event_log" ]]; then
    echo "debugEventLogPrepare=false path=$event_log reason=not-regular-file"
    return 1
  fi
  if [[ -n "$event_log_provided" ]]; then
    : >"$event_log"
  else
    /bin/rm -f "$event_log"
  fi
  echo "debugEventLogPrepare=true path=$event_log"
}

inputia_assert_debug_event_log_clean() {
  local event_log="$1"
  local ready_var="$2"
  local exit_code="$3"

  if [[ -s "$event_log" ]]; then
    echo "debugEventLogClean=false path=$event_log"
    echo "$ready_var=false reason=debug-event-log-not-clean"
    /usr/bin/tail -n 120 "$event_log" 2>/dev/null || true
    exit "$exit_code"
  fi
  echo "debugEventLogClean=true"
}

inputia_current_input_source_id() {
  local executable="$1"
  local tis_tool="${2:-}"
  if [[ -n "$tis_tool" && -x "$tis_tool" ]]; then
    "$tis_tool" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return 0
  fi
  if [[ -x "$executable" ]]; then
    "$executable" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return 0
  fi
  echo ""
}

inputia_restore_previous_input_source() {
  local tis_tool="$1"
  local executable="$2"
  local restore_log="${3:-}"
  local previous_id="${INPUTIA_PREVIOUS_INPUT_SOURCE_ID:-}"
  local current_id

  if [[ -z "$previous_id" ]]; then
    return 0
  fi

  current_id="$(inputia_current_input_source_id "$executable" "$tis_tool")"
  if [[ "$current_id" == "$previous_id" ]]; then
    echo "inputSourceRestore=skipped reason=already-current"
    return 0
  fi

  if [[ -x "$tis_tool" ]]; then
    if [[ -n "$restore_log" ]]; then
      "$tis_tool" --select-source-id "$previous_id" >"$restore_log" 2>&1 || true
    else
      "$tis_tool" --select-source-id "$previous_id" >/dev/null 2>&1 || true
    fi
  else
    echo "inputSourceRestore=skipped reason=missing-tis-tool"
    return 0
  fi

  current_id="$(inputia_current_input_source_id "$executable" "$tis_tool")"
  if [[ "$current_id" == "$previous_id" ]]; then
    echo "inputSourceRestore=true id=$previous_id"
    return 0
  else
    echo "inputSourceRestore=false expected=$previous_id actual=${current_id:-unknown}"
    return 1
  fi
}

inputia_select_input_source_or_exit() {
  local app="$1"
  local executable="$2"
  local tis_tool="$3"
  local select_log="$4"
  local ready_var="$5"
  local exit_code="$6"

  INPUTIA_PREVIOUS_INPUT_SOURCE_ID="$(inputia_current_input_source_id "$executable" "$tis_tool")"
  echo "previousInputSourceID=${INPUTIA_PREVIOUS_INPUT_SOURCE_ID:-unknown}" >>"$select_log"
  if [[ -z "$INPUTIA_PREVIOUS_INPUT_SOURCE_ID" ]]; then
    echo "guiSmokeReady=false reason=input-source-capture-failed"
    echo "$ready_var=false reason=input-source-capture-failed"
    /usr/bin/tail -n 120 "$select_log"
    exit "$exit_code"
  fi

  if [[ -x "$tis_tool" ]]; then
    if [[ "${INPUTIA_TIS_REGISTER_BEFORE_SELECT:-0}" == "1" ]]; then
      INPUTIA_APP="$app" INPUTIA_TIS_REQUIRE_APP_MATCH=1 "$tis_tool" >>"$select_log"
    else
      INPUTIA_APP="$app" INPUTIA_TIS_REQUIRE_APP_MATCH=1 \
        "$tis_tool" --select-inputia-source-id "${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Hans}" >>"$select_log"
    fi
  elif [[ "${INPUTIA_TIS_REGISTER_BEFORE_SELECT:-0}" == "1" ]]; then
    "$executable" --select-input-source >>"$select_log"
  else
    echo "selectSourceFoundInEnabledList=false" >>"$select_log"
    echo "selectOnlyMissingTISTool=true path=$tis_tool" >>"$select_log"
  fi

  if ! /usr/bin/grep -q 'selectStatus=0' "$select_log" ||
    ! /usr/bin/grep -q 'selectCurrentMatchesTarget=true' "$select_log"; then
    echo "guiSmokeReady=false reason=input-source-not-selected"
    echo "$ready_var=false reason=input-source-not-selected"
    /usr/bin/tail -n 120 "$select_log"
    exit "$exit_code"
  fi
}
