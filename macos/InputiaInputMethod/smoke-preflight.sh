#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$ROOT_DIR/smoke-common.sh"
USER_DEFAULT_APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
DEFAULT_APP="/Library/Input Methods/InputiaInputMethod.app"
if [[ -d "$USER_DEFAULT_APP" ]]; then
  DEFAULT_APP="$USER_DEFAULT_APP"
fi
APP="${1:-$DEFAULT_APP}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
TIS_TOOL="$ROOT_DIR/build/inputia-tis-tool"
EXECUTABLE="$APP/Contents/MacOS/InputiaInputMethod"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Main}"

cdhash() {
  /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
    /usr/bin/awk -F= '/^CDHash=/{print $2}'
}

absolute_path() {
  /usr/bin/python3 - "$1" <<'PY'
import os
import sys

print(os.path.abspath(sys.argv[1]))
PY
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
  if [[ -x "$EXECUTABLE" ]]; then
    "$EXECUTABLE" --dump-current-input-source 2>/dev/null |
      /usr/bin/awk -F= '$1 == "id" { print $2; exit }'
    return
  fi
  echo ""
}

print_tis_readiness_or_exit() {
  local readiness_output block_reason smoke_reason
  readiness_output="$("$ROOT_DIR/tis-readiness.sh" "$APP" 2>&1 || true)"
  printf '%s\n' "$readiness_output"
  if ! /usr/bin/grep -q '^tisReadiness=true$' <<<"$readiness_output"; then
    block_reason="$(/usr/bin/awk -F= '$1 == "tis.readinessBlockReason" { print $2; found = 1; exit } END { if (!found) print "tis-not-ready" }' <<<"$readiness_output")"
    if [[ "$block_reason" == "signature-rejected" ]]; then
      smoke_reason=signature-rejected
    else
      smoke_reason=tis-not-ready
    fi
    echo "guiSmokeReady=false reason=$smoke_reason"
    echo "smokePreflightReady=false reason=$smoke_reason"
    exit 8
  fi
}

echo "app=$APP"
if [[ ! -x "$EXECUTABLE" ]]; then
  echo "guiSmokeReady=false reason=missing-executable path=$EXECUTABLE"
  echo "smokePreflightReady=false reason=missing-executable"
  exit 1
fi

INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=0 inputia_require_textedit_idle "smokePreflightReady" 6
INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=0 inputia_require_safari_idle "smokePreflightReady" 7
inputia_require_process_not_running \
  "InputiaInputMethod" "smokePreflightReady" 9 \
  "inputia-host-running" "-"

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" == "1" ]]; then
  inputia_require_gui_session "smokePreflightReady" 5
fi

if [[ -d "$BUILD_APP" && "${INPUTIA_SKIP_CDHASH_CHECK:-0}" != "1" ]]; then
  expected_cdhash="$(cdhash "$BUILD_APP")"
  actual_cdhash="$(cdhash "$APP")"
  echo "expectedCDHash=$expected_cdhash"
  echo "actualCDHash=$actual_cdhash"
  if [[ "$actual_cdhash" != "$expected_cdhash" ]]; then
    echo "guiSmokeReady=false reason=cdhash-mismatch"
    echo "smokePreflightReady=false reason=cdhash-mismatch"
    exit 2
  fi
fi

if [[ ! -f "$APP/Contents/Resources/inputia.pdf" ]]; then
  echo "guiSmokeReady=false reason=missing-mode-icon"
  echo "smokePreflightReady=false reason=missing-mode-icon"
  exit 3
fi

if [[ "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
  echo "guiSmokeReady=false reason=ui-smoke-disabled"
  echo "smokePreflightReady=false reason=ui-smoke-disabled"
  echo "set INPUTIA_RUN_UI_SMOKE=1 to allow GUI smoke"
  exit 4
fi

print_tis_readiness_or_exit

echo "guiSmokeReady=true"
echo "smokePreflightReady=true"
