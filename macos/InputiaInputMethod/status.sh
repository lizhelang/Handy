#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
SYSTEM_APP="/Library/Input Methods/InputiaInputMethod.app"
USER_APP="${INPUTIA_USER_APP_FOR_TEST:-$HOME/Library/Input Methods/InputiaInputMethod.app}"
USER_LEGACY_APP="${INPUTIA_USER_LEGACY_APP_FOR_TEST:-$HOME/Library/Input Methods/IputiaInputMethod.app}"
LEGACY_APP="/Library/Input Methods/IputiaInputMethod.app"
SYSTEM_SETTINGS_APP="/Applications/Inputia 设置.app"
USER_SETTINGS_APP="${INPUTIA_USER_SETTINGS_APP_FOR_TEST:-$HOME/Applications/Inputia 设置.app}"
LATEST_PKG="$ROOT_DIR/dist/InputiaInputMethod-latest.pkg"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
TARGET_APP="${INPUTIA_STATUS_APP:-}"
if [[ -z "$TARGET_APP" ]]; then
  if [[ -d "$USER_APP" ]]; then
    TARGET_APP="$USER_APP"
  else
    TARGET_APP="$SYSTEM_APP"
  fi
fi
TARGET_SETTINGS_APP="$SYSTEM_SETTINGS_APP"
if [[ "$TARGET_APP" == "$USER_APP" ]]; then
  TARGET_SETTINGS_APP="$USER_SETTINGS_APP"
fi

section() {
  printf '\n== %s ==\n' "$1"
}

plist_value() {
  local plist="$1"
  local key="$2"
  if [[ -f "$plist" ]]; then
    /usr/libexec/PlistBuddy -c "Print :$key" "$plist" 2>/dev/null || true
  fi
  return 0
}

app_version() {
  plist_value "$1/Contents/Info.plist" CFBundleVersion
}

app_short_version() {
  plist_value "$1/Contents/Info.plist" CFBundleShortVersionString
}

app_bundle_id() {
  plist_value "$1/Contents/Info.plist" CFBundleIdentifier
}

app_cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
  return 0
}

app_assessment() {
  if [[ -d "$1" ]]; then
    /usr/sbin/spctl --assess --type execute --verbose=4 "$1" 2>&1 || true
  fi
}

app_top_level_tis_source_id() {
  plist_value "$1/Contents/Info.plist" TISInputSourceID
}

print_app() {
  local label="$1"
  local path="$2"

  section "$label"
  echo "path=$path"
  if [[ ! -d "$path" ]]; then
    echo "exists=false"
    return
  fi

  echo "exists=true"
  if [[ ! -f "$path/Contents/Info.plist" ]]; then
    echo "validBundle=false"
    echo "reason=missing-info-plist"
    return
  fi
  echo "validBundle=true"
  echo "bundleID=$(app_bundle_id "$path")"
  echo "version=$(app_version "$path")"
  echo "shortVersion=$(app_short_version "$path")"
  top_level_tis_source_id="$(app_top_level_tis_source_id "$path")"
  if [[ -n "$top_level_tis_source_id" ]]; then
    echo "topLevelTISInputSourceID=$top_level_tis_source_id"
  else
    echo "topLevelTISInputSourceID=absent"
  fi
  echo "cdhash=$(app_cdhash "$path")"
  assessment="$(app_assessment "$path")"
  if [[ -n "$assessment" ]]; then
    echo "assessment=$assessment"
  fi
  if [[ -x "$path/Contents/MacOS/InputiaInputMethod" ]]; then
    echo "hostExecutable=true"
  elif [[ -x "$path/Contents/MacOS/InputiaSettingsLauncher" ]]; then
    echo "settingsLauncherExecutable=true"
  else
    echo "knownExecutable=false"
  fi
}

matches_build() {
  local path="$1"
  local build_cdhash="$2"
  local actual_cdhash
  actual_cdhash="$(app_cdhash "$path")"
  [[ -n "$build_cdhash" && "$actual_cdhash" == "$build_cdhash" ]]
}

append_reason() {
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

admin_install_ready() {
  if [[ -w "/Library/Input Methods" && -w "/Applications" ]]; then
    echo true
  elif /usr/bin/sudo -n true >/dev/null 2>&1; then
    echo true
  else
    echo false
  fi
}

current_user_directory_status() {
  /usr/bin/python3 <<'PY'
import os
import pwd

uid = os.getuid()
print(f"statusCurrentUID={uid}")
try:
    record = pwd.getpwuid(uid)
except KeyError:
    print("statusCurrentUserName=unknown")
    print("statusUserDirectoryReady=false")
    print("statusUserDirectoryBlockReason=missing-passwd-record")
else:
    print(f"statusCurrentUserName={record.pw_name}")
    print("statusUserDirectoryReady=true")
    print("statusUserDirectoryBlockReason=none")
PY
}

hitoolbox_preferences_status() {
  local output rc
  set +e
  output="$(/usr/bin/defaults read com.apple.HIToolbox 2>&1 >/dev/null)"
  rc=$?
  set -e
  if [[ "$rc" -eq 0 ]]; then
    echo "statusHIToolboxDefaultsReadable=true"
    echo "statusHIToolboxDefaultsBlockReason=none"
  elif [[ "$output" == *"Domain com.apple.HIToolbox does not exist"* ]]; then
    echo "statusHIToolboxDefaultsReadable=false"
    echo "statusHIToolboxDefaultsBlockReason=domain-missing"
  else
    echo "statusHIToolboxDefaultsReadable=false"
    echo "statusHIToolboxDefaultsBlockReason=defaults-read-failed"
  fi
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
  local process_check_output process_check_rc ps_rc
  PROCESS_LIST_OUTPUT=""
  set +e
  process_check_output="$(/usr/bin/pgrep -x "$process_name" 2>&1)"
  process_check_rc=$?
  set -e
  if [[ "$process_check_rc" -eq 0 ]]; then
    printf '%s\n' "$process_check_output"
    return 0
  fi
  set +e
  process_pids_by_ps "$process_name"
  ps_rc=$?
  set -e
  if [[ "$ps_rc" -eq 0 || "$ps_rc" -eq 1 ]]; then
    return "$ps_rc"
  fi
  if [[ -n "$process_check_output" ]]; then
    PROCESS_LIST_OUTPUT="$process_check_output"
    printf '%s\n' "$process_check_output"
    return 2
  fi
  return 1
}

process_state() {
  local process_name="$1"
  local pids_rc
  if process_pids "$process_name" >/dev/null; then
    echo running
  else
    pids_rc=$?
    if [[ "$pids_rc" -eq 2 ]]; then
      echo unknown
    else
      echo not-running
    fi
  fi
}

gui_session_block_reason() {
  if [[ -n "${INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST:-}" ]]; then
    echo "$INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST"
    return
  fi

  if [[ "${INPUTIA_SKIP_GUI_SESSION_CHECK:-0}" == "1" ]]; then
    echo none
    return
  fi

  local console_user
  console_user="$(/usr/bin/stat -f '%Su' /dev/console 2>/dev/null || true)"
  if [[ -z "$console_user" || "$console_user" == "root" || "$console_user" == "_mbsetupuser" ]]; then
    echo no-console-user
    return
  fi
  local console_uid
  console_uid="$(/usr/bin/stat -f '%u' /dev/console 2>/dev/null || true)"
  if [[ -z "$console_uid" ]] || ! /bin/launchctl print "gui/$console_uid" >/dev/null 2>&1; then
    echo gui-bootstrap-unavailable
    return
  fi

  local session_state
  session_state="$(/usr/sbin/ioreg -n Root -d1 2>/dev/null || true)"
  if [[ "$session_state" != *"kCGSessionLoginDoneKey\"=Yes"* ]]; then
    echo login-not-complete
    return
  fi
  if [[ "$session_state" == *"CGSSessionScreenIsLocked\"=Yes"* ||
    "$session_state" == *"kCGSSessionScreenIsLocked\"=Yes"* ]]; then
    echo screen-locked
    return
  fi

  local frontmost_app
  if ! frontmost_app="$(/usr/bin/osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null)"; then
    echo frontmost-unavailable
    return
  fi
  if [[ "$frontmost_app" == "loginwindow" ]]; then
    echo loginwindow-frontmost
    return
  fi
  echo none
}

section "expected build"
build_version="$(app_version "$BUILD_APP")"
build_cdhash="$(app_cdhash "$BUILD_APP")"
print_app "expected build app" "$BUILD_APP"
echo "buildVersion=$build_version"
echo "buildCDHash=$build_cdhash"

print_app "system host" "$SYSTEM_APP"
system_assessment="$(app_assessment "$SYSTEM_APP")"
if [[ "$system_assessment" == *": accepted"* ]]; then
  system_signature_accepted=true
else
  system_signature_accepted=false
fi
if matches_build "$SYSTEM_APP" "$build_cdhash"; then
  system_matches_build=true
  echo "systemMatchesBuild=true"
else
  system_matches_build=false
  echo "systemMatchesBuild=false"
fi

print_app "user host" "$USER_APP"
if matches_build "$USER_APP" "$build_cdhash"; then
  user_matches_build=true
  echo "userMatchesBuild=true"
else
  user_matches_build=false
  echo "userMatchesBuild=false"
fi
if [[ -e "$USER_LEGACY_APP" ]]; then
  user_host_conflict=true
elif [[ -e "$USER_APP" && -e "$SYSTEM_APP" ]]; then
  user_host_conflict=true
else
  user_host_conflict=false
fi
echo "userHostConflict=$user_host_conflict"

section "legacy typo host"
echo "path=$LEGACY_APP"
echo "legacyIputiaPresent=$([[ -e "$LEGACY_APP" ]] && echo true || echo false)"

print_app "system settings launcher" "$SYSTEM_SETTINGS_APP"
settings_version="$(app_version "$SYSTEM_SETTINGS_APP")"
if [[ -n "$build_version" && "$settings_version" == "$build_version" ]]; then
  system_settings_matches_build=true
  echo "systemSettingsMatchesBuildVersion=true"
else
  system_settings_matches_build=false
  echo "systemSettingsMatchesBuildVersion=false"
fi

print_app "user settings launcher" "$USER_SETTINGS_APP"
user_settings_version="$(app_version "$USER_SETTINGS_APP")"
if [[ -n "$build_version" && "$user_settings_version" == "$build_version" ]]; then
  user_settings_matches_build=true
  echo "userSettingsMatchesBuildVersion=true"
else
  user_settings_matches_build=false
  echo "userSettingsMatchesBuildVersion=false"
fi

section "target host"
echo "targetPath=$TARGET_APP"
if [[ "$TARGET_APP" == "$USER_APP" ]]; then
  echo "targetScope=user"
elif [[ "$TARGET_APP" == "$SYSTEM_APP" ]]; then
  echo "targetScope=system"
else
  echo "targetScope=custom"
fi
print_app "target app" "$TARGET_APP"
target_assessment="$(app_assessment "$TARGET_APP")"
if [[ "$target_assessment" == *": accepted"* ]]; then
  target_signature_accepted=true
else
  target_signature_accepted=false
fi
if matches_build "$TARGET_APP" "$build_cdhash"; then
  target_matches_build=true
  echo "targetMatchesBuild=true"
else
  target_matches_build=false
  echo "targetMatchesBuild=false"
fi
echo "targetSettingsPath=$TARGET_SETTINGS_APP"
target_settings_version="$(app_version "$TARGET_SETTINGS_APP")"
if [[ -n "$build_version" && "$target_settings_version" == "$build_version" ]]; then
  target_settings_matches_build=true
  echo "targetSettingsMatchesBuildVersion=true"
else
  target_settings_matches_build=false
  echo "targetSettingsMatchesBuildVersion=false"
fi
if [[ "$TARGET_APP" == "$SYSTEM_APP" || "$TARGET_SETTINGS_APP" == "$SYSTEM_SETTINGS_APP" ]]; then
  target_requires_admin=true
else
  target_requires_admin=false
fi
echo "targetRequiresAdmin=$target_requires_admin"

section "tis sources"
tis_enabled_matches=unknown
tis_installed_matches=unknown
if [[ -x "$TIS_TOOL" ]]; then
  tis_dump="$(INPUTIA_APP="$TARGET_APP" "$TIS_TOOL" --dump 2>/dev/null || true)"
  tis_enabled_matches="$(/usr/bin/awk -F= '
    $1 == "includeAllInstalled" { active = ($2 == "false") }
    active && $1 == "matches" { print $2; exit }
  ' <<<"$tis_dump")"
  tis_installed_matches="$(/usr/bin/awk -F= '
    $1 == "includeAllInstalled" { active = ($2 == "true") }
    active && $1 == "matches" { print $2; exit }
  ' <<<"$tis_dump")"
  tis_enabled_matches="${tis_enabled_matches:-unknown}"
  tis_installed_matches="${tis_installed_matches:-unknown}"
  printf '%s\n' "$tis_dump" | /usr/bin/awk '
    /^includeAllInstalled=/{include=$0; print; next}
    /^matches=/{print; next}
    /^id=|^bundle=|^mode=|^name=|^iconURL=|^languages=|^enabled=|^selectable=|^selected=/{print; next}
  '
else
  echo "tisToolPresent=false path=$TIS_TOOL"
fi

section "menu readiness"
menu_readiness=unknown
menu_block_reason=unknown
if [[ -x "$ROOT_DIR/menu-readiness.sh" ]]; then
  menu_output="$("$ROOT_DIR/menu-readiness.sh" 2>&1 || true)"
  printf '%s\n' "$menu_output"
  menu_readiness="$(/usr/bin/awk -F= '$1 == "menuReadiness" { print $2; exit }' <<<"$menu_output")"
  menu_block_reason="$(/usr/bin/awk -F= '$1 == "menuReadinessBlockReason" { print $2; exit }' <<<"$menu_output")"
  menu_readiness="${menu_readiness:-unknown}"
  menu_block_reason="${menu_block_reason:-unknown}"
else
  echo "menuReadiness=false"
  echo "menuReadinessBlockReason=missing-menu-readiness-script"
  menu_readiness=false
  menu_block_reason=missing-menu-readiness-script
fi

section "running host"
set +e
running_pids_output="$(process_pids InputiaInputMethod 2>&1)"
running_pids_rc=$?
set -e
if [[ "$running_pids_rc" -eq 0 ]]; then
  running_pids="$running_pids_output"
elif [[ "$running_pids_rc" -eq 2 ]]; then
  running_pids=""
  inputia_running=unknown
  echo "running=unknown"
  echo "processListAvailable=false reason=process-list-unavailable"
  printf '%s\n' "$running_pids_output" | /usr/bin/sed 's/^/processListOutput: /'
else
  running_pids=""
fi
if [[ "${inputia_running:-}" != "unknown" && -z "$running_pids" ]]; then
  inputia_running=false
  echo "running=false"
elif [[ "${inputia_running:-}" != "unknown" ]]; then
  inputia_running=true
  echo "running=true"
  while IFS= read -r pid; do
    [[ -z "$pid" ]] && continue
    command="$(/bin/ps -p "$pid" -o command= 2>/dev/null || true)"
    echo "pid=$pid"
    echo "command=$command"
    running_app="unknown"
    if [[ "$command" == "$SYSTEM_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
      running_app="$SYSTEM_APP"
    elif [[ "$command" == "$USER_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
      running_app="$USER_APP"
    elif [[ "$command" == "$BUILD_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
      running_app="$BUILD_APP"
    fi
    echo "runningApp=$running_app"
    if [[ "$running_app" != "unknown" ]]; then
      echo "runningVersion=$(app_version "$running_app")"
      echo "runningCDHash=$(app_cdhash "$running_app")"
      if matches_build "$running_app" "$build_cdhash"; then
        echo "runningMatchesBuild=true"
      else
        echo "runningMatchesBuild=false"
      fi
    fi
  done <<<"$running_pids"
fi

section "latest package"
echo "path=$LATEST_PKG"
if [[ -f "$LATEST_PKG" ]]; then
  latest_pkg_exists=true
  echo "exists=true"
  echo "sha256=$(/usr/bin/shasum -a 256 "$LATEST_PKG" | /usr/bin/awk '{print $1}')"
  echo "sizeBytes=$(/usr/bin/stat -f%z "$LATEST_PKG")"
  /usr/sbin/pkgutil --check-signature "$LATEST_PKG" >/dev/null 2>&1 &&
    echo "pkgSignature=present" ||
    echo "pkgSignature=none"
else
  latest_pkg_exists=false
  echo "exists=false"
fi

section "user directory"
user_directory_output="$(current_user_directory_status)"
printf '%s\n' "$user_directory_output"
user_directory_ready="$(/usr/bin/awk -F= '$1 == "statusUserDirectoryReady" { print $2; exit }' <<<"$user_directory_output")"
user_directory_block_reason="$(/usr/bin/awk -F= '$1 == "statusUserDirectoryBlockReason" { print $2; exit }' <<<"$user_directory_output")"
user_directory_ready="${user_directory_ready:-unknown}"
user_directory_block_reason="${user_directory_block_reason:-unknown}"

hitoolbox_defaults_output="$(hitoolbox_preferences_status)"
printf '%s\n' "$hitoolbox_defaults_output"
hitoolbox_defaults_readable="$(/usr/bin/awk -F= '$1 == "statusHIToolboxDefaultsReadable" { print $2; exit }' <<<"$hitoolbox_defaults_output")"
hitoolbox_defaults_block_reason="$(/usr/bin/awk -F= '$1 == "statusHIToolboxDefaultsBlockReason" { print $2; exit }' <<<"$hitoolbox_defaults_output")"
hitoolbox_defaults_readable="${hitoolbox_defaults_readable:-unknown}"
hitoolbox_defaults_block_reason="${hitoolbox_defaults_block_reason:-unknown}"

section "gui smoke summary"
admin_ready="$(admin_install_ready)"
gui_block_reason="$(gui_session_block_reason)"
textedit_state="$(process_state TextEdit)"
safari_state="$(process_state Safari)"
if [[ "$inputia_running" == "true" ]]; then
  inputia_state=running
elif [[ "$inputia_running" == "unknown" ]]; then
  inputia_state=unknown
else
  inputia_state=not-running
fi
block_reasons=""
if [[ "$latest_pkg_exists" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" pkg-not-ready)"
fi
if [[ "$target_matches_build" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" target-cdhash-mismatch)"
  [[ "$target_requires_admin" == "true" && "$admin_ready" != "true" ]] &&
    block_reasons="$(append_reason "$block_reasons" admin-required)"
fi
if [[ "$target_settings_matches_build" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" settings-version-mismatch)"
  [[ "$target_requires_admin" == "true" && "$admin_ready" != "true" ]] &&
    block_reasons="$(append_reason "$block_reasons" admin-required)"
fi
if [[ "$tis_enabled_matches" == "0" || "$tis_enabled_matches" == "unknown" ]]; then
  block_reasons="$(append_reason "$block_reasons" tis-not-ready)"
fi
if [[ "$user_directory_ready" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" user-directory-unavailable)"
fi
if [[ "$hitoolbox_defaults_readable" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" hitoolbox-preferences-unavailable)"
fi
if [[ "$target_signature_accepted" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" signature-rejected)"
fi
if [[ "$menu_readiness" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" "menu-$menu_block_reason")"
fi
if [[ "$user_host_conflict" == "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" user-host-conflict)"
fi
if [[ "$inputia_running" == "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" inputia-host-running)"
fi
if [[ "$textedit_state" == "unknown" || "$safari_state" == "unknown" || "$inputia_state" == "unknown" ]]; then
  block_reasons="$(append_reason "$block_reasons" process-list-unavailable)"
fi
if [[ "$gui_block_reason" != "none" ]]; then
  block_reasons="$(append_reason "$block_reasons" "$gui_block_reason")"
fi
if [[ "$textedit_state" == "running" && "${INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
  block_reasons="$(append_reason "$block_reasons" textedit-already-running)"
fi
if [[ "$safari_state" == "running" && "${INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
  block_reasons="$(append_reason "$block_reasons" safari-already-running)"
fi
if [[ -z "$block_reasons" ]]; then
  block_reasons=none
fi
echo "statusAdminInstallReady=$admin_ready"
echo "statusTargetPath=$TARGET_APP"
echo "statusTargetMatchesBuild=$target_matches_build"
echo "statusTISEnabledMatches=$tis_enabled_matches"
echo "statusTISInstalledMatches=$tis_installed_matches"
echo "statusSignatureAccepted=$target_signature_accepted"
if [[ "$target_signature_accepted" != "true" ]]; then
  echo "statusSigningRequiredAction=sign-with-accepted-identity"
fi
echo "statusMenuReadiness=$menu_readiness"
echo "statusMenuBlockReason=$menu_block_reason"
echo "statusUserHostConflict=$user_host_conflict"
echo "statusGuiSessionBlockReason=$gui_block_reason"
echo "statusTextEditPreflight=$textedit_state"
echo "statusSafariPreflight=$safari_state"
echo "statusInputiaHostPreflight=$inputia_state"
echo "statusGuiSmokeBlockReasons=$block_reasons"
if [[ "$block_reasons" == "none" ]]; then
  echo "statusGuiSmokeReady=true reason=none"
else
  echo "statusGuiSmokeReady=false reason=$block_reasons"
fi
