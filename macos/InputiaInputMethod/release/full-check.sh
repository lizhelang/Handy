#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
MENU_CACHE="${TMPDIR:-/tmp}/inputia-menu-readiness.$$.$RANDOM.cache"
export INPUTIA_VERIFICATION_OWNER_PID="${INPUTIA_VERIFICATION_OWNER_PID:-$$}"

cleanup() {
  /bin/rm -f "$MENU_CACHE" >/dev/null 2>&1 || true
}
trap cleanup EXIT

section() {
  printf '\n== %s ==\n' "$1"
}

export INPUTIA_MENU_READINESS_CACHE_FILE="$MENU_CACHE"
export INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1
export INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1
export INPUTIA_TIS_INCLUDE_MENU_READINESS=1
export INPUTIA_STATUS_INCLUDE_MENU_READINESS=1
export INPUTIA_STATUS_INCLUDE_GUI_SMOKE_READINESS=1
export INPUTIA_RUN_UI_SMOKE=1

section "policy"
echo "validationTier=release/full-check"
echo "touchesMenuBar=true"
echo "opensGUI=true"
echo "changesSystemInputSource=false"
echo "checksNotarization=true"
echo "menuReadinessCacheFile=$MENU_CACHE"

section "pkg"
"$ROOT_DIR/build-pkg.sh"
"$ROOT_DIR/verify-pkg.sh"

section "notarization readiness"
notarization_output="$("$ROOT_DIR/notarization-readiness.sh" "$ROOT_DIR/build/InputiaInputMethod.app" 2>&1)"
printf '%s\n' "$notarization_output"
if ! /usr/bin/grep -q '^inputiaGatekeeperReady=true$' <<<"$notarization_output"; then
  echo "releaseFullCheckPassed=false reason=gatekeeper-not-ready"
  exit 10
fi
if ! /usr/bin/grep -q '^inputiaNotarySubmissionReady=true$' <<<"$notarization_output"; then
  echo "releaseFullCheckPassed=false reason=notary-submission-not-ready"
  exit 11
fi

section "menu readiness"
"$ROOT_DIR/menu-readiness.sh"

section "postinstall and GUI smoke"
"$ROOT_DIR/post-install-regression.sh" "$APP"

section "result"
echo "releaseFullCheckPassed=true"
