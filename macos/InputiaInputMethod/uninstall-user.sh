#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
LEGACY_APP="$HOME/Library/Input Methods/IputiaInputMethod.app"
SETTINGS_APP="$HOME/Applications/Inputia 设置.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

detect_verification_processes() {
  local process_list
  if [[ -n "${INPUTIA_UNINSTALL_USER_PROCESS_LIST_FOR_TEST:-}" ]]; then
    process_list="$INPUTIA_UNINSTALL_USER_PROCESS_LIST_FOR_TEST"
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
    echo "userUninstallReady=false reason=verification-running"
    printf '%s\n' "$blocking_processes" | /usr/bin/sed 's/^/userUninstallBlockingProcess: /'
    exit 20
  fi
}

if [[ "${INPUTIA_UNINSTALL_USER_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  original_process_list="${INPUTIA_UNINSTALL_USER_PROCESS_LIST_FOR_TEST:-}"
  INPUTIA_UNINSTALL_USER_PROCESS_LIST_FOR_TEST="123 /usr/bin/true"
  clear_processes="$(detect_verification_processes)"
  INPUTIA_UNINSTALL_USER_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/gui-smoke-suite.sh"
  blocked_processes="$(detect_verification_processes)"
  INPUTIA_UNINSTALL_USER_PROCESS_LIST_FOR_TEST="$original_process_list"
  if [[ -z "$clear_processes" && -n "$blocked_processes" ]]; then
    echo "userUninstallPreflightSelfCheck clear=true"
    echo "userUninstallPreflightSelfCheck blocked=true"
    echo "userUninstallPreflightSelfCheck=true"
    exit 0
  fi
  echo "userUninstallPreflightSelfCheck=false"
  exit 1
fi

require_no_verification_processes

for candidate in "$APP" "$LEGACY_APP"; do
  if [[ -x "$candidate/Contents/MacOS/InputiaInputMethod" ]]; then
    "$candidate/Contents/MacOS/InputiaInputMethod" --disable-input-source || true
  elif [[ -x "$candidate/Contents/MacOS/IputiaInputMethod" ]]; then
    "$candidate/Contents/MacOS/IputiaInputMethod" --disable-input-source || true
  fi
  "$LSREGISTER" -u "$candidate" >/dev/null 2>&1 || true
done

rm -rf "$APP" "$LEGACY_APP" "$SETTINGS_APP"
killall InputiaInputMethod >/dev/null 2>&1 || true
killall IputiaInputMethod >/dev/null 2>&1 || true
echo "removed $APP"
