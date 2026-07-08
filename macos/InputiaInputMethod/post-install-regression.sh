#!/bin/zsh
set -euo pipefail

SCRIPT_PATH="$0"
if [[ -n "${BASH_SOURCE:-}" ]]; then
  SCRIPT_PATH="${BASH_SOURCE[0]}"
fi
ROOT_DIR="$(cd "$(dirname "$SCRIPT_PATH")" && pwd)"
USER_DEFAULT_APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
DEFAULT_APP="/Library/Input Methods/InputiaInputMethod.app"
if [[ -d "$USER_DEFAULT_APP" ]]; then
  DEFAULT_APP="$USER_DEFAULT_APP"
fi
APP="${1:-$DEFAULT_APP}"
LEGACY_APP="${INPUTIA_LEGACY_APP:-/Library/Input Methods/IputiaInputMethod.app}"
USER_APP="${INPUTIA_USER_APP:-$HOME/Library/Input Methods/InputiaInputMethod.app}"
USER_LEGACY_APP="${INPUTIA_USER_LEGACY_APP:-$HOME/Library/Input Methods/IputiaInputMethod.app}"
USER_SETTINGS_APP="${INPUTIA_USER_SETTINGS_APP:-$HOME/Applications/Inputia 设置.app}"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Main}"
LOCK_DIR="${INPUTIA_POST_INSTALL_LOCK_DIR:-/tmp/inputia-post-install-regression.lock}"
LOCK_HELD=0
POST_INSTALL_TIS_BLOCK_REASON=unknown

section() {
  printf '\n== %s ==\n' "$1"
}

release_regression_lock() {
  if [[ "$LOCK_HELD" == "1" ]]; then
    /bin/rm -rf "$LOCK_DIR" >/dev/null 2>&1 || true
    LOCK_HELD=0
  fi
}

acquire_regression_lock() {
  if [[ "${INPUTIA_SKIP_POST_INSTALL_LOCK:-0}" == "1" ]]; then
    echo "postInstallLock=skipped"
    return 0
  fi

  while ! /bin/mkdir "$LOCK_DIR" 2>/dev/null; do
    local existing_pid=""
    if [[ -f "$LOCK_DIR/pid" ]]; then
      existing_pid="$(/bin/cat "$LOCK_DIR/pid" 2>/dev/null || true)"
    fi

    if [[ -n "$existing_pid" ]] && /bin/kill -0 "$existing_pid" >/dev/null 2>&1; then
      echo "postInstallRegressionReady=false reason=already-running pid=$existing_pid"
      exit 5
    fi

    echo "postInstallLockStale=true path=$LOCK_DIR pid=${existing_pid:-unknown}"
    /bin/rm -rf "$LOCK_DIR" >/dev/null 2>&1 || true
  done

  LOCK_HELD=1
  echo "$$" >"$LOCK_DIR/pid"
  echo "postInstallLockAcquired=true"
  trap release_regression_lock EXIT
}

process_match_by_ps() {
  local process_name="$1"
  /bin/ps -axo pid=,comm=,command= 2>/dev/null |
    /usr/bin/awk -v process_name="$process_name" '
      {
        command = (NF >= 3) ? substr($0, index($0, $3)) : ""
        launcher = (command ~ "^/bin/(zsh|bash|sh)( |$)" || command ~ "^/usr/bin/(sudo|awk|grep|sed)( |$)")
      }
      !launcher {
        if ($2 == process_name ||
          $3 == process_name ||
          $3 ~ ("/" process_name "$") ||
          command ~ ("^" process_name "([ ]|$)") ||
          command ~ ("/" process_name "([ ]|$)") ||
          command ~ (process_name "\\.app/Contents/MacOS/" process_name "([ ]|$)")) {
          found = 1
        }
      }
      END { exit found ? 0 : 1 }
    '
}

process_running() {
  local process_name="$1"
  local process_check_output process_check_rc
  POST_INSTALL_LAST_PROCESS_CHECK_OUTPUT=""
  set +e
  process_check_output="$(/usr/bin/pgrep -x "$process_name" 2>&1 >/dev/null)"
  process_check_rc=$?
  set -e
  if [[ "$process_check_rc" -eq 0 ]]; then
    return 0
  fi
  if process_match_by_ps "$process_name"; then
    return 0
  fi
  if /bin/ps -axo pid=,comm=,command= >/dev/null 2>&1; then
    return 1
  fi
  if [[ -n "$process_check_output" ]]; then
    POST_INSTALL_LAST_PROCESS_CHECK_OUTPUT="$process_check_output"
    return 2
  fi
  return 1
}

require_ui_process_idle() {
  local process_name="$1"
  local allow_value="$2"
  local reason="$3"

  local process_state="not-running"
  local process_check_rc
  if [[ ",${INPUTIA_UI_PROCESS_RUNNING_FOR_TEST:-}," == *",$process_name,"* ]]; then
    process_state="running"
  elif [[ "${INPUTIA_UI_PROCESS_IGNORE_REAL_FOR_TEST:-0}" != "1" ]]; then
    if process_running "$process_name"; then
      process_state="running"
    else
      process_check_rc=$?
    fi
    if [[ "${process_check_rc:-1}" -eq 2 ]]; then
      echo "${process_name}Preflight=unknown"
      echo "processListAvailable=false reason=process-list-unavailable"
      printf '%s\n' "$POST_INSTALL_LAST_PROCESS_CHECK_OUTPUT" | /usr/bin/sed 's/^/processListOutput: /'
      echo "guiSmokeReady=false reason=process-list-unavailable"
      echo "postInstallUiSmokeReady=false reason=process-list-unavailable"
      exit 4
    fi
  fi

  if [[ "$process_state" == "running" ]]; then
    echo "${process_name}Preflight=running"
    if [[ "$allow_value" != "1" ]]; then
      echo "guiSmokeReady=false reason=$reason"
      echo "postInstallUiSmokeReady=false reason=$reason"
      exit 4
    fi
    echo "${process_name}PreflightAllowed=true"
  else
    echo "${process_name}Preflight=not-running"
  fi
}

run_ui_process_preflight_self_check_case() {
  local label="$1"
  local process_name="$2"
  local allow_value="$3"
  local reason="$4"
  local expected_rc="$5"
  local output rc
  set +e
  output="$(
    INPUTIA_UI_PROCESS_RUNNING_FOR_TEST="$process_name" \
      require_ui_process_idle "$process_name" "$allow_value" "$reason" 2>&1
  )"
  rc=$?
  set -e
  printf '%s\n' "$output" | /usr/bin/sed "s/^/postInstallUiPreflightSelfCheck case=$label /"
  echo "postInstallUiPreflightSelfCheck case=$label rc=$rc"
  if [[ "$rc" != "$expected_rc" ]]; then
    echo "postInstallUiPreflightSelfCheck=false case=$label reason=unexpected-rc expected=$expected_rc actual=$rc"
    exit 20
  fi
}

if [[ "${INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  export INPUTIA_UI_PROCESS_IGNORE_REAL_FOR_TEST=1
  run_ui_process_preflight_self_check_case textedit-block TextEdit 0 textedit-already-running 4
  run_ui_process_preflight_self_check_case safari-block Safari 0 safari-already-running 4
  run_ui_process_preflight_self_check_case inputia-block InputiaInputMethod 0 inputia-host-running 4
  run_ui_process_preflight_self_check_case textedit-allow TextEdit 1 textedit-already-running 0
  run_ui_process_preflight_self_check_case safari-allow Safari 1 safari-already-running 0
  echo "postInstallUiPreflightSelfCheck=true"
  exit 0
fi

require_gui_session() {
  if [[ "${INPUTIA_SKIP_GUI_SESSION_CHECK:-0}" == "1" ]]; then
    echo "guiSessionCheck=skipped"
    return 0
  fi

  local console_user
  console_user="$(/usr/bin/stat -f '%Su' /dev/console 2>/dev/null || true)"
  echo "guiConsoleUser=${console_user:-unknown}"
  if [[ -z "$console_user" || "$console_user" == "root" || "$console_user" == "_mbsetupuser" ]]; then
    echo "guiSmokeReady=false reason=no-console-user"
    echo "postInstallUiSmokeReady=false reason=no-console-user"
    exit 7
  fi
  local console_uid
  console_uid="$(/usr/bin/stat -f '%u' /dev/console 2>/dev/null || true)"
  echo "guiConsoleUID=${console_uid:-unknown}"
  if [[ -z "$console_uid" ]] || ! /bin/launchctl print "gui/$console_uid" >/dev/null 2>&1; then
    echo "guiSmokeReady=false reason=gui-bootstrap-unavailable"
    echo "postInstallUiSmokeReady=false reason=gui-bootstrap-unavailable"
    exit 7
  fi

  local session_state
  session_state="$(/usr/sbin/ioreg -n Root -d1 2>/dev/null || true)"
  if [[ "$session_state" != *"kCGSessionLoginDoneKey\"=Yes"* ]]; then
    echo "guiSmokeReady=false reason=login-not-complete"
    echo "postInstallUiSmokeReady=false reason=login-not-complete"
    exit 7
  fi
  if [[ "$session_state" == *"CGSSessionScreenIsLocked\"=Yes"* ||
    "$session_state" == *"kCGSSessionScreenIsLocked\"=Yes"* ]]; then
    echo "guiSmokeReady=false reason=screen-locked"
    echo "postInstallUiSmokeReady=false reason=screen-locked"
    exit 7
  fi

  local frontmost_app
  if ! frontmost_app="$(/usr/bin/osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null)"; then
    echo "guiSmokeReady=false reason=frontmost-unavailable"
    echo "postInstallUiSmokeReady=false reason=frontmost-unavailable"
    exit 7
  fi
  echo "guiFrontmostApp=$frontmost_app"
  if [[ "$frontmost_app" == "loginwindow" ]]; then
    echo "guiSmokeReady=false reason=loginwindow-frontmost"
    echo "postInstallUiSmokeReady=false reason=loginwindow-frontmost"
    exit 7
  fi
}

tis_value() {
  local dump="$1"
  local source_id="$2"
  local key="$3"
  /usr/bin/awk -F= -v source_id="$source_id" -v key="$key" '
    $1 == "id" { active = ($2 == source_id) }
    active && $1 == key { print $2; exit }
  ' <<<"$dump"
}

tis_matches() {
  local dump="$1"
  local include="$2"
  /usr/bin/awk -F= -v include="$include" '
    $1 == "includeAllInstalled" { active = ($2 == include) }
    active && $1 == "matches" { print $2; exit }
  ' <<<"$dump"
}

absolute_path() {
  /usr/bin/python3 - "$1" <<'PY'
import os
import sys

print(os.path.abspath(sys.argv[1]))
PY
}

current_source_id() {
  if [[ -x "$TIS_TOOL" ]]; then
    "$TIS_TOOL" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return
  fi
  if [[ -x "$EXECUTABLE" ]]; then
    "$EXECUTABLE" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return
  fi
  echo ""
}

print_tis_gui_readiness() {
  local readiness_output ready block_reason
  readiness_output="$("$ROOT_DIR/tis-readiness.sh" "$APP" 2>&1 || true)"
  printf '%s\n' "$readiness_output"
  if /usr/bin/grep -q '^tisReadiness=true$' <<<"$readiness_output"; then
    ready=true
  else
    ready=false
  fi
  block_reason="$(/usr/bin/awk -F= '$1 == "tis.readinessBlockReason" { print $2; found = 1; exit } END { if (!found) print "unknown" }' <<<"$readiness_output")"
  POST_INSTALL_TIS_BLOCK_REASON="$block_reason"
  echo "postInstallTISReady=$ready"
  echo "postInstallTISBlockReason=$block_reason"
  [[ "$ready" == "true" ]]
}

acquire_regression_lock

section "installed Inputia app"
if [[ ! -x "$EXECUTABLE" ]]; then
  echo "inputiaInstalled=false path=$APP"
  exit 1
fi
echo "inputiaInstalled=true path=$APP"

section "legacy typo app"
if [[ -e "$LEGACY_APP" ]]; then
  echo "legacyIputiaPresent=true path=$LEGACY_APP"
  exit 2
fi
echo "legacyIputiaPresent=false"

section "user host conflict"
if [[ "$APP" == "$USER_APP" ]]; then
  if [[ -e "$USER_LEGACY_APP" ]]; then
    echo "userHostConflict=true path=$USER_APP legacyPath=$USER_LEGACY_APP settingsPath=$USER_SETTINGS_APP"
    exit 3
  fi
  echo "userHostConflict=false"
elif [[ -e "$USER_APP" || -e "$USER_LEGACY_APP" || -e "$USER_SETTINGS_APP" ]]; then
  echo "userHostConflict=true path=$USER_APP legacyPath=$USER_LEGACY_APP settingsPath=$USER_SETTINGS_APP"
  exit 3
else
  echo "userHostConflict=false"
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" == "1" ]]; then
  section "TIS readiness for GUI smoke"
  if print_tis_gui_readiness; then
    post_install_tis_ready=true
  else
    post_install_tis_ready=false
  fi

  if [[ "$post_install_tis_ready" != "true" ]]; then
    if [[ "$POST_INSTALL_TIS_BLOCK_REASON" == "signature-rejected" ]]; then
      echo "guiSmokeReady=false reason=signature-rejected"
      echo "postInstallUiSmokeReady=false reason=signature-rejected"
    else
      echo "guiSmokeReady=false reason=tis-not-ready"
      echo "postInstallUiSmokeReady=false reason=tis-not-ready"
    fi
    exit 6
  fi
fi

section "system verification"
/bin/zsh "$ROOT_DIR/verify-system.sh" "$APP"

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" == "1" ]]; then
  section "UI smoke preflight"
  require_gui_session
  require_ui_process_idle \
    "TextEdit" \
    "0" \
    "textedit-already-running"
  require_ui_process_idle \
    "Safari" \
    "0" \
    "safari-already-running"
  require_ui_process_idle \
    "InputiaInputMethod" \
    "0" \
    "inputia-host-running"
  echo "postInstallUiSmokeReady=true"

  section "TextEdit IMK smoke"
  "$ROOT_DIR/smoke-textedit.sh" "$APP"

  section "TextEdit command shortcut smoke"
  "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$APP"

  section "Safari input source context"
  "$ROOT_DIR/diagnose-safari-input-source.sh" "$APP"

  section "Safari typing smoke"
  "$ROOT_DIR/smoke-safari-typing.sh" "$APP"

  section "Safari command shortcut smoke"
  "$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$APP"

  section "Safari raw ASCII enter smoke"
  "$ROOT_DIR/smoke-safari-enter.sh" "$APP"

  section "Clipboard recall smoke"
  /bin/bash "$ROOT_DIR/smoke-clipboard-recall.sh" "$APP"
else
  section "UI smoke"
  echo "uiSmokeSkipped=true reason=disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow TextEdit/Safari smoke tests"
fi

section "result"
echo "postInstallRegressionPassed=true"
