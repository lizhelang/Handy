#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
BUILD_EXEC="$BUILD_APP/Contents/MacOS/InputiaInputMethod"
SYSTEM_APP="${INPUTIA_REPAIR_SYSTEM_APP:-/Library/Input Methods/InputiaInputMethod.app}"
SYSTEM_EXEC="$SYSTEM_APP/Contents/MacOS/InputiaInputMethod"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Hans}"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
export INPUTIA_VERIFICATION_OWNER_PID="${INPUTIA_VERIFICATION_OWNER_PID:-$$}"

section() {
  printf '\n== %s ==\n' "$1"
}

app_cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
}

plist_value() {
  local plist="$1"
  local key="$2"
  if [[ -f "$plist" ]]; then
    /usr/libexec/PlistBuddy -c "Print :$key" "$plist" 2>/dev/null || true
  fi
}

run_best_effort() {
  "$@" || true
}

tis_value() {
  local output="$1"
  local key="$2"
  /usr/bin/awk -F= -v key="$key" '$1 == key { print $2; found = 1; exit } END { if (!found) print "unknown" }' <<<"$output"
}

run_tis_readiness() {
  if [[ -n "${INPUTIA_REPAIR_TIS_READINESS_FOR_TEST:-}" ]]; then
    printf '%s\n' "$INPUTIA_REPAIR_TIS_READINESS_FOR_TEST"
    return
  fi
  INPUTIA_TIS_INCLUDE_MENU_READINESS=0 "$ROOT_DIR/tis-readiness.sh" "$SYSTEM_APP" 2>&1 || true
}

run_self_check() {
  local sample output duplicate ready
  sample=$'tis.targetDuplicateMatches=true\ntisReadiness=false'
  duplicate="$(tis_value "$sample" "tis.targetDuplicateMatches")"
  ready="$(tis_value "$sample" "tisReadiness")"
  if [[ "$duplicate" != "true" || "$ready" != "false" ]]; then
    echo "tisDuplicateRepairSelfCheck=false reason=parse-sample-failed"
    exit 1
  fi
  output="$(
    INPUTIA_REPAIR_TIS_DUPLICATES=0 \
      INPUTIA_REPAIR_TIS_DUPLICATES_SELF_CHECK=0 \
      INPUTIA_REPAIR_TIS_READINESS_FOR_TEST="$sample" \
      INPUTIA_REPAIR_SYSTEM_APP="$SYSTEM_APP" \
      "$0" 2>&1 || true
  )"
  if ! /usr/bin/grep -q '^tisDuplicateRepairReady=false reason=opt-in-required$' <<<"$output"; then
    echo "tisDuplicateRepairSelfCheck=false reason=missing-opt-in-gate"
    exit 1
  fi
  if ! /usr/bin/grep -q '^changesSystemInputSource=false$' <<<"$output"; then
    echo "tisDuplicateRepairSelfCheck=false reason=missing-dry-run-policy"
    exit 1
  fi
  echo "tisDuplicateRepairSelfCheck=true"
}

if [[ "${INPUTIA_REPAIR_TIS_DUPLICATES_SELF_CHECK:-0}" == "1" ]]; then
  run_self_check
  exit 0
fi

section "policy"
echo "validationTier=tis-duplicate-repair"
echo "touchesMenuBar=false"
echo "opensGUI=false"
if [[ "${INPUTIA_REPAIR_TIS_DUPLICATES:-0}" == "1" ]]; then
  echo "changesSystemInputSource=true"
else
  echo "changesSystemInputSource=false"
fi
echo "checksNotarization=false"

section "preflight"
build_cdhash="$(app_cdhash "$BUILD_APP")"
system_cdhash="$(app_cdhash "$SYSTEM_APP")"
build_version="$(plist_value "$BUILD_APP/Contents/Info.plist" CFBundleVersion)"
system_version="$(plist_value "$SYSTEM_APP/Contents/Info.plist" CFBundleVersion)"
echo "buildApp=$BUILD_APP"
echo "buildVersion=${build_version:-unknown}"
echo "buildCDHash=${build_cdhash:-unknown}"
echo "systemApp=$SYSTEM_APP"
echo "systemVersion=${system_version:-unknown}"
echo "systemCDHash=${system_cdhash:-unknown}"
if [[ -n "${build_cdhash:-}" && "$build_cdhash" == "$system_cdhash" ]]; then
  echo "systemMatchesBuild=true"
else
  echo "systemMatchesBuild=false"
fi

readiness_before="$(run_tis_readiness)"
printf '%s\n' "$readiness_before" | /usr/bin/awk '
  /^tis.targetEnabledMatches=|^tis.targetInstalledMatches=|^tis.targetDuplicateMatches=|^tis.readinessBlockReason=|^tis.requiredAction=|^tisReadiness=/ { print "before." $0 }
'
duplicate_before="$(tis_value "$readiness_before" "tis.targetDuplicateMatches")"

if [[ "$duplicate_before" != "true" ]]; then
  echo "tisDuplicateRepairReady=true reason=no-duplicate"
  echo "tisDuplicateRepairPassed=true"
  exit 0
fi

if [[ "${INPUTIA_REPAIR_TIS_DUPLICATES:-0}" != "1" ]]; then
  echo "tisDuplicateRepairReady=false reason=opt-in-required"
  echo "tisDuplicateRepairRequiredAction=rerun-with-INPUTIA_REPAIR_TIS_DUPLICATES=1"
  echo "tisDuplicateRepairCommand=INPUTIA_REPAIR_TIS_DUPLICATES=1 $0"
  exit 2
fi

if [[ ! -x "$BUILD_EXEC" && ! -x "$SYSTEM_EXEC" ]]; then
  echo "tisDuplicateRepairReady=false reason=missing-inputia-executable"
  exit 3
fi
if [[ ! -x "$SYSTEM_EXEC" ]]; then
  echo "tisDuplicateRepairReady=false reason=missing-system-inputia-executable"
  exit 4
fi

CONTROL_EXEC="$BUILD_EXEC"
if [[ ! -x "$CONTROL_EXEC" ]]; then
  CONTROL_EXEC="$SYSTEM_EXEC"
fi

section "repair"
echo "tisDuplicateRepairControlExecutable=$CONTROL_EXEC"
echo "tisDuplicateRepairSystemExecutable=$SYSTEM_EXEC"
run_best_effort "$CONTROL_EXEC" --disable-all-inputia-sources
run_best_effort "$LSREGISTER" -u "$SYSTEM_APP"
run_best_effort "$LSREGISTER" -f "$SYSTEM_APP"
run_best_effort "$SYSTEM_EXEC" --register-input-source
run_best_effort "$SYSTEM_EXEC" --enable-input-source
run_best_effort "$SYSTEM_EXEC" --select-input-source

section "result"
readiness_after="$(run_tis_readiness)"
printf '%s\n' "$readiness_after" | /usr/bin/awk '
  /^tis.targetEnabledMatches=|^tis.targetInstalledMatches=|^tis.targetDuplicateMatches=|^tis.readinessBlockReason=|^tis.requiredAction=|^tisReadiness=/ { print "after." $0 }
'
duplicate_after="$(tis_value "$readiness_after" "tis.targetDuplicateMatches")"
ready_after="$(tis_value "$readiness_after" "tisReadiness")"
if [[ "$duplicate_after" == "true" ]]; then
  echo "tisDuplicateRepairPassed=false reason=duplicate-still-present"
  exit 5
fi
if [[ "$ready_after" != "true" ]]; then
  echo "tisDuplicateRepairPassed=false reason=tis-not-ready-after-repair"
  exit 6
fi
echo "tisDuplicateRepairPassed=true"
