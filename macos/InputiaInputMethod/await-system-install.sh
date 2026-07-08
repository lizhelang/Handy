#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
SYSTEM_APP="/Library/Input Methods/InputiaInputMethod.app"
USER_APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
USER_LEGACY_APP="$HOME/Library/Input Methods/IputiaInputMethod.app"
USER_SETTINGS_APP="$HOME/Applications/Inputia 设置.app"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Main}"
WAIT_SECONDS="${INPUTIA_INSTALL_WAIT_SECONDS:-300}"
POLL_SECONDS="${INPUTIA_INSTALL_POLL_SECONDS:-5}"

cdhash() {
  if [[ ! -d "$1" ]]; then
    return 1
  fi
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

version() {
  if [[ ! -f "$1/Contents/Info.plist" ]]; then
    echo ""
    return
  fi
  /usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$1/Contents/Info.plist" 2>/dev/null || true
}

app_signature_accepted() {
  if [[ ! -d "$1" ]]; then
    echo false
    return
  fi
  local assessment
  assessment="$(/usr/sbin/spctl --assess --type execute --verbose=4 "$1" 2>&1 || true)"
  if [[ "$assessment" == *": accepted"* ]]; then
    echo true
  else
    echo false
  fi
}

expected_cdhash="$(cdhash "$BUILD_APP")"
expected_version="$(version "$BUILD_APP")"
expected_icon="$(/usr/bin/python3 - "$APP/Contents/Resources/inputia.pdf" <<'PY'
import os
import sys

print(os.path.abspath(sys.argv[1]))
PY
)"
deadline=$((SECONDS + WAIT_SECONDS))
last_target_matches_build=false
last_tis_block_reason=unknown

app_status_line() {
  local label="$1"
  local path="$2"
  local actual_version actual_cdhash matches
  actual_version="$(version "$path")"
  actual_cdhash="$(cdhash "$path" || true)"
  if [[ -n "$expected_cdhash" && "$actual_cdhash" == "$expected_cdhash" ]]; then
    matches=true
  else
    matches=false
  fi
  echo "$label.exists=$([[ -d "$path" ]] && echo true || echo false) $label.validBundle=$([[ -f "$path/Contents/Info.plist" ]] && echo true || echo false) $label.version=$actual_version $label.cdhash=$actual_cdhash $label.matchesBuild=$matches"
}

process_pids_by_ps() {
  local process_name="$1"
  local ps_output
  ps_output="$(/bin/ps -axo pid=,comm=,command= 2>/dev/null |
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
          print $1
        }
      }
    ')"
  if [[ -n "$ps_output" ]]; then
    printf '%s\n' "$ps_output"
    return 0
  fi
  /bin/ps -axo pid=,comm=,command= >/dev/null 2>&1 || return 2
  return 1
}

process_pids() {
  local process_name="$1"
  local pgrep_output pgrep_rc ps_rc
  set +e
  pgrep_output="$(/usr/bin/pgrep -x "$process_name" 2>&1)"
  pgrep_rc=$?
  set -e
  if [[ "$pgrep_rc" -eq 0 ]]; then
    printf '%s\n' "$pgrep_output"
    return 0
  fi
  set +e
  process_pids_by_ps "$process_name"
  ps_rc=$?
  set -e
  if [[ "$ps_rc" -eq 0 || "$ps_rc" -eq 1 ]]; then
    return "$ps_rc"
  fi
  if [[ -n "$pgrep_output" ]]; then
    printf '%s\n' "$pgrep_output"
    return 2
  fi
  return 1
}

running_status() {
  local pids pid command running_app running_cdhash running_version matches
  local pids_output pids_rc
  set +e
  pids_output="$(process_pids InputiaInputMethod 2>&1)"
  pids_rc=$?
  set -e
  if [[ "$pids_rc" -eq 0 ]]; then
    pids="$pids_output"
  elif [[ "$pids_rc" -eq 2 ]]; then
    echo "running.exists=unknown"
    echo "processListAvailable=false reason=process-list-unavailable"
    printf '%s\n' "$pids_output" | /usr/bin/sed 's/^/processListOutput: /'
    return
  else
    pids=""
  fi
  if [[ -z "$pids" ]]; then
    echo "running.exists=false"
    return
  fi

  while IFS= read -r pid; do
    [[ -z "$pid" ]] && continue
    command="$(/bin/ps -p "$pid" -o command= 2>/dev/null || true)"
    running_app=unknown
    if [[ "$command" == "$SYSTEM_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
      running_app="$SYSTEM_APP"
    elif [[ "$command" == "$USER_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
      running_app="$USER_APP"
    elif [[ "$command" == "$BUILD_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
      running_app="$BUILD_APP"
    fi
    running_version=""
    running_cdhash=""
    matches=false
    if [[ "$running_app" != "unknown" ]]; then
      running_version="$(version "$running_app")"
      running_cdhash="$(cdhash "$running_app" || true)"
      if [[ -n "$expected_cdhash" && "$running_cdhash" == "$expected_cdhash" ]]; then
        matches=true
      fi
    fi
    echo "running.exists=true running.pid=$pid running.app=$running_app running.version=$running_version running.cdhash=$running_cdhash running.matchesBuild=$matches"
  done <<<"$pids"
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

current_source_id() {
  if [[ -x "$TIS_TOOL" ]]; then
    "$TIS_TOOL" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return
  fi
  if [[ -x "$APP/Contents/MacOS/InputiaInputMethod" ]]; then
    "$APP/Contents/MacOS/InputiaInputMethod" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return
  fi
  echo ""
}

tis_status_line() {
  local dump enabled_matches installed_matches hans_icon hans_enabled hans_selected icon_matches current_id current_matches block_reason signature_accepted
  if [[ ! -x "$TIS_TOOL" ]]; then
    echo "tis.tool=false path=$TIS_TOOL"
    return
  fi

  signature_accepted="$(app_signature_accepted "$APP")"
  dump="$(INPUTIA_APP="$APP" "$TIS_TOOL" --dump 2>/dev/null || true)"
  enabled_matches="$(tis_matches "$dump" false)"
  installed_matches="$(tis_matches "$dump" true)"
  hans_icon="$(tis_value "$dump" "$TARGET_MODE_ID" iconURL)"
  hans_enabled="$(tis_value "$dump" "$TARGET_MODE_ID" enabled)"
  hans_selected="$(tis_value "$dump" "$TARGET_MODE_ID" selected)"
  if [[ "$hans_icon" == "$expected_icon" ]]; then
    icon_matches=true
  else
    icon_matches=false
  fi
  current_id="$(current_source_id)"
  if [[ "$current_id" == "$TARGET_MODE_ID" ]]; then
    current_matches=true
  else
    current_matches=false
  fi
  block_reason=unknown
  if [[ "$signature_accepted" != "true" ]]; then
    block_reason=signature-rejected
  elif [[ "${enabled_matches:-0}" != "0" &&
    -n "${enabled_matches:-}" &&
    "$icon_matches" == "true" &&
    "${hans_enabled:-false}" == "true" ]]; then
    block_reason=none
  elif [[ "${enabled_matches:-0}" == "0" || -z "${enabled_matches:-}" ]]; then
    block_reason=missing-enabled-source
  elif [[ "$icon_matches" != "true" ]]; then
    block_reason=icon-mismatch
  elif [[ "${hans_enabled:-false}" != "true" ]]; then
    block_reason=hans-disabled
  fi
  echo "tis.tool=true tis.appSignatureAccepted=$signature_accepted tis.enabledMatches=${enabled_matches:-unknown} tis.installedMatches=${installed_matches:-unknown} tis.hansIconMatchesApp=$icon_matches tis.hansEnabled=${hans_enabled:-unknown} tis.hansSelected=${hans_selected:-unknown} tis.currentID=${current_id:-unknown} tis.currentMatchesTarget=$current_matches tis.readinessBlockReason=$block_reason"
}

tis_ready() {
  local status_line="$1"
  [[ "$status_line" == *"tis.appSignatureAccepted=true"* &&
    "$status_line" != *"tis.enabledMatches=0"* &&
    "$status_line" != *"tis.enabledMatches=unknown"* &&
    "$status_line" == *"tis.hansIconMatchesApp=true"* &&
    "$status_line" == *"tis.hansEnabled=true"* ]]
}

tis_block_reason_from_status() {
  local status_line="$1"
  local reason="${status_line##*tis.readinessBlockReason=}"
  reason="${reason%% *}"
  if [[ -z "$reason" || "$reason" == "$status_line" ]]; then
    echo tis-not-ready
  else
    echo "$reason"
  fi
}

append_block_reason() {
  local reasons="$1"
  local reason="$2"
  if [[ ",$reasons," == *",$reason,"* ]]; then
    echo "$reasons"
  elif [[ -z "$reasons" ]]; then
    echo "$reason"
  else
    echo "$reasons,$reason"
  fi
}

user_host_conflict() {
  if [[ -n "${INPUTIA_AWAIT_USER_HOST_CONFLICT_FOR_TEST:-}" ]]; then
    echo "$INPUTIA_AWAIT_USER_HOST_CONFLICT_FOR_TEST"
    return
  fi
  if [[ -e "$USER_APP" || -e "$USER_LEGACY_APP" || -e "$USER_SETTINGS_APP" ]]; then
    echo true
  else
    echo false
  fi
}

process_preflight() {
  local process_name="$1"
  local process_rc
  if [[ ",${INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST:-}," == *",$process_name,"* ]]; then
    echo "running"
    return
  fi
  if [[ "${INPUTIA_AWAIT_IGNORE_REAL_PROCESSES_FOR_TEST:-0}" == "1" ]]; then
    echo "not-running"
    return
  fi
  if process_pids "$process_name" >/dev/null; then
    echo "running"
  else
    process_rc=$?
    if [[ "$process_rc" -eq 2 ]]; then
      echo "unknown"
    else
      echo "not-running"
    fi
  fi
}

gui_session_block_reason() {
  if [[ -n "${INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST:-}" ]]; then
    echo "$INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST"
    return
  fi

  if [[ "${INPUTIA_SKIP_GUI_SESSION_CHECK:-0}" == "1" ]]; then
    echo "none"
    return
  fi

  local console_user
  console_user="$(/usr/bin/stat -f '%Su' /dev/console 2>/dev/null || true)"
  if [[ -z "$console_user" || "$console_user" == "root" || "$console_user" == "_mbsetupuser" ]]; then
    echo "no-console-user"
    return
  fi
  local console_uid
  console_uid="$(/usr/bin/stat -f '%u' /dev/console 2>/dev/null || true)"
  if [[ -z "$console_uid" ]] || ! /bin/launchctl print "gui/$console_uid" >/dev/null 2>&1; then
    echo "gui-bootstrap-unavailable"
    return
  fi

  local session_state
  session_state="$(/usr/sbin/ioreg -n Root -d1 2>/dev/null || true)"
  if [[ "$session_state" != *"kCGSessionLoginDoneKey\"=Yes"* ]]; then
    echo "login-not-complete"
    return
  fi
  if [[ "$session_state" == *"CGSSessionScreenIsLocked\"=Yes"* ||
    "$session_state" == *"kCGSSessionScreenIsLocked\"=Yes"* ]]; then
    echo "screen-locked"
    return
  fi

  local frontmost_app
  if ! frontmost_app="$(/usr/bin/osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null)"; then
    echo "frontmost-unavailable"
    return
  fi
  if [[ "$frontmost_app" == "loginwindow" ]]; then
    echo "loginwindow-frontmost"
    return
  fi

  echo "none"
}

ui_smoke_status_line() {
  local target_matches_build="$1"
  local tis_status_line="$2"
  local gui_block_reason textedit_state safari_state inputia_state would_start block_reasons user_host_conflict_state
  if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
    echo "uiSmokeRequested=false uiSmokeWouldStart=false uiSmokeBlockReason=ui-smoke-disabled uiSmokeBlockReasons=ui-smoke-disabled"
    return
  fi

  block_reasons=""
  user_host_conflict_state="$(user_host_conflict)"
  if [[ "$target_matches_build" != "true" ]]; then
    block_reasons="$(append_block_reason "$block_reasons" "target-cdhash-mismatch")"
  fi
  if ! tis_ready "$tis_status_line"; then
    block_reasons="$(append_block_reason "$block_reasons" "$(tis_block_reason_from_status "$tis_status_line")")"
  fi
  if [[ "$user_host_conflict_state" == "true" ]]; then
    block_reasons="$(append_block_reason "$block_reasons" "user-host-conflict")"
  fi
  if [[ "$target_matches_build" != "true" ]]; then
    echo "uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=$block_reasons"
    return
  fi
  if ! tis_ready "$tis_status_line"; then
    echo "uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=$(tis_block_reason_from_status "$tis_status_line") uiSmokeBlockReasons=$block_reasons"
    return
  fi
  if [[ "$user_host_conflict_state" == "true" ]]; then
    echo "uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=user-host-conflict uiSmokeBlockReasons=$block_reasons"
    return
  fi
  gui_block_reason="$(gui_session_block_reason)"
  if [[ "$gui_block_reason" != "none" ]]; then
    echo "uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=$gui_block_reason uiSmokeBlockReasons=$gui_block_reason"
    return
  fi

  textedit_state="$(process_preflight TextEdit)"
  safari_state="$(process_preflight Safari)"
  inputia_state="$(process_preflight InputiaInputMethod)"
  would_start=true
  if [[ "$textedit_state" == "unknown" || "$safari_state" == "unknown" || "$inputia_state" == "unknown" ]]; then
    would_start=false
    echo "uiSmokeRequested=true uiTextEditPreflight=$textedit_state uiSafariPreflight=$safari_state uiInputiaHostPreflight=$inputia_state uiSmokeWouldStart=$would_start uiSmokeBlockReason=process-list-unavailable uiSmokeBlockReasons=process-list-unavailable"
    return
  fi
  if [[ "$inputia_state" == "running" ]]; then
    would_start=false
    echo "uiSmokeRequested=true uiTextEditPreflight=$textedit_state uiSafariPreflight=$safari_state uiInputiaHostPreflight=$inputia_state uiSmokeWouldStart=$would_start uiSmokeBlockReason=inputia-host-running uiSmokeBlockReasons=inputia-host-running"
    return
  fi
  if [[ "$textedit_state" == "running" && "${INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
    would_start=false
    echo "uiSmokeRequested=true uiTextEditPreflight=$textedit_state uiSafariPreflight=$safari_state uiInputiaHostPreflight=$inputia_state uiSmokeWouldStart=$would_start uiSmokeBlockReason=textedit-already-running uiSmokeBlockReasons=textedit-already-running"
    return
  fi
  if [[ "$safari_state" == "running" && "${INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
    would_start=false
    echo "uiSmokeRequested=true uiTextEditPreflight=$textedit_state uiSafariPreflight=$safari_state uiInputiaHostPreflight=$inputia_state uiSmokeWouldStart=$would_start uiSmokeBlockReason=safari-already-running uiSmokeBlockReasons=safari-already-running"
    return
  fi

  echo "uiSmokeRequested=true uiTextEditPreflight=$textedit_state uiSafariPreflight=$safari_state uiInputiaHostPreflight=$inputia_state uiSmokeWouldStart=$would_start uiSmokeBlockReason=none uiSmokeBlockReasons=none"
}

if [[ "${INPUTIA_AWAIT_UI_STATUS_SELF_CHECK:-0}" == "1" ]]; then
  export INPUTIA_AWAIT_IGNORE_REAL_PROCESSES_FOR_TEST=1
  ready_tis_status="tis.tool=true tis.appSignatureAccepted=true tis.enabledMatches=1 tis.installedMatches=1 tis.hansIconMatchesApp=true tis.hansEnabled=true tis.currentMatchesTarget=true tis.readinessBlockReason=none"
  not_ready_tis_status="tis.tool=true tis.appSignatureAccepted=true tis.enabledMatches=0 tis.installedMatches=0 tis.hansIconMatchesApp=false tis.hansEnabled=unknown tis.currentMatchesTarget=false tis.readinessBlockReason=missing-enabled-source"
  rejected_tis_status="tis.tool=true tis.appSignatureAccepted=false tis.enabledMatches=0 tis.installedMatches=0 tis.hansIconMatchesApp=false tis.hansEnabled=unknown tis.currentMatchesTarget=false tis.readinessBlockReason=signature-rejected"
  INPUTIA_RUN_UI_SMOKE=1 ui_smoke_status_line false "$not_ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=target-and-tis /"
  INPUTIA_RUN_UI_SMOKE=1 INPUTIA_AWAIT_USER_HOST_CONFLICT_FOR_TEST=true ui_smoke_status_line false "$not_ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=target-tis-userhost /"
  INPUTIA_RUN_UI_SMOKE=1 ui_smoke_status_line true "$rejected_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=signature-rejected /"
  for reason in no-console-user gui-bootstrap-unavailable login-not-complete screen-locked frontmost-unavailable loginwindow-frontmost; do
    INPUTIA_RUN_UI_SMOKE=1 INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST="$reason" ui_smoke_status_line true "$ready_tis_status" |
      /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=$reason /"
  done
  INPUTIA_RUN_UI_SMOKE=1 INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=none INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST=TextEdit \
    ui_smoke_status_line true "$ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=textedit-already-running /"
  INPUTIA_RUN_UI_SMOKE=1 INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=none INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST=Safari \
    ui_smoke_status_line true "$ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=safari-already-running /"
  INPUTIA_RUN_UI_SMOKE=1 INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=none INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod \
    ui_smoke_status_line true "$ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=inputia-host-running /"
  INPUTIA_RUN_UI_SMOKE=1 INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=none INPUTIA_AWAIT_USER_HOST_CONFLICT_FOR_TEST=true \
    ui_smoke_status_line true "$ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=user-host-conflict /"
  INPUTIA_RUN_UI_SMOKE=1 INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=none INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST=TextEdit INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 \
    ui_smoke_status_line true "$ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=textedit-allow /"
  INPUTIA_RUN_UI_SMOKE=1 INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=none INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST=Safari INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
    ui_smoke_status_line true "$ready_tis_status" |
    /usr/bin/sed "s/^/awaitUiStatusSelfCheck reason=safari-allow /"
  echo "awaitUiStatusSelfCheck=true"
  exit 0
fi

echo "expectedVersion=$expected_version"
echo "expectedCDHash=$expected_cdhash"
echo "expectedTISModeID=$TARGET_MODE_ID"
echo "expectedTISIcon=$expected_icon"
while (( SECONDS <= deadline )); do
  app_status_line target "$APP"
  if [[ "$APP" != "$SYSTEM_APP" ]]; then
    app_status_line system "$SYSTEM_APP"
  fi
  app_status_line user "$USER_APP"
  echo "userHostConflict=$(user_host_conflict)"
  running_status
  tis_line="$(tis_status_line)"
  echo "$tis_line"
  actual_cdhash="$(cdhash "$APP" || true)"
  if [[ "$actual_cdhash" == "$expected_cdhash" ]]; then
    last_target_matches_build=true
  else
    last_target_matches_build=false
  fi
  ui_smoke_status_line "$last_target_matches_build" "$tis_line"
  last_tis_block_reason="${tis_line##*tis.readinessBlockReason=}"
  last_tis_block_reason="${last_tis_block_reason%% *}"
  if [[ "$actual_cdhash" == "$expected_cdhash" ]] && tis_ready "$tis_line"; then
    echo "systemInstallObserved=true"
    echo "systemInstallTISReady=true"
    "$ROOT_DIR/post-install-regression.sh" "$APP"
    exit 0
  fi
  sleep "$POLL_SECONDS"
done

echo "systemInstallObserved=false reason=timeout"
echo "systemInstallTargetMatchesBuild=$last_target_matches_build"
if [[ "$last_target_matches_build" != "true" ]]; then
  echo "systemInstallTISReady=false reason=target-cdhash-mismatch"
else
  echo "systemInstallTISReady=false reason=$last_tis_block_reason"
fi
"$ROOT_DIR/status.sh"
exit 2
