#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_NAME="InputiaInputMethod.app"
SETTINGS_APP_NAME="Inputia 设置.app"
SOURCE_APP="$ROOT_DIR/build/$APP_NAME"
SOURCE_SETTINGS_APP="$ROOT_DIR/build/$SETTINGS_APP_NAME"
DEST_DIR="$HOME/Library/Input Methods"
DEST_APP="$DEST_DIR/$APP_NAME"
LEGACY_APP="$DEST_DIR/IputiaInputMethod.app"
DEST_SETTINGS_DIR="$HOME/Applications"
DEST_SETTINGS_APP="$DEST_SETTINGS_DIR/$SETTINGS_APP_NAME"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
TMP_ROOT="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-install-user.XXXXXX")"
TMP_APP="$TMP_ROOT/$APP_NAME"
TMP_SETTINGS_APP="$TMP_ROOT/$SETTINGS_APP_NAME"
BUILD_LOG="$TMP_ROOT/build.log"

cleanup() {
  /bin/rm -rf "$TMP_ROOT"
}
trap cleanup EXIT

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
    echo "userInstallReady=false reason=verification-running"
    printf '%s\n' "$blocking_processes" | /usr/bin/sed 's/^/userInstallBlockingProcess: /'
    exit 20
  fi
}

if [[ "${INPUTIA_INSTALL_USER_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  original_process_list="${INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST:-}"
  INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="123 /usr/bin/true"
  clear_processes="$(detect_verification_processes)"
  INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/smoke-clipboard-recall.sh"
  blocked_processes="$(detect_verification_processes)"
  INPUTIA_INSTALL_PROCESS_LIST_FOR_TEST="$original_process_list"
  if [[ -z "$clear_processes" && -n "$blocked_processes" ]]; then
    echo "userInstallPreflightSelfCheck clear=true"
    echo "userInstallPreflightSelfCheck blocked=true"
    echo "userInstallPreflightSelfCheck=true"
    exit 0
  fi
  echo "userInstallPreflightSelfCheck=false"
  exit 1
fi

require_no_verification_processes

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

if "$ROOT_DIR/build.sh" >"$BUILD_LOG" 2>&1; then
  echo "userInstallBuild=true"
else
  echo "userInstallBuild=false"
  /usr/bin/tail -n 80 "$BUILD_LOG" >&2 || true
  exit 1
fi

mkdir -p "$DEST_DIR" "$DEST_SETTINGS_DIR"
killall InputiaInputMethod >/dev/null 2>&1 || true
killall IputiaInputMethod >/dev/null 2>&1 || true
"$LSREGISTER" -u "$LEGACY_APP" >/dev/null 2>&1 || true
/usr/bin/ditto --noextattr --noqtn "$SOURCE_APP" "$TMP_APP"
/usr/bin/ditto --noextattr --noqtn "$SOURCE_SETTINGS_APP" "$TMP_SETTINGS_APP"

source_cdhash="$(cdhash "$SOURCE_APP")"
tmp_cdhash="$(cdhash "$TMP_APP")"
if [[ -z "$source_cdhash" || "$tmp_cdhash" != "$source_cdhash" ]]; then
  echo "userInstallVerified=false reason=temp-cdhash-mismatch" >&2
  exit 1
fi
if [[ ! -f "$TMP_APP/Contents/Info.plist" || ! -x "$TMP_APP/Contents/MacOS/InputiaInputMethod" ]]; then
  echo "userInstallVerified=false reason=temp-app-incomplete" >&2
  exit 1
fi

rm -rf "$DEST_APP" "$LEGACY_APP" "$DEST_SETTINGS_APP"
/bin/mv "$TMP_APP" "$DEST_APP"
/bin/mv "$TMP_SETTINGS_APP" "$DEST_SETTINGS_APP"

"$LSREGISTER" -u "$DEST_APP" >/dev/null 2>&1 || true
"$LSREGISTER" -f "$DEST_APP" >/dev/null 2>&1 || true
"$DEST_APP/Contents/MacOS/InputiaInputMethod" --register-input-source
"$DEST_APP/Contents/MacOS/InputiaInputMethod" --enable-input-source
dump_output="$("$DEST_APP/Contents/MacOS/InputiaInputMethod" --dump-input-source 2>&1 || true)"
printf '%s\n' "$dump_output" | /usr/bin/awk -F= '
  $1 == "id" && $2 ~ /^com[.]inputia[.]inputmethod[.]Inputia/ { print "userInstallTISDump: " $0; next }
  $1 == "iconURL" { print "userInstallTISDump: " $0; next }
  $1 == "enabled" { print "userInstallTISDump: " $0; next }
  $1 == "selected" { print "userInstallTISDump: " $0; next }
'
tis_readiness_output="$(/bin/bash "$ROOT_DIR/tis-readiness.sh" "$DEST_APP" 2>&1 || true)"
printf '%s\n' "$tis_readiness_output" | /usr/bin/sed 's/^/userInstallTIS: /'
dest_cdhash="$(cdhash "$DEST_APP")"
if [[ "$dest_cdhash" != "$source_cdhash" ]]; then
  echo "userInstallVerified=false reason=dest-cdhash-mismatch" >&2
  exit 1
fi
echo "userInstallVerified=true"
if /usr/bin/grep -q '^tisReadiness=true$' <<<"$tis_readiness_output"; then
  echo "userInstallTISReady=true"
else
  echo "userInstallTISReady=false"
fi
echo "settingsLauncherInstalled=$DEST_SETTINGS_APP"
