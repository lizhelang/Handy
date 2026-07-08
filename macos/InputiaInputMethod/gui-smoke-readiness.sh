#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
BUILD_SETTINGS_APP="$ROOT_DIR/build/Inputia 设置.app"
SYSTEM_SETTINGS_APP="/Applications/Inputia 设置.app"
USER_APP="${INPUTIA_USER_APP_FOR_TEST:-$HOME/Library/Input Methods/InputiaInputMethod.app}"
USER_LEGACY_APP="${INPUTIA_USER_LEGACY_APP_FOR_TEST:-$HOME/Library/Input Methods/IputiaInputMethod.app}"
USER_SETTINGS_APP="${INPUTIA_USER_SETTINGS_APP_FOR_TEST:-$HOME/Applications/Inputia 设置.app}"
SYSTEM_APP="${INPUTIA_SYSTEM_APP_FOR_TEST:-/Library/Input Methods/InputiaInputMethod.app}"
DEFAULT_APP="$SYSTEM_APP"
if [[ -d "$USER_APP" ]]; then
  DEFAULT_APP="$USER_APP"
fi
APP="${1:-$DEFAULT_APP}"
TARGET_SETTINGS_APP="$SYSTEM_SETTINGS_APP"
if [[ "$APP" == "$USER_APP" ]]; then
  TARGET_SETTINGS_APP="$USER_SETTINGS_APP"
fi
PKG_PATH="$ROOT_DIR/dist/InputiaInputMethod-latest.pkg"
TIS_READINESS="$ROOT_DIR/tis-readiness.sh"

cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
}

version() {
  if [[ -f "$1/Contents/Info.plist" ]]; then
    /usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$1/Contents/Info.plist" 2>/dev/null || true
  fi
}

sha256() {
  if [[ -f "$1" ]]; then
    /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
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
    printf '%s\n' "$process_check_output"
    return 2
  fi
  return 1
}

process_state() {
  local process_name="$1"
  local process_rc
  if process_pids "$process_name" >/dev/null; then
    echo running
  else
    process_rc=$?
    if [[ "$process_rc" -eq 2 ]]; then
      echo unknown
    else
      echo not-running
    fi
  fi
}

admin_status() {
  if [[ "$APP" == "$USER_APP" ]]; then
    echo "adminInstallReady=true reason=user-scope"
  elif [[ -w "/Library/Input Methods" && -w "/Applications" ]]; then
    echo "adminInstallReady=true reason=writable"
  elif /usr/bin/sudo -n true >/dev/null 2>&1; then
    echo "adminInstallReady=true reason=sudo-noninteractive"
  else
    echo "adminInstallReady=false reason=admin-required"
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

readiness_reason() {
  local pkg_ready="$1"
  local app_matches="$2"
  local settings_matches="$3"
  local tis_ready="$4"
  local gui_block="$5"
  local textedit_state="$6"
  local safari_state="$7"
  local admin_ready="$8"
  local inputia_state="$9"
  local user_host_conflict="${10}"
  local tis_block_reason="${11:-}"

  if [[ "$pkg_ready" != "true" ]]; then
    echo pkg-not-ready
  elif [[ "$tis_block_reason" == "app-missing" ]]; then
    echo app-missing
  elif [[ "$app_matches" != "true" ]]; then
    if [[ "$admin_ready" != "true" ]]; then
      echo admin-required
    else
      echo target-cdhash-mismatch
    fi
  elif [[ "$settings_matches" != "true" ]]; then
    if [[ "$admin_ready" != "true" ]]; then
      echo admin-required
    else
      echo settings-version-mismatch
    fi
  elif [[ "$tis_ready" != "true" && "$tis_block_reason" == "signature-rejected" ]]; then
    echo signature-rejected
  elif [[ "$tis_ready" != "true" ]]; then
    echo tis-not-ready
  elif [[ "$user_host_conflict" == "true" ]]; then
    echo user-host-conflict
  elif [[ "$textedit_state" == "unknown" || "$safari_state" == "unknown" || "$inputia_state" == "unknown" ]]; then
    echo process-list-unavailable
  elif [[ "$inputia_state" == "running" ]]; then
    echo inputia-host-running
  elif [[ "$gui_block" != "none" ]]; then
    echo "$gui_block"
  elif [[ "$textedit_state" == "running" && "${INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
    echo textedit-already-running
  elif [[ "$safari_state" == "running" && "${INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
    echo safari-already-running
  else
    echo none
  fi
}

readiness_block_reasons() {
  local pkg_ready="$1"
  local app_matches="$2"
  local settings_matches="$3"
  local tis_ready="$4"
  local gui_block="$5"
  local textedit_state="$6"
  local safari_state="$7"
  local admin_ready="$8"
  local inputia_state="$9"
  local user_host_conflict="${10}"
  local tis_block_reason="${11:-}"
  local reasons=""

  append_reason() {
    if [[ ",$reasons," == *",$1,"* ]]; then
      return
    fi
    if [[ -z "$reasons" ]]; then
      reasons="$1"
    else
      reasons="$reasons,$1"
    fi
  }

  if [[ "$pkg_ready" != "true" ]]; then
    append_reason pkg-not-ready
  fi
  if [[ "$app_matches" != "true" ]]; then
    if [[ "$tis_block_reason" == "app-missing" ]]; then
      append_reason app-missing
    else
      append_reason target-cdhash-mismatch
    fi
    [[ "$admin_ready" != "true" ]] && append_reason admin-required
  fi
  if [[ "$settings_matches" != "true" ]]; then
    append_reason settings-version-mismatch
    [[ "$admin_ready" != "true" ]] && append_reason admin-required
  fi
  if [[ "$tis_ready" != "true" && "$tis_block_reason" == "signature-rejected" ]]; then
    append_reason signature-rejected
  elif [[ "$tis_ready" != "true" ]]; then
    append_reason tis-not-ready
  fi
  if [[ "$user_host_conflict" == "true" ]]; then
    append_reason user-host-conflict
  fi
  if [[ "$textedit_state" == "unknown" || "$safari_state" == "unknown" || "$inputia_state" == "unknown" ]]; then
    append_reason process-list-unavailable
  fi
  if [[ "$inputia_state" == "running" ]]; then
    append_reason inputia-host-running
  fi
  if [[ "$gui_block" != "none" ]]; then
    append_reason "$gui_block"
  fi
  if [[ "$textedit_state" == "running" && "${INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
    append_reason textedit-already-running
  fi
  if [[ "$safari_state" == "running" && "${INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING:-0}" != "1" ]]; then
    append_reason safari-already-running
  fi

  if [[ -z "$reasons" ]]; then
    echo none
  else
    echo "$reasons"
  fi
}

if [[ "${INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK:-0}" == "1" ]]; then
	  for case_line in \
    "pkg false true true true none not-running not-running true not-running false pkg-not-ready" \
    "admin true false true true none not-running not-running false not-running false admin-required" \
    "target true false true true none not-running not-running true not-running false target-cdhash-mismatch" \
    "settings true true false true none not-running not-running true not-running false settings-version-mismatch" \
    "tis true true true false none not-running not-running true not-running false tis-not-ready" \
    "userhost true true true true none not-running not-running true not-running true user-host-conflict" \
    "inputia true true true true none not-running not-running true running false inputia-host-running" \
    "gui-bootstrap true true true true gui-bootstrap-unavailable not-running not-running true not-running false gui-bootstrap-unavailable" \
    "gui true true true true screen-locked not-running not-running true not-running false screen-locked" \
    "textedit true true true true none running not-running true not-running false textedit-already-running" \
    "safari true true true true none not-running running true not-running false safari-already-running" \
    "ready true true true true none not-running not-running true not-running false none"; do
    set -- ${=case_line}
    label="$1"
    actual="$(readiness_reason "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}" "${11}")"
    expected="${12}"
    echo "guiSmokeReadinessSelfCheck case=$label expected=$expected actual=$actual"
    if [[ "$actual" != "$expected" ]]; then
      echo "guiSmokeReadinessSelfCheck=false case=$label"
      exit 1
    fi
  done
  allow_textedit_reason="$(INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 readiness_reason true true true true none running not-running true not-running false)"
  echo "guiSmokeReadinessSelfCheck case=allow-textedit expected=none actual=$allow_textedit_reason"
  if [[ "$allow_textedit_reason" != "none" ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=allow-textedit"
    exit 1
  fi
  allow_safari_reason="$(INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 readiness_reason true true true true none not-running running true not-running false)"
  echo "guiSmokeReadinessSelfCheck case=allow-safari expected=none actual=$allow_safari_reason"
  if [[ "$allow_safari_reason" != "none" ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=allow-safari"
    exit 1
  fi
  signature_reason="$(readiness_reason true true true false none not-running not-running true not-running false signature-rejected)"
  echo "guiSmokeReadinessSelfCheck case=signature expected=signature-rejected actual=$signature_reason"
  if [[ "$signature_reason" != "signature-rejected" ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=signature"
    exit 1
  fi
  app_missing_reason="$(readiness_reason true false true false none not-running not-running true not-running false app-missing)"
  echo "guiSmokeReadinessSelfCheck case=appmissing expected=app-missing actual=$app_missing_reason"
  if [[ "$app_missing_reason" != "app-missing" ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=appmissing"
    exit 1
  fi
  multi_reasons="$(readiness_block_reasons true true false false none not-running not-running false not-running false)"
  echo "guiSmokeReadinessSelfCheck blockReasons=settings-version-mismatch,admin-required,tis-not-ready actual=$multi_reasons"
  if [[ "$multi_reasons" != *"settings-version-mismatch"* ||
    "$multi_reasons" != *"admin-required"* ||
    "$multi_reasons" != *"tis-not-ready"* ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=block-reasons"
    exit 1
  fi
  all_reasons="$(readiness_block_reasons true false false false screen-locked running running false running true)"
  echo "guiSmokeReadinessSelfCheck allBlockReasons=target-cdhash-mismatch,admin-required,settings-version-mismatch,tis-not-ready,user-host-conflict,inputia-host-running,screen-locked,textedit-already-running,safari-already-running actual=$all_reasons"
  for required_reason in target-cdhash-mismatch admin-required settings-version-mismatch tis-not-ready user-host-conflict inputia-host-running screen-locked textedit-already-running safari-already-running; do
    if [[ "$all_reasons" != *"$required_reason"* ]]; then
      echo "guiSmokeReadinessSelfCheck=false case=all-block-reasons missing=$required_reason"
      exit 1
    fi
  done
  if [[ "$all_reasons" == *"admin-required,admin-required"* ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=all-block-reasons duplicate=admin-required"
    exit 1
  fi
  signature_reasons="$(readiness_block_reasons true true true false none not-running not-running true not-running false signature-rejected)"
  echo "guiSmokeReadinessSelfCheck signatureBlockReasons=signature-rejected actual=$signature_reasons"
  if [[ "$signature_reasons" != "signature-rejected" ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=signature-block-reasons"
    exit 1
  fi
  app_missing_reasons="$(readiness_block_reasons true false true false none not-running not-running true not-running false app-missing)"
  echo "guiSmokeReadinessSelfCheck appMissingBlockReasons=app-missing,tis-not-ready actual=$app_missing_reasons"
  if [[ "$app_missing_reasons" != *"app-missing"* || "$app_missing_reasons" != *"tis-not-ready"* ]]; then
    echo "guiSmokeReadinessSelfCheck=false case=app-missing-block-reasons"
    exit 1
  fi
  echo "guiSmokeReadinessSelfCheck=true"
  exit 0
fi

build_cdhash="$(cdhash "$BUILD_APP")"
target_cdhash="$(cdhash "$APP")"
build_version="$(version "$BUILD_APP")"
target_version="$(version "$APP")"
build_settings_version="$(version "$BUILD_SETTINGS_APP")"
target_settings_version="$(version "$TARGET_SETTINGS_APP")"
pkg_sha="$(sha256 "$PKG_PATH")"

echo "build.exists=$([[ -d "$BUILD_APP" ]] && echo true || echo false)"
echo "build.version=$build_version"
echo "build.cdhash=$build_cdhash"
echo "target.path=$APP"
echo "target.exists=$([[ -d "$APP" ]] && echo true || echo false)"
echo "target.version=$target_version"
echo "target.cdhash=$target_cdhash"
if [[ -n "$build_cdhash" && "$target_cdhash" == "$build_cdhash" ]]; then
  target_matches=true
else
  target_matches=false
fi
echo "target.matchesBuild=$target_matches"
echo "settings.buildVersion=$build_settings_version"
echo "settings.targetPath=$TARGET_SETTINGS_APP"
echo "settings.targetVersion=$target_settings_version"
if [[ -n "$build_settings_version" && "$target_settings_version" == "$build_settings_version" ]]; then
  settings_matches=true
else
  settings_matches=false
fi
echo "settings.matchesBuild=$settings_matches"
echo "pkg.path=$PKG_PATH"
echo "pkg.exists=$([[ -f "$PKG_PATH" ]] && echo true || echo false)"
echo "pkg.sha256=$pkg_sha"
if pkg_output="$("$ROOT_DIR/verify-pkg.sh" "$PKG_PATH" 2>&1)" &&
  /usr/bin/grep -q '^pkgVerificationPassed=true$' <<<"$pkg_output"; then
  pkg_ready=true
else
  pkg_ready=false
fi
echo "pkg.ready=$pkg_ready"

tis_output="$("$TIS_READINESS" "$APP" 2>&1 || true)"
printf '%s\n' "$tis_output" | /usr/bin/sed 's/^/tis: /'
if /usr/bin/grep -q '^tisReadiness=true$' <<<"$tis_output"; then
  tis_ready=true
else
  tis_ready=false
fi
tis_block_reason="$(/usr/bin/awk -F= '$1 == "tis.readinessBlockReason" { print $2; found = 1; exit } END { if (!found) print "none" }' <<<"$tis_output")"
echo "tis.ready=$tis_ready"
echo "tis.blockReason=$tis_block_reason"

admin_line="$(admin_status)"
echo "$admin_line"
admin_ready=false
if [[ "$admin_line" == adminInstallReady=true* ]]; then
  admin_ready=true
fi
gui_block="$(gui_session_block_reason)"
textedit_state="$(process_state TextEdit)"
safari_state="$(process_state Safari)"
inputia_state="$(process_state InputiaInputMethod)"
echo "guiSessionBlockReason=$gui_block"
echo "textEditPreflight=$textedit_state"
echo "safariPreflight=$safari_state"
echo "inputiaHostPreflight=$inputia_state"
if [[ -e "$USER_LEGACY_APP" ]]; then
  user_host_conflict=true
elif [[ -e "$USER_APP" && -d "$SYSTEM_APP" ]]; then
  user_host_conflict=true
else
  user_host_conflict=false
fi
echo "userHostConflict=$user_host_conflict"

reason="$(readiness_reason "$pkg_ready" "$target_matches" "$settings_matches" "$tis_ready" "$gui_block" "$textedit_state" "$safari_state" "$admin_ready" "$inputia_state" "$user_host_conflict" "$tis_block_reason")"
block_reasons="$(readiness_block_reasons "$pkg_ready" "$target_matches" "$settings_matches" "$tis_ready" "$gui_block" "$textedit_state" "$safari_state" "$admin_ready" "$inputia_state" "$user_host_conflict" "$tis_block_reason")"
echo "guiSmokeReadinessBlockReasons=$block_reasons"
if [[ "$reason" == "none" ]]; then
  echo "guiSmokeReadinessReady=true reason=none"
else
  echo "guiSmokeReadinessReady=false reason=$reason"
fi
