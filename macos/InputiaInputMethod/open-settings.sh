#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP="${INPUTIA_APP:-/Library/Input Methods/InputiaInputMethod.app}"
SETTINGS_APP="${INPUTIA_SETTINGS_APP:-/Applications/Inputia 设置.app}"
BUILD_APP="${INPUTIA_BUILD_APP:-$ROOT_DIR/build/InputiaInputMethod.app}"

detect_verification_processes() {
  local process_list
  if [[ -n "${INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST:-}" ]]; then
    process_list="$INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST"
  else
    process_list="$(/bin/ps -axo pid=,command=)"
  fi
  printf '%s\n' "$process_list" |
    /usr/bin/awk -v root="$ROOT_DIR" -v self="$$" -v owner="${INPUTIA_VERIFICATION_OWNER_PID:-}" '
      $1 == self { next }
      owner != "" && $1 == owner { next }
      index($0, root) &&
        $0 ~ /\/(dev-fast|install-check|release\/full-check|verify-nongui|post-install-regression|verify-system|verify-pkg|await-system-install|smoke-preflight|smoke-textedit|smoke-textedit-command-shortcuts|smoke-clipboard-recall|smoke-safari[^ ]*|diagnose-safari-input-source|gui-smoke-readiness|gui-smoke-suite|status|tis-readiness)\.sh( |$)/ {
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
  INPUTIA_OPEN_SETTINGS_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/install-check.sh"
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

plist_value() {
  /usr/libexec/PlistBuddy -c "Print :$2" "$1/Contents/Info.plist" 2>/dev/null || true
}

app_cdhash() {
  if [[ ! -d "$1" ]]; then
    return 0
  fi
  if [[ "${INPUTIA_OPEN_SETTINGS_ALLOW_TEST_CDHASH:-0}" == "1" ]]; then
    plist_value "$1" InputiaTestCDHash
    return 0
  fi
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2; exit}'
}

build_version="${INPUTIA_BUILD_VERSION_FOR_TEST:-$(bundle_version "$BUILD_APP")}"
build_host_cdhash="${INPUTIA_BUILD_HOST_CDHASH_FOR_TEST:-$(app_cdhash "$BUILD_APP")}"

open_path() {
  local path="$1"
  shift
  if [[ "${INPUTIA_OPEN_SETTINGS_DRY_RUN:-0}" == "1" ]]; then
    echo "openSettingsDryRun=true path=$path args=$*"
  else
    /usr/bin/open -n "$path" "$@"
  fi
}

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
  local expected_host_cdhash
  expected_host_cdhash="$(plist_value "$launcher_path" InputiaExpectedHostCDHash)"
  if [[ -n "$build_host_cdhash" && "$expected_host_cdhash" != "$build_host_cdhash" ]]; then
    echo "skippedStaleSettingsLauncher=$launcher_path expectedHostCDHash=${expected_host_cdhash:-unknown} expected=$build_host_cdhash"
    return 1
  fi

  open_path "$launcher_path"
  echo "openedSettingsLauncher=$launcher_path"
  return 0
}

host_is_current() {
  local host_path="$1"
  if [[ ! -d "$host_path" ]]; then
    return 1
  fi

  local host_version
  host_version="$(bundle_version "$host_path")"
  if [[ -n "$build_version" && "$host_version" != "$build_version" ]]; then
    echo "skippedStaleSettingsHost=$host_path version=$host_version expected=$build_version"
    return 1
  fi

  local host_cdhash
  host_cdhash="$(app_cdhash "$host_path")"
  if [[ -n "$build_host_cdhash" && "$host_cdhash" != "$build_host_cdhash" ]]; then
    echo "skippedStaleSettingsHost=$host_path cdhash=${host_cdhash:-unknown} expected=$build_host_cdhash"
    return 1
  fi

  return 0
}

write_fake_app() {
  local app_path="$1"
  local version="$2"
  local expected_host_cdhash="$3"
  local test_cdhash="$4"
  /bin/mkdir -p "$app_path/Contents"
  /usr/bin/python3 - "$app_path/Contents/Info.plist" "$version" "$expected_host_cdhash" "$test_cdhash" <<'PY'
import plistlib
import sys

path, version, expected_host_cdhash, test_cdhash = sys.argv[1:]
payload = {"CFBundleVersion": version}
if expected_host_cdhash:
    payload["InputiaExpectedHostCDHash"] = expected_host_cdhash
if test_cdhash:
    payload["InputiaTestCDHash"] = test_cdhash
with open(path, "wb") as fh:
    plistlib.dump(payload, fh)
PY
}

if [[ "${INPUTIA_OPEN_SETTINGS_RESOLUTION_SELF_CHECK:-0}" == "1" ]]; then
  self_check_root="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-open-settings-self-check.XXXXXX")"
  trap '/bin/rm -rf "$self_check_root" >/dev/null 2>&1 || true' EXIT

  fake_build_app="$self_check_root/build/InputiaInputMethod.app"
  fake_system_launcher="$self_check_root/system/Inputia 设置.app"
  fake_user_launcher="$self_check_root/user/Applications/Inputia 设置.app"
  fake_system_host="$self_check_root/system/InputiaInputMethod.app"
  write_fake_app "$fake_build_app" "47" "" "build-hash"
  write_fake_app "$fake_system_launcher" "47" "stale-hash" ""
  write_fake_app "$fake_user_launcher" "47" "build-hash" ""
  write_fake_app "$fake_system_host" "47" "" "stale-host-hash"

  launcher_output="$(
    INPUTIA_OPEN_SETTINGS_RESOLUTION_SELF_CHECK=0 \
      INPUTIA_BUILD_APP="$fake_build_app" \
      INPUTIA_SETTINGS_APP="$fake_system_launcher" \
      INPUTIA_APP="$fake_system_host" \
      HOME="$self_check_root/user" \
      INPUTIA_OPEN_SETTINGS_DRY_RUN=1 \
      INPUTIA_OPEN_SETTINGS_ALLOW_TEST_CDHASH=1 \
      "$0" 2>&1
  )"
  if ! /usr/bin/grep -q "^skippedStaleSettingsLauncher=$fake_system_launcher expectedHostCDHash=stale-hash expected=build-hash$" <<<"$launcher_output"; then
    echo "openSettingsResolutionSelfCheck=false reason=stale-system-launcher-not-skipped"
    exit 1
  fi
  if ! /usr/bin/grep -q "^openedSettingsLauncher=$fake_user_launcher$" <<<"$launcher_output"; then
    echo "openSettingsResolutionSelfCheck=false reason=current-user-launcher-not-selected"
    exit 1
  fi

  /bin/rm -rf "$fake_user_launcher"
  host_output="$(
    INPUTIA_OPEN_SETTINGS_RESOLUTION_SELF_CHECK=0 \
      INPUTIA_BUILD_APP="$fake_build_app" \
      INPUTIA_SETTINGS_APP="$fake_system_launcher" \
      INPUTIA_APP="$fake_system_host" \
      HOME="$self_check_root/user" \
      INPUTIA_OPEN_SETTINGS_DRY_RUN=1 \
      INPUTIA_OPEN_SETTINGS_ALLOW_TEST_CDHASH=1 \
      "$0" 2>&1
  )"
  if ! /usr/bin/grep -q "^skippedStaleSettingsHost=$fake_system_host cdhash=stale-host-hash expected=build-hash$" <<<"$host_output"; then
    echo "openSettingsResolutionSelfCheck=false reason=stale-host-not-skipped"
    exit 1
  fi
  if ! /usr/bin/grep -q "^openedSettingsApp=$fake_build_app$" <<<"$host_output"; then
    echo "openSettingsResolutionSelfCheck=false reason=build-host-not-selected"
    exit 1
  fi

  echo "openSettingsResolutionSelfCheck=true"
  exit 0
fi

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

if ! host_is_current "$APP" && [[ -d "$BUILD_APP" ]]; then
  APP="$BUILD_APP"
fi

if [[ ! -d "$APP" ]]; then
  echo "Inputia app not found. Run ./macos/InputiaInputMethod/build.sh or install the pkg first." >&2
  exit 1
fi

open_path "$APP" --args --open-settings
echo "openedSettingsApp=$APP"
