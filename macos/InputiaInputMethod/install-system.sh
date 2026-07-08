#!/bin/zsh
set -eu
set -o pipefail
umask 022

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_NAME="InputiaInputMethod.app"
SETTINGS_APP_NAME="Inputia 设置.app"
SOURCE_APP="$ROOT_DIR/build/$APP_NAME"
SOURCE_SETTINGS_APP="$ROOT_DIR/build/$SETTINGS_APP_NAME"
DEST_DIR="/Library/Input Methods"
DEST_APP="$DEST_DIR/$APP_NAME"
LEGACY_APP="$DEST_DIR/IputiaInputMethod.app"
DEST_SETTINGS_APP="/Applications/$SETTINGS_APP_NAME"
LOGIN_USER="$(/usr/bin/stat -f%Su /dev/console)"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
USER_HOME=""
USER_APP=""
USER_LEGACY_APP=""

if [[ -n "$LOGIN_USER" && "$LOGIN_USER" != "root" && "$LOGIN_USER" != "_mbsetupuser" ]]; then
  USER_HOME="$(/usr/bin/dscl . -read "/Users/$LOGIN_USER" NFSHomeDirectory 2>/dev/null | /usr/bin/awk '{print $2}' || true)"
  if [[ -n "$USER_HOME" && "$USER_HOME" == /* ]]; then
    USER_APP="$USER_HOME/Library/Input Methods/$APP_NAME"
    USER_LEGACY_APP="$USER_HOME/Library/Input Methods/IputiaInputMethod.app"
  fi
fi

detect_verification_processes() {
  local process_list
  if [[ -n "${INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST:-}" ]]; then
    process_list="$INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST"
  else
    process_list="$(/bin/ps -axo pid=,command=)"
  fi
  printf '%s\n' "$process_list" |
    /usr/bin/awk -v root="$ROOT_DIR" -v self="$$" '
      $1 == self { next }
      index($0, root) &&
        $0 ~ /\/(verify-nongui|post-install-regression|verify-system|verify-pkg|await-system-install|smoke-preflight|smoke-textedit|smoke-textedit-command-shortcuts|smoke-clipboard-recall|smoke-safari[^ ]*|diagnose-safari-input-source|gui-smoke-readiness|gui-smoke-suite|status|tis-readiness)\.sh( |$)/ {
          print
        }
    '
}

require_no_verification_processes() {
  local blocking_processes
  blocking_processes="$(detect_verification_processes)"
  if [[ -n "$blocking_processes" ]]; then
    echo "systemInstallReady=false reason=verification-running"
    printf '%s\n' "$blocking_processes" | /usr/bin/sed 's/^/systemInstallBlockingProcess: /'
    exit 20
  fi
}

if [[ "${INPUTIA_INSTALL_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  original_process_list="${INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST:-}"
  INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="123 /usr/bin/true"
  clear_processes="$(detect_verification_processes)"
  INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/smoke-textedit.sh"
  blocked_processes="$(detect_verification_processes)"
  INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="$original_process_list"
  if [[ -z "$clear_processes" && -n "$blocked_processes" ]]; then
    echo "systemInstallPreflightSelfCheck clear=true"
    echo "systemInstallPreflightSelfCheck blocked=true"
    echo "systemInstallPreflightSelfCheck=true"
    exit 0
  fi
  echo "systemInstallPreflightSelfCheck=false"
  exit 1
fi

require_no_verification_processes

current_user_directory_status() {
  /usr/bin/python3 <<'PY'
import os
import pwd

uid = os.getuid()
print(f"systemInstallCurrentUID={uid}")
try:
    record = pwd.getpwuid(uid)
except KeyError:
    print("systemInstallCurrentUserName=unknown")
    print("systemInstallUserDirectoryReady=false")
    print("systemInstallUserDirectoryBlockReason=missing-passwd-record")
else:
    print(f"systemInstallCurrentUserName={record.pw_name}")
    print("systemInstallUserDirectoryReady=true")
    print("systemInstallUserDirectoryBlockReason=none")
PY
}

require_admin_channel_if_needed() {
  if [[ -w "$DEST_DIR" && -w "/Applications" ]]; then
    echo "systemInstallAdminChannelReady=true reason=writable"
    return
  fi

  echo "systemInstallNeedsAdmin=true"

  local user_directory_output user_directory_ready user_directory_block_reason
  user_directory_output="$(current_user_directory_status)"
  printf '%s\n' "$user_directory_output"
  user_directory_ready="$(/usr/bin/awk -F= '$1 == "systemInstallUserDirectoryReady" { print $2; exit }' <<<"$user_directory_output")"
  user_directory_block_reason="$(/usr/bin/awk -F= '$1 == "systemInstallUserDirectoryBlockReason" { print $2; exit }' <<<"$user_directory_output")"

  if [[ "$user_directory_ready" != "true" ]]; then
    echo "systemInstallAdminChannelReady=false reason=user-directory-unavailable"
    echo "systemInstallReady=false reason=user-directory-unavailable"
    echo "systemInstallBlockReason=${user_directory_block_reason:-unknown}"
    echo "systemInstallRequiredAction=repair-current-user-directory-service"
    exit 13
  fi

  if [[ "${INPUTIA_INSTALL_NO_ADMIN_PROMPT:-0}" == "1" ]]; then
    if /usr/bin/sudo -n true >/dev/null 2>&1; then
      echo "systemInstallAdminChannelReady=true reason=sudo-noninteractive"
      return
    fi
    echo "systemInstallAdminChannelReady=false reason=admin-required"
    echo "systemInstallReady=false reason=admin-required"
    exit 12
  fi

  if [[ -n "${INPUTIA_SUDO_PASSWORD:-}" ]]; then
    if /usr/bin/printf '%s\n' "$INPUTIA_SUDO_PASSWORD" | /usr/bin/sudo -S -p '' true >/dev/null 2>&1; then
      echo "systemInstallAdminChannelReady=true reason=sudo-password"
      return
    fi
    echo "systemInstallAdminChannelReady=false reason=sudo-password-rejected"
    echo "systemInstallReady=false reason=admin-required"
    exit 12
  fi

  echo "systemInstallAdminChannelReady=unknown reason=osascript-administrator-privileges-prompt"
}

require_admin_channel_if_needed

run_with_timeout() {
  local timeout_seconds="$1"
  local label="$2"
  shift 2

  "$@" &
  local command_pid=$!

  (
    /bin/sleep "$timeout_seconds"
    if /bin/kill -0 "$command_pid" >/dev/null 2>&1; then
      echo "inputiaInstallTimeout=$label seconds=$timeout_seconds" >&2
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

run_best_effort() {
  local timeout_seconds="$1"
  local label="$2"
  shift 2
  run_with_timeout "$timeout_seconds" "$label" "$@" || true
}

run_login_best_effort() {
  local timeout_seconds="$1"
  local label="$2"
  shift 2

  if [[ -n "$LOGIN_USER" && "$LOGIN_USER" != "root" && "$(/usr/bin/id -un)" != "$LOGIN_USER" ]]; then
    if [[ -n "$USER_HOME" ]]; then
      run_best_effort "$timeout_seconds" "$label" /usr/bin/sudo -u "$LOGIN_USER" /usr/bin/env \
        HOME="$USER_HOME" USER="$LOGIN_USER" LOGNAME="$LOGIN_USER" "$@"
    else
      run_best_effort "$timeout_seconds" "$label" /usr/bin/sudo -u "$LOGIN_USER" "$@"
    fi
  else
    run_best_effort "$timeout_seconds" "$label" "$@"
  fi
}

run_login_capture() {
  if [[ -n "$LOGIN_USER" && "$LOGIN_USER" != "root" && "$(/usr/bin/id -un)" != "$LOGIN_USER" ]]; then
    if [[ -n "$USER_HOME" ]]; then
      /usr/bin/sudo -u "$LOGIN_USER" /usr/bin/env HOME="$USER_HOME" USER="$LOGIN_USER" LOGNAME="$LOGIN_USER" "$@" 2>&1 || true
    else
      /usr/bin/sudo -u "$LOGIN_USER" "$@" 2>&1 || true
    fi
  else
    "$@" 2>&1 || true
  fi
}

run_admin_copy_with_password() {
  /usr/bin/printf '%s\n' "$INPUTIA_SUDO_PASSWORD" |
    /usr/bin/sudo -S -p '' /bin/zsh -c "$copy_command"
}

cleanup_user_inputia_residue() {
  if [[ -z "$USER_HOME" ]]; then
    echo "userHostBackupRemoved=skipped"
    return
  fi

  local input_methods_dir="$USER_HOME/Library/Input Methods"
  if [[ ! -d "$input_methods_dir" ]]; then
    echo "userHostBackupRemoved=false"
    return
  fi

  local removed_paths
  removed_paths="$(/usr/bin/find "$input_methods_dir" -maxdepth 1 \
    \( -name 'InputiaInputMethod.app.inputia-smoke-backup-*' -o -name 'IputiaInputMethod.app.inputia-smoke-backup-*' \) \
    -print 2>/dev/null || true)"
  if [[ -z "$removed_paths" ]]; then
    echo "userHostBackupRemoved=false"
    return
  fi

  while IFS= read -r backup_path; do
    [[ -z "$backup_path" ]] && continue
    /bin/rm -rf "$backup_path"
    echo "userHostBackupRemoved=true path=$backup_path"
  done <<<"$removed_paths"
}

/bin/zsh "$ROOT_DIR/build.sh" >/dev/null

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

assess_app() {
  local app="$1"
  /usr/sbin/spctl --assess --type execute --verbose=4 "$app" 2>&1
}

source_cdhash="$(cdhash "$SOURCE_APP")"
source_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$SOURCE_APP/Contents/Info.plist")"
echo "sourceVersion=$source_version"
echo "sourceCDHash=$source_cdhash"

copy_command=$(/usr/bin/python3 - "$SOURCE_APP" "$DEST_APP" "$LEGACY_APP" "$SOURCE_SETTINGS_APP" "$DEST_SETTINGS_APP" <<'PY'
import shlex
import sys

source, dest, legacy, source_settings, dest_settings = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
print(
    "rm -rf {dest} {legacy} {dest_settings} && "
    "/usr/bin/ditto --noextattr --noqtn {source} {dest} && "
    "/usr/bin/ditto --noextattr --noqtn {source_settings} {dest_settings} && "
    "/usr/sbin/chown -R root:wheel {dest} {dest_settings} && "
    "/usr/bin/find {dest} {dest_settings} -type d -exec /bin/chmod 755 {{}} + && "
    "/usr/bin/find {dest} {dest_settings} -type f -exec /bin/chmod 644 {{}} + && "
    "/bin/chmod 755 {dest}/Contents/MacOS/InputiaInputMethod {dest_settings}/Contents/MacOS/InputiaSettingsLauncher".format(
        source=shlex.quote(source),
        dest=shlex.quote(dest),
        legacy=shlex.quote(legacy),
        source_settings=shlex.quote(source_settings),
        dest_settings=shlex.quote(dest_settings),
    )
)
PY
)

/usr/bin/killall InputiaInputMethod >/dev/null 2>&1 || true
/usr/bin/killall IputiaInputMethod >/dev/null 2>&1 || true
"$LSREGISTER" -u "$LEGACY_APP" >/dev/null 2>&1 || true
if [[ -n "$USER_APP" ]]; then
  "$LSREGISTER" -u "$USER_APP" >/dev/null 2>&1 || true
  "$LSREGISTER" -u "$USER_LEGACY_APP" >/dev/null 2>&1 || true
  /bin/rm -rf "$USER_APP" "$USER_LEGACY_APP"
  echo "userHostRemoved=true path=$USER_APP"
else
  echo "userHostRemoved=skipped"
fi
cleanup_user_inputia_residue

if [[ -w "$DEST_DIR" && -w "/Applications" ]]; then
  /bin/zsh -c "$copy_command"
else
  echo "systemInstallNeedsAdmin=true"
  if [[ "${INPUTIA_INSTALL_NO_ADMIN_PROMPT:-0}" == "1" ]]; then
    run_with_timeout 120 admin-copy /usr/bin/sudo -n /bin/zsh -c "$copy_command"
  elif [[ -n "${INPUTIA_SUDO_PASSWORD:-}" ]]; then
    run_with_timeout 120 admin-copy run_admin_copy_with_password
  else
    run_with_timeout 120 admin-copy /usr/bin/osascript \
      -e 'on run argv' \
      -e 'do shell script item 1 of argv with administrator privileges' \
      -e 'end run' \
      "$copy_command"
  fi
fi

dest_cdhash="$(cdhash "$DEST_APP")"
dest_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$DEST_APP/Contents/Info.plist")"
echo "destVersion=$dest_version"
echo "destCDHash=$dest_cdhash"
if [[ "$dest_cdhash" != "$source_cdhash" ]]; then
  echo "systemInstallVerified=false reason=cdhash-mismatch" >&2
  exit 1
fi
echo "systemInstallVerified=true"
if [[ ! -e "$LEGACY_APP" ]]; then
  echo "legacyIputiaRemoved=true"
else
  echo "legacyIputiaRemoved=false path=$LEGACY_APP"
fi
if [[ -x "$DEST_SETTINGS_APP/Contents/MacOS/InputiaSettingsLauncher" ]]; then
  echo "settingsLauncherInstalled=true path=$DEST_SETTINGS_APP"
else
  echo "settingsLauncherInstalled=false path=$DEST_SETTINGS_APP"
fi

assessment_output="$(assess_app "$DEST_APP" || true)"
printf '%s\n' "$assessment_output" | /usr/bin/sed 's/^/systemInstallAssessment: /'
if [[ "$assessment_output" == *": accepted"* ]]; then
  echo "systemInstallSignatureAccepted=true"
  echo "systemInstallSignatureOverride=false"
elif [[ "${INPUTIA_ALLOW_REJECTED_SIGNATURE:-0}" == "1" ]]; then
  echo "systemInstallSignatureAccepted=false"
  echo "systemInstallSignatureOverride=true reason=signature-rejected"
  echo "systemInstallInputiaUsable=unknown reason=signature-rejected-override"
  echo "systemInstallRequiredAction=sign-with-accepted-identity"
  echo "systemInstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts"
else
  echo "systemInstallInputiaUsable=false reason=signature-rejected"
  echo "systemInstallRequiredAction=sign-with-accepted-identity"
  echo "systemInstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts"
  echo "systemInstallAction=stop-before-tis-registration"
  exit 14
fi

run_best_effort 8 lsregister-unregister-inputia "$LSREGISTER" -u "$DEST_APP"
run_best_effort 8 lsregister-unregister-build-inputia "$LSREGISTER" -u "$SOURCE_APP"
run_best_effort 8 lsregister-unregister-build-settings "$LSREGISTER" -u "$SOURCE_SETTINGS_APP"
run_best_effort 8 lsregister-register-inputia "$LSREGISTER" -f "$DEST_APP"
run_best_effort 8 lsregister-register-settings "$LSREGISTER" -f "$DEST_SETTINGS_APP"
run_best_effort 12 inputia-register "$DEST_APP/Contents/MacOS/InputiaInputMethod" --register-input-source
run_best_effort 12 inputia-dump-installed "$DEST_APP/Contents/MacOS/InputiaInputMethod" --dump-input-source
run_best_effort 12 inputia-dump-installed "$DEST_APP/Contents/MacOS/InputiaInputMethod" --dump-input-source

run_best_effort 12 inputia-register-before-refresh "$DEST_APP/Contents/MacOS/InputiaInputMethod" --register-input-source
/usr/bin/killall TextInputMenuAgent >/dev/null 2>&1 || true
/usr/bin/killall SystemUIServer >/dev/null 2>&1 || true
/bin/sleep 2
run_best_effort 12 inputia-register-after-refresh "$DEST_APP/Contents/MacOS/InputiaInputMethod" --register-input-source
echo "systemInstallRegistered=true"
echo "systemInstallPath=$DEST_APP"
echo "systemInstallTISReady=false reason=manual-add-required"
echo "systemInstallRequiredAction=add-input-source-in-system-settings"
echo "systemInstallNextStep=System Settings > Keyboard > Text Input > Edit > Add Inputia"
echo "systemInstallOpenSettingsCommand=open 'x-apple.systempreferences:com.apple.Keyboard-Settings.extension'"
