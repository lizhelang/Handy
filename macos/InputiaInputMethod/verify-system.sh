#!/bin/zsh
set -euo pipefail

APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
DIAGNOSTIC_ROOT=""
TEMP_FILES=()

cleanup() {
  if [[ -n "$DIAGNOSTIC_ROOT" ]]; then
    /bin/rm -rf "$DIAGNOSTIC_ROOT"
  fi
  for temp_file in "${TEMP_FILES[@]}"; do
    [[ -n "$temp_file" ]] && /bin/rm -f "$temp_file" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT

section() {
  printf '\n== %s ==\n' "$1"
}

run_inputia() {
  local label="$1"
  shift
  local attempt output pid elapsed exit_status
  for attempt in 1 2; do
    output="${TMPDIR:-/tmp}/inputia-${label}-${attempt}-$$.log"
    TEMP_FILES+=("$output")
    : >"$output"
    "$@" >"$output" 2>&1 &
    pid="$!"
    elapsed=0
    while /bin/kill -0 "$pid" >/dev/null 2>&1; do
      if ((elapsed >= 100)); then
        /bin/kill "$pid" >/dev/null 2>&1 || true
        wait "$pid" >/dev/null 2>&1 || true
        echo "${label}TimedOut=true attempt=$attempt"
        [[ -f "$output" ]] && /bin/cat "$output"
        /bin/rm -f "$output"
        if ((attempt == 2)); then
          return 124
        fi
        /bin/sleep 1
        break
      fi
      /bin/sleep 0.1
      elapsed=$((elapsed + 1))
    done
    if ! /bin/kill -0 "$pid" >/dev/null 2>&1; then
      if wait "$pid"; then
        exit_status=0
      else
        exit_status=$?
      fi
      [[ -f "$output" ]] && /bin/cat "$output"
      /bin/rm -f "$output"
      if ((exit_status == 137 && attempt < 2)); then
        echo "${label}Killed=true attempt=$attempt"
        /bin/sleep 1
        continue
      fi
      return "$exit_status"
    fi
  done
}

section "bundle"
if [[ ! -d "$APP" ]]; then
  echo "installed=false"
  echo "path=$APP"
  exit 1
fi
echo "installed=true"
echo "path=$APP"
/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$APP/Contents/Info.plist"

section "resources"
find "$APP/Contents/Resources" -maxdepth 3 -type f | sort

section "required host resources"
for required_path in \
  "$APP/Contents/Resources/Inputia.icns" \
  "$APP/Contents/Resources/inputia.pdf" \
  "$APP/Contents/Resources/en.lproj/InfoPlist.strings" \
  "$APP/Contents/Resources/zh-Hans.lproj/InfoPlist.strings" \
  "$APP/Contents/Resources/zh-Hant.lproj/InfoPlist.strings"; do
  if [[ -f "$required_path" ]]; then
    echo "resourcePresent=true path=$required_path"
  else
    echo "resourcePresent=false path=$required_path"
  fi
done

section "plist"
/usr/bin/plutil -lint "$APP/Contents/Info.plist"
find "$APP/Contents/Resources" -name 'InfoPlist.strings' -print0 |
  while IFS= read -r -d '' strings_file; do
    /usr/bin/plutil -lint "$strings_file"
  done

section "codesign"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP"
/usr/bin/codesign -dv --verbose=4 "$APP" 2>&1 | sed -n '1,80p'

section "diagnostic executable"
DIAGNOSTIC_EXECUTABLE="$EXECUTABLE"
echo "diagnosticExecutable=$DIAGNOSTIC_EXECUTABLE"

section "class self-check"
run_inputia selfCheck "$DIAGNOSTIC_EXECUTABLE" --self-check

section "bridge self-check"
run_inputia bridgeSelfCheck "$DIAGNOSTIC_EXECUTABLE" --bridge-self-check
run_inputia bridgeMemorySelfCheck "$DIAGNOSTIC_EXECUTABLE" --bridge-memory-self-check
run_inputia bridgeSettingsSelfCheck "$DIAGNOSTIC_EXECUTABLE" --bridge-settings-self-check
if /usr/bin/grep -a -q -- '--bridge-default-chinese-self-check' "$DIAGNOSTIC_EXECUTABLE"; then
  run_inputia bridgeDefaultChineseSelfCheck "$DIAGNOSTIC_EXECUTABLE" --bridge-default-chinese-self-check
else
  echo "bridgeDefaultChineseSelfCheck=skipped"
fi
if /usr/bin/grep -a -q -- '--host-shortcut-self-check' "$DIAGNOSTIC_EXECUTABLE"; then
  run_inputia hostShortcutSelfCheck "$DIAGNOSTIC_EXECUTABLE" --host-shortcut-self-check
else
  echo "hostShortcutSelfCheck=skipped"
fi

section "tis all-installed and enabled"
run_inputia dumpSourcePrefix "$DIAGNOSTIC_EXECUTABLE" --dump-source-prefix com.inputia.inputmethod.Inputia

section "current source"
if [[ -x "$TIS_TOOL" ]]; then
  "$TIS_TOOL" --dump-current-input-source
else
  run_inputia dumpCurrentInputSource "$DIAGNOSTIC_EXECUTABLE" --dump-current-input-source
fi

section "launchservices"
if [[ "${INPUTIA_VERIFY_LAUNCHSERVICES:-0}" == "1" ]]; then
  launchservices_output="${TMPDIR:-/tmp}/inputia-launchservices-$$.log"
  TEMP_FILES+=("$launchservices_output")
  : >"$launchservices_output"
  (
    "$LSREGISTER" -dump 2>/dev/null |
      /usr/bin/grep -i -A 8 -B 4 'com.inputia.inputmethod.Inputia' |
      sed -n '1,160p'
  ) >"$launchservices_output" &
  launchservices_pid="$!"
  launchservices_elapsed=0
  while /bin/kill -0 "$launchservices_pid" >/dev/null 2>&1; do
    if ((launchservices_elapsed >= 30)); then
      /bin/kill "$launchservices_pid" >/dev/null 2>&1 || true
      /usr/bin/pkill -P "$launchservices_pid" >/dev/null 2>&1 || true
      wait "$launchservices_pid" >/dev/null 2>&1 || true
      echo "launchservicesTimedOut=true"
      break
    fi
    /bin/sleep 0.1
    launchservices_elapsed=$((launchservices_elapsed + 1))
  done
  if ! /bin/kill -0 "$launchservices_pid" >/dev/null 2>&1; then
    wait "$launchservices_pid" >/dev/null 2>&1 || true
  fi
  [[ -f "$launchservices_output" ]] && /bin/cat "$launchservices_output"
  /bin/rm -f "$launchservices_output"
else
  echo "launchservicesDumpSkipped=true"
fi
