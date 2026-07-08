#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
SYSTEM_APP="${INPUTIA_SYSTEM_APP_FOR_TEST:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_SETTINGS_APP="$ROOT_DIR/build/Inputia 设置.app"
SYSTEM_SETTINGS_APP="${INPUTIA_SYSTEM_SETTINGS_APP_FOR_TEST:-/Applications/Inputia 设置.app}"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Hans}"
export INPUTIA_VERIFICATION_OWNER_PID="${INPUTIA_VERIFICATION_OWNER_PID:-$$}"

section() {
  printf '\n== %s ==\n' "$1"
}

plist_value() {
  local plist="$1"
  local key="$2"
  if [[ -f "$plist" ]]; then
    /usr/libexec/PlistBuddy -c "Print :$key" "$plist" 2>/dev/null || true
  fi
}

app_version() {
  plist_value "$1/Contents/Info.plist" CFBundleVersion
}

app_cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
}

process_pids() {
  /usr/bin/pgrep -x InputiaInputMethod 2>/dev/null || true
}

append_reason() {
  local reasons="$1"
  local reason="$2"
  if [[ ",$reasons," == *",$reason,"* ]]; then
    echo "$reasons"
  elif [[ -z "$reasons" ]]; then
    echo "$reason"
  else
    echo "$reasons,$reason"
  fi
}

append_action() {
  local actions="$1"
  local action="$2"
  if [[ ",$actions," == *",$action,"* ]]; then
    echo "$actions"
  elif [[ -z "$actions" ]]; then
    echo "$action"
  else
    echo "$actions,$action"
  fi
}

admin_install_ready() {
  if [[ -w "/Library/Input Methods" && -w "/Applications" ]]; then
    echo true
  elif /usr/bin/sudo -n true >/dev/null 2>&1; then
    echo true
  else
    echo false
  fi
}

install_required_action() {
  local block_reasons="$1"
  if [[ "$block_reasons" == "none" ]]; then
    echo "none"
  elif [[ ",$block_reasons," == *,system-app-missing,* ||
    ",$block_reasons," == *,system-cdhash-mismatch,* ||
    ",$block_reasons," == *,settings-version-mismatch,* ]]; then
    if [[ ",$block_reasons," == *,admin-required,* ]]; then
      echo "run-install-handoff-and-admin-install"
    else
      echo "run-install-system"
    fi
  elif [[ ",$block_reasons," == *,tis-duplicate-matches,* ]]; then
    echo "remove-duplicate-inputia-and-readd-once"
  elif [[ ",$block_reasons," == *,tis-not-ready,* ]]; then
    echo "select-or-readd-inputia-in-system-settings"
  elif [[ ",$block_reasons," == *,running-host-missing,* ||
    ",$block_reasons," == *,running-cdhash-mismatch,* ]]; then
    echo "restart-inputia-host-after-install"
  else
    echo "inspect-install-check-output"
  fi
}

install_required_actions() {
  local block_reasons="$1"
  local actions=""

  if [[ "$block_reasons" == "none" ]]; then
    echo "none"
    return
  fi

  if [[ ",$block_reasons," == *,system-app-missing,* ||
    ",$block_reasons," == *,system-cdhash-mismatch,* ||
    ",$block_reasons," == *,settings-version-mismatch,* ]]; then
    if [[ ",$block_reasons," == *,admin-required,* ]]; then
      actions="$(append_action "$actions" "run-install-handoff-and-admin-install")"
    else
      actions="$(append_action "$actions" "run-install-system")"
    fi
  fi

  if [[ ",$block_reasons," == *,tis-duplicate-matches,* ]]; then
    actions="$(append_action "$actions" "run-repair-tis-duplicates")"
  elif [[ ",$block_reasons," == *,tis-not-ready,* ]]; then
    actions="$(append_action "$actions" "select-or-readd-inputia-in-system-settings")"
  fi

  if [[ ",$block_reasons," == *,running-host-missing,* ||
    ",$block_reasons," == *,running-cdhash-mismatch,* ]]; then
    actions="$(append_action "$actions" "restart-inputia-host-after-install")"
  fi

  if [[ -z "$actions" ]]; then
    actions="inspect-install-check-output"
  fi
  echo "$actions"
}

install_check_block_reasons() {
  local system_exists="$1"
  local system_matches_build="$2"
  local settings_matches_build="$3"
  local tis_ready="$4"
  local tis_duplicate_matches="$5"
  local running_found="$6"
  local running_matches_build="$7"
  local admin_ready="$8"
  local block_reasons=""

  if [[ "$system_exists" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "system-app-missing")"
  elif [[ "$system_matches_build" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "system-cdhash-mismatch")"
  fi
  if [[ "$settings_matches_build" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "settings-version-mismatch")"
  fi
  if [[ "$tis_duplicate_matches" == "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "tis-duplicate-matches")"
  elif [[ "$tis_ready" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "tis-not-ready")"
  fi
  if [[ "$running_found" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "running-host-missing")"
  elif [[ "$running_matches_build" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "running-cdhash-mismatch")"
  fi
  if [[ "$admin_ready" != "true" &&
    ( "$system_matches_build" != "true" || "$settings_matches_build" != "true" ) ]]; then
    block_reasons="$(append_reason "$block_reasons" "admin-required")"
  fi
  if [[ -z "$block_reasons" ]]; then
    block_reasons=none
  fi
  echo "$block_reasons"
}

run_install_check_self_check_case() {
  local label="$1"
  local system_exists="$2"
  local system_matches_build="$3"
  local settings_matches_build="$4"
  local tis_ready="$5"
  local tis_duplicate_matches="$6"
  local running_found="$7"
  local running_matches_build="$8"
  local admin_ready="$9"
  local expected_reasons="${10}"
  local expected_action="${11}"
  local expected_actions="${12}"
  local actual_reasons actual_action actual_actions

  actual_reasons="$(
    install_check_block_reasons \
      "$system_exists" \
      "$system_matches_build" \
      "$settings_matches_build" \
      "$tis_ready" \
      "$tis_duplicate_matches" \
      "$running_found" \
      "$running_matches_build" \
      "$admin_ready"
  )"
  actual_action="$(install_required_action "$actual_reasons")"
  actual_actions="$(install_required_actions "$actual_reasons")"
  echo "installCheckSelfCheck case=$label reasons=$actual_reasons action=$actual_action actions=$actual_actions"
  if [[ "$actual_reasons" != "$expected_reasons" ]]; then
    echo "installCheckSelfCheck=false case=$label reason=reasons-mismatch expected=$expected_reasons actual=$actual_reasons"
    exit 1
  fi
  if [[ "$actual_action" != "$expected_action" ]]; then
    echo "installCheckSelfCheck=false case=$label reason=action-mismatch expected=$expected_action actual=$actual_action"
    exit 1
  fi
  if [[ "$actual_actions" != "$expected_actions" ]]; then
    echo "installCheckSelfCheck=false case=$label reason=actions-mismatch expected=$expected_actions actual=$actual_actions"
    exit 1
  fi
}

if [[ "${INPUTIA_INSTALL_CHECK_SELF_CHECK:-0}" == "1" ]]; then
  run_install_check_self_check_case \
    ready true true true true false true true true \
    none none none
  run_install_check_self_check_case \
    admin-required true false true true false true false false \
    system-cdhash-mismatch,running-cdhash-mismatch,admin-required \
    run-install-handoff-and-admin-install \
    run-install-handoff-and-admin-install,restart-inputia-host-after-install
  run_install_check_self_check_case \
    tis-not-ready true true true false false true true true \
    tis-not-ready \
    select-or-readd-inputia-in-system-settings \
    select-or-readd-inputia-in-system-settings
  run_install_check_self_check_case \
    tis-duplicate true true true false true true true true \
    tis-duplicate-matches \
    remove-duplicate-inputia-and-readd-once \
    run-repair-tis-duplicates
  run_install_check_self_check_case \
    running-missing true true true true false false false true \
    running-host-missing \
    restart-inputia-host-after-install \
    restart-inputia-host-after-install
  run_install_check_self_check_case \
    settings-admin true true false true false true true false \
    settings-version-mismatch,admin-required \
    run-install-handoff-and-admin-install \
    run-install-handoff-and-admin-install
  run_install_check_self_check_case \
    system-missing false false true false false false false false \
    system-app-missing,tis-not-ready,running-host-missing,admin-required \
    run-install-handoff-and-admin-install \
    run-install-handoff-and-admin-install,select-or-readd-inputia-in-system-settings,restart-inputia-host-after-install
  echo "installCheckSelfCheck=true"
  exit 0
fi

section "policy"
echo "validationTier=install-check"
echo "touchesMenuBar=false"
echo "opensGUI=false"
echo "changesSystemInputSource=false"
echo "checksNotarization=false"
admin_ready="$(admin_install_ready)"
echo "adminInstallReady=$admin_ready"

section "system host"
build_version="$(app_version "$BUILD_APP")"
system_version="$(app_version "$SYSTEM_APP")"
build_cdhash="$(app_cdhash "$BUILD_APP")"
system_cdhash="$(app_cdhash "$SYSTEM_APP")"
echo "buildApp=$BUILD_APP"
echo "buildVersion=${build_version:-unknown}"
echo "buildCDHash=${build_cdhash:-unknown}"
echo "systemApp=$SYSTEM_APP"
echo "systemExists=$([[ -d "$SYSTEM_APP" ]] && echo true || echo false)"
echo "systemVersion=${system_version:-unknown}"
echo "systemCDHash=${system_cdhash:-unknown}"
if [[ -n "${build_cdhash:-}" && "$system_cdhash" == "$build_cdhash" ]]; then
  system_matches_build=true
else
  system_matches_build=false
fi
echo "systemMatchesBuild=$system_matches_build"

section "settings app"
build_settings_version="$(app_version "$BUILD_SETTINGS_APP")"
system_settings_version="$(app_version "$SYSTEM_SETTINGS_APP")"
echo "buildSettingsVersion=${build_settings_version:-unknown}"
echo "systemSettingsApp=$SYSTEM_SETTINGS_APP"
echo "systemSettingsExists=$([[ -d "$SYSTEM_SETTINGS_APP" ]] && echo true || echo false)"
echo "systemSettingsVersion=${system_settings_version:-unknown}"
if [[ -n "${build_settings_version:-}" && "$system_settings_version" == "$build_settings_version" ]]; then
  settings_matches_build=true
else
  settings_matches_build=false
fi
echo "settingsMatchesBuild=$settings_matches_build"

section "tis"
tis_output="$(INPUTIA_TIS_INCLUDE_MENU_READINESS=0 "$ROOT_DIR/tis-readiness.sh" "$SYSTEM_APP" 2>&1 || true)"
printf '%s\n' "$tis_output" | /usr/bin/awk '
  /^app=|^appExists=|^buildCDHash=|^appCDHash=|^appMatchesBuild=|^expectedTISModeID=|^tis.targetEnabledMatches=|^tis.targetInstalledMatches=|^tis.targetDuplicateMatches=|^tis.hansIconMatchesApp=|^tis.hansEnabled=|^tis.hansSelectable=|^tis.hansSelected=|^tis.currentID=|^tis.currentMatchesTarget=|^tis.menuReadiness=|^tis.menuBlockReason=|^tis.readinessBlockReason=|^tis.requiredAction=|^tisReadiness=/ { print }
'
tis_ready=false
if /usr/bin/grep -q '^tisReadiness=true$' <<<"$tis_output"; then
  tis_ready=true
fi
tis_duplicate_matches=false
if /usr/bin/grep -q '^tis.targetDuplicateMatches=true$' <<<"$tis_output"; then
  tis_duplicate_matches=true
fi
echo "installCheckTISReady=$tis_ready"
echo "installCheckTISDuplicateMatches=$tis_duplicate_matches"

section "running host"
running_matches_build=false
running_found=false
while IFS= read -r pid; do
  [[ -z "$pid" ]] && continue
  running_found=true
  command="$(/bin/ps -p "$pid" -o command= 2>/dev/null || true)"
  echo "runningPID=$pid"
  echo "runningCommand=$command"
  if [[ "$command" == "$SYSTEM_APP/Contents/MacOS/InputiaInputMethod"* ]]; then
    running_version="$(app_version "$SYSTEM_APP")"
    running_cdhash="$(app_cdhash "$SYSTEM_APP")"
    echo "runningApp=$SYSTEM_APP"
    echo "runningVersion=${running_version:-unknown}"
    echo "runningCDHash=${running_cdhash:-unknown}"
    if [[ -n "${build_cdhash:-}" && "$running_cdhash" == "$build_cdhash" ]]; then
      running_matches_build=true
    fi
  fi
done <<<"$(process_pids)"
echo "runningHostFound=$running_found"
echo "runningMatchesBuild=$running_matches_build"

section "result"
system_exists="$([[ -d "$SYSTEM_APP" ]] && echo true || echo false)"
block_reasons="$(
  install_check_block_reasons \
    "$system_exists" \
    "$system_matches_build" \
    "$settings_matches_build" \
    "$tis_ready" \
    "$tis_duplicate_matches" \
    "$running_found" \
    "$running_matches_build" \
    "$admin_ready"
)"
echo "installCheckBlockReasons=$block_reasons"
if [[ "$block_reasons" == "none" ]]; then
  echo "installCheckRequiredAction=none"
  echo "installCheckRequiredActions=none"
else
  echo "installCheckRequiredAction=$(install_required_action "$block_reasons")"
  echo "installCheckRequiredActions=$(install_required_actions "$block_reasons")"
fi
if [[ "$system_matches_build" == "true" &&
  "$settings_matches_build" == "true" &&
  "$tis_ready" == "true" &&
  "$tis_duplicate_matches" != "true" &&
  "$running_matches_build" == "true" ]]; then
  echo "installCheckPassed=true"
else
  echo "installCheckPassed=false"
  exit 1
fi
