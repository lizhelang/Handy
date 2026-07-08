#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP="${INPUTIA_APP:-/Library/Input Methods/InputiaInputMethod.app}"
SETTINGS_APP="${INPUTIA_SETTINGS_APP:-/Applications/Inputia 设置.app}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"

detect_verification_processes() {
  local process_list
  if [[ -n "${INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST:-}" ]]; then
    process_list="$INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST"
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
    echo "openSettingsReady=false reason=verification-running"
    printf '%s\n' "$blocking_processes" | /usr/bin/sed 's/^/openSettingsBlockingProcess: /'
    exit 20
  fi
}

if [[ "${INPUTIA_OPEN_SETTINGS_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  original_process_list="${INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST:-}"
  INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST="123 /usr/bin/true"
  clear_processes="$(detect_verification_processes)"
  INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/smoke-textedit.sh"
  blocked_processes="$(detect_verification_processes)"
  INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST="$original_process_list"
  if [[ -z "$clear_processes" && -n "$blocked_processes" ]]; then
    echo "openSettingsPreflightSelfCheck clear=true"
    echo "openSettingsPreflightSelfCheck blocked=true"
    echo "openSettingsPreflightSelfCheck=true"
    exit 0
  fi
  echo "openSettingsPreflightSelfCheck=false"
  exit 1
fi

require_no_verification_processes

bundle_version() {
  /usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$1/Contents/Info.plist" 2>/dev/null || true
}

build_version="$(bundle_version "$BUILD_APP")"

open_settings_launcher_if_current() {
  local launcher_path="$1"
  if [[ ! -d "$launcher_path" ]]; then
    return 1
  fi

  local launcher_version
  launcher_version="$(bundle_version "$launcher_path")"
  if [[ -n "$build_version" && "$launcher_version" != "$build_version" ]]; then
    echo "skippedStaleSettingsLauncher=$launcher_path version=$launcher_version expected=$build_version"
    return 1
  fi

  /usr/bin/open -n "$launcher_path"
  echo "openedSettingsLauncher=$launcher_path"
  return 0
}

if open_settings_launcher_if_current "$SETTINGS_APP"; then
  exit 0
fi

if open_settings_launcher_if_current "$HOME/Applications/Inputia 设置.app"; then
  exit 0
fi

if [[ ! -d "$APP" ]]; then
  if [[ -d "$HOME/Library/Input Methods/InputiaInputMethod.app" ]]; then
    APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
  elif [[ -d "$BUILD_APP" ]]; then
    APP="$BUILD_APP"
  fi
fi

app_version="$(bundle_version "$APP")"
if [[ -n "$build_version" && "$app_version" != "$build_version" && -d "$BUILD_APP" ]]; then
  echo "skippedStaleSettingsHost=$APP version=$app_version expected=$build_version"
  APP="$BUILD_APP"
fi

if [[ ! -d "$APP" ]]; then
  echo "Inputia app not found. Run ./macos/InputiaInputMethod/build.sh or install the pkg first." >&2
  exit 1
fi

/usr/bin/open -n "$APP" --args --open-settings
echo "openedSettingsApp=$APP"
