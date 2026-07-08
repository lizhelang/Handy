#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$ROOT_DIR/../.." && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
SYSTEM_APP="${INPUTIA_SYSTEM_APP_FOR_TEST:-/Library/Input Methods/InputiaInputMethod.app}"
BUILD_SETTINGS_APP="$ROOT_DIR/build/Inputia 设置.app"
SYSTEM_SETTINGS_APP="${INPUTIA_SYSTEM_SETTINGS_APP_FOR_TEST:-/Applications/Inputia 设置.app}"
TARGET_MODE_ID="${INPUTIA_TIS_MODE_ID:-com.inputia.inputmethod.Inputia.Hans}"
HANDOFF_PATH="${INPUTIA_INSTALL_HANDOFF_PATH:-$ROOT_DIR/build/install-handoff.txt}"
export INPUTIA_VERIFICATION_OWNER_PID="${INPUTIA_VERIFICATION_OWNER_PID:-$$}"

section() {
  printf '\n== %s ==\n' "$1"
}

quote() {
  /usr/bin/python3 - "$1" <<'PY'
import shlex
import sys

print(shlex.quote(sys.argv[1]))
PY
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

app_source_commit() {
  plist_value "$1/Contents/Info.plist" InputiaSourceCommit
}

app_source_dirty() {
  plist_value "$1/Contents/Info.plist" InputiaSourceDirty
}

app_cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
}

sha256() {
  if [[ -f "$1" ]]; then
    /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
  fi
}

git_value() {
  local fallback="$1"
  shift
  /usr/bin/git -C "$REPO_ROOT" "$@" 2>/dev/null || echo "$fallback"
}

git_dirty_state() {
  if ! /usr/bin/git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo unknown
    return
  fi
  if [[ -n "$(/usr/bin/git -C "$REPO_ROOT" status --porcelain 2>/dev/null)" ]]; then
    echo true
  else
    echo false
  fi
}

handoff_value() {
  local key="$1"
  if [[ ! -f "$HANDOFF_PATH" ]]; then
    echo ""
    return
  fi
  /usr/bin/awk -F= -v key="$key" '$1 == key { print $2; found = 1; exit } END { if (!found) print "" }' "$HANDOFF_PATH"
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

action_present() {
  local actions="$1"
  local action="$2"
  [[ ",$actions," == *",$action,"* ]]
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
  local handoff_current="${2:-false}"
  if [[ "$block_reasons" == "none" ]]; then
    echo "none"
  elif [[ ",$block_reasons," == *,system-app-missing,* ||
    ",$block_reasons," == *,system-cdhash-mismatch,* ||
    ",$block_reasons," == *,settings-version-mismatch,* ]]; then
    if [[ ",$block_reasons," == *,admin-required,* ]]; then
      if [[ "$handoff_current" == "true" ]]; then
        echo "admin-install-current-handoff"
      else
        echo "run-install-handoff-and-admin-install"
      fi
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
  local handoff_current="${2:-false}"
  local actions=""

  if [[ "$block_reasons" == "none" ]]; then
    echo "none"
    return
  fi

  if [[ ",$block_reasons," == *,system-app-missing,* ||
    ",$block_reasons," == *,system-cdhash-mismatch,* ||
    ",$block_reasons," == *,settings-version-mismatch,* ]]; then
    if [[ ",$block_reasons," == *,admin-required,* ]]; then
      if [[ "$handoff_current" == "true" ]]; then
        actions="$(append_action "$actions" "admin-install-current-handoff")"
      else
        actions="$(append_action "$actions" "run-install-handoff-and-admin-install")"
      fi
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

print_install_required_commands() {
  local required_actions="$1"
  local pkg_path="$2"
  local root_quoted pkg_quoted command_keys

  root_quoted="$(quote "$ROOT_DIR")"
  if [[ -z "$pkg_path" ]]; then
    pkg_path="$ROOT_DIR/dist/InputiaInputMethod-latest.pkg"
  fi
  pkg_quoted="$(quote "$pkg_path")"
  command_keys="$(install_required_command_keys "$required_actions")"

  section "required commands"
  if [[ "$command_keys" == "none" ]]; then
    echo "installCheckRequiredCommands=none"
    return
  fi

  echo "installCheckRequiredCommands=present"
  if [[ ",$command_keys," == *,runInstallHandoff,* ]]; then
    echo "installCheckCommand.runInstallHandoff=cd $root_quoted && ./install-handoff.sh"
  fi
  if [[ ",$command_keys," == *,adminInstall,* ]]; then
    echo "installCheckCommand.adminInstall=sudo /usr/sbin/installer -pkg $pkg_quoted -target /"
  fi
  if [[ ",$command_keys," == *,runInstallSystem,* ]]; then
    echo "installCheckCommand.runInstallSystem=cd $root_quoted && ./install-system.sh"
  fi
  if [[ ",$command_keys," == *,repairTISDuplicates,* ]]; then
    echo "installCheckCommand.repairTISDuplicates=cd $root_quoted && INPUTIA_REPAIR_TIS_DUPLICATES=1 ./repair-tis-duplicates.sh"
  fi
  if [[ ",$command_keys," == *,openKeyboardSettings,* ]]; then
    echo "installCheckCommand.openKeyboardSettings=open 'x-apple.systempreferences:com.apple.Keyboard-Settings.extension'"
  fi
  if [[ ",$command_keys," == *,awaitSystemInstall,* ]]; then
    echo "installCheckCommand.awaitSystemInstall=cd $root_quoted && ./await-system-install.sh"
  fi
  if [[ ",$command_keys," == *,verify,* ]]; then
    echo "installCheckCommand.verify=cd $root_quoted && ./install-check.sh"
  fi
  if [[ ",$command_keys," == *,adminInstall,* ||
    ",$command_keys," == *,repairTISDuplicates,* ||
    ",$command_keys," == *,awaitSystemInstall,* ]]; then
    echo "installCheckCommand.applyCurrentHandoff=cd $root_quoted && INPUTIA_ALLOW_ADMIN_PROMPT=1 ./apply-current-handoff.sh"
  fi
}

install_required_command_keys() {
  local required_actions="$1"
  local command_keys=""

  if [[ "$required_actions" == "none" ]]; then
    echo "none"
    return
  fi

  if [[ ",$required_actions," == *,run-install-handoff-and-admin-install,* ]]; then
    command_keys="$(append_action "$command_keys" "runInstallHandoff")"
    command_keys="$(append_action "$command_keys" "adminInstall")"
  elif [[ ",$required_actions," == *,admin-install-current-handoff,* ]]; then
    command_keys="$(append_action "$command_keys" "adminInstall")"
  elif [[ ",$required_actions," == *,run-install-system,* ]]; then
    command_keys="$(append_action "$command_keys" "runInstallSystem")"
  fi
  if [[ ",$required_actions," == *,run-repair-tis-duplicates,* ]]; then
    command_keys="$(append_action "$command_keys" "repairTISDuplicates")"
  fi
  if [[ ",$required_actions," == *,select-or-readd-inputia-in-system-settings,* ]]; then
    command_keys="$(append_action "$command_keys" "openKeyboardSettings")"
  fi
  if [[ ",$required_actions," == *,restart-inputia-host-after-install,* ]]; then
    command_keys="$(append_action "$command_keys" "awaitSystemInstall")"
  fi
  command_keys="$(append_action "$command_keys" "verify")"

  echo "$command_keys"
}

install_next_step() {
  local required_actions="$1"
  local handoff_current="${2:-false}"

  if [[ "$required_actions" == "none" ]]; then
    echo "none"
  elif [[ "$handoff_current" != "true" ]]; then
    echo "run-install-handoff"
  elif action_present "$required_actions" "admin-install-current-handoff" ||
    action_present "$required_actions" "run-repair-tis-duplicates" ||
    action_present "$required_actions" "restart-inputia-host-after-install"; then
    echo "apply-current-handoff"
  elif action_present "$required_actions" "run-install-system"; then
    echo "run-install-system"
  elif action_present "$required_actions" "select-or-readd-inputia-in-system-settings"; then
    echo "open-keyboard-settings"
  else
    echo "inspect-install-check-output"
  fi
}

print_install_next_step() {
  local required_actions="$1"
  local handoff_current="$2"
  local step root_quoted

  root_quoted="$(quote "$ROOT_DIR")"
  step="$(install_next_step "$required_actions" "$handoff_current")"

  section "next step"
  echo "installCheckNextStep=$step"
  case "$step" in
  none)
    echo "installCheckNextCommand=none"
    ;;
  apply-current-handoff)
    echo "installCheckNextCommand=cd $root_quoted && INPUTIA_ALLOW_ADMIN_PROMPT=1 ./apply-current-handoff.sh"
    ;;
  run-install-handoff)
    echo "installCheckNextCommand=cd $root_quoted && ./install-handoff.sh"
    ;;
  run-install-system)
    echo "installCheckNextCommand=cd $root_quoted && ./install-system.sh"
    ;;
  open-keyboard-settings)
    echo "installCheckNextCommand=open 'x-apple.systempreferences:com.apple.Keyboard-Settings.extension'"
    ;;
  *)
    echo "installCheckNextCommand=cd $root_quoted && ./install-check.sh"
    ;;
  esac
}

install_handoff_block_reasons() {
  local handoff_exists="$1"
  local current_source_commit="$2"
  local current_source_dirty="$3"
  local handoff_source_commit="$4"
  local handoff_source_dirty="$5"
  local build_cdhash="$6"
  local handoff_build_cdhash="$7"
  local handoff_pkg_verification="$8"
  local handoff_package_exists="$9"
  local handoff_package_sha_matches="${10}"
  local block_reasons=""

  if [[ "$handoff_exists" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "missing-handoff")"
  fi
  if [[ "$handoff_source_commit" != "$current_source_commit" ]]; then
    block_reasons="$(append_reason "$block_reasons" "source-commit-mismatch")"
  fi
  if [[ "$current_source_dirty" != "false" ]]; then
    block_reasons="$(append_reason "$block_reasons" "worktree-dirty")"
  fi
  if [[ "$handoff_source_dirty" != "false" ]]; then
    block_reasons="$(append_reason "$block_reasons" "handoff-source-dirty")"
  fi
  if [[ -n "$build_cdhash" && "$handoff_build_cdhash" != "$build_cdhash" ]]; then
    block_reasons="$(append_reason "$block_reasons" "build-cdhash-mismatch")"
  fi
  if [[ "$handoff_pkg_verification" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "pkg-verification-missing")"
  fi
  if [[ "$handoff_package_exists" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "package-missing")"
  elif [[ "$handoff_package_sha_matches" != "true" ]]; then
    block_reasons="$(append_reason "$block_reasons" "package-sha-mismatch")"
  fi
  if [[ -z "$block_reasons" ]]; then
    block_reasons=none
  fi
  echo "$block_reasons"
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
  local handoff_current="${10}"
  local expected_reasons="${11}"
  local expected_action="${12}"
  local expected_actions="${13}"
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
  actual_action="$(install_required_action "$actual_reasons" "$handoff_current")"
  actual_actions="$(install_required_actions "$actual_reasons" "$handoff_current")"
  echo "installCheckSelfCheck case=$label handoffCurrent=$handoff_current reasons=$actual_reasons action=$actual_action actions=$actual_actions"
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

run_install_handoff_self_check_case() {
  local label="$1"
  local handoff_exists="$2"
  local current_source_commit="$3"
  local current_source_dirty="$4"
  local handoff_source_commit="$5"
  local handoff_source_dirty="$6"
  local build_cdhash="$7"
  local handoff_build_cdhash="$8"
  local handoff_pkg_verification="$9"
  local handoff_package_exists="${10}"
  local handoff_package_sha_matches="${11}"
  local expected_reasons="${12}"
  local actual_reasons

  actual_reasons="$(
    install_handoff_block_reasons \
      "$handoff_exists" \
      "$current_source_commit" \
      "$current_source_dirty" \
      "$handoff_source_commit" \
      "$handoff_source_dirty" \
      "$build_cdhash" \
      "$handoff_build_cdhash" \
      "$handoff_pkg_verification" \
      "$handoff_package_exists" \
      "$handoff_package_sha_matches"
  )"
  echo "installHandoffFreshnessSelfCheck case=$label reasons=$actual_reasons"
  if [[ "$actual_reasons" != "$expected_reasons" ]]; then
    echo "installHandoffFreshnessSelfCheck=false case=$label reason=reasons-mismatch expected=$expected_reasons actual=$actual_reasons"
    exit 1
  fi
}

run_install_required_commands_self_check_case() {
  local label="$1"
  local required_actions="$2"
  local expected_command_keys="$3"
  local actual_command_keys

  actual_command_keys="$(install_required_command_keys "$required_actions")"
  echo "installRequiredCommandsSelfCheck case=$label commandKeys=$actual_command_keys"
  if [[ "$actual_command_keys" != "$expected_command_keys" ]]; then
    echo "installRequiredCommandsSelfCheck=false case=$label reason=command-keys-mismatch expected=$expected_command_keys actual=$actual_command_keys"
    exit 1
  fi
}

run_install_next_step_self_check_case() {
  local label="$1"
  local required_actions="$2"
  local handoff_current="$3"
  local expected_step="$4"
  local actual_step

  actual_step="$(install_next_step "$required_actions" "$handoff_current")"
  echo "installNextStepSelfCheck case=$label step=$actual_step"
  if [[ "$actual_step" != "$expected_step" ]]; then
    echo "installNextStepSelfCheck=false case=$label reason=step-mismatch expected=$expected_step actual=$actual_step"
    exit 1
  fi
}

if [[ "${INPUTIA_INSTALL_CHECK_SELF_CHECK:-0}" == "1" ]]; then
  run_install_check_self_check_case \
    ready true true true true false true true true false \
    none none none
  run_install_check_self_check_case \
    admin-required true false true true false true false false false \
    system-cdhash-mismatch,running-cdhash-mismatch,admin-required \
    run-install-handoff-and-admin-install \
    run-install-handoff-and-admin-install,restart-inputia-host-after-install
  run_install_check_self_check_case \
    admin-required-current-handoff true false true true false true false false true \
    system-cdhash-mismatch,running-cdhash-mismatch,admin-required \
    admin-install-current-handoff \
    admin-install-current-handoff,restart-inputia-host-after-install
  run_install_check_self_check_case \
    tis-not-ready true true true false false true true true false \
    tis-not-ready \
    select-or-readd-inputia-in-system-settings \
    select-or-readd-inputia-in-system-settings
  run_install_check_self_check_case \
    tis-duplicate true true true false true true true true false \
    tis-duplicate-matches \
    remove-duplicate-inputia-and-readd-once \
    run-repair-tis-duplicates
  run_install_check_self_check_case \
    running-missing true true true true false false false true false \
    running-host-missing \
    restart-inputia-host-after-install \
    restart-inputia-host-after-install
  run_install_check_self_check_case \
    settings-admin true true false true false true true false false \
    settings-version-mismatch,admin-required \
    run-install-handoff-and-admin-install \
    run-install-handoff-and-admin-install
  run_install_check_self_check_case \
    settings-admin-current-handoff true true false true false true true false true \
    settings-version-mismatch,admin-required \
    admin-install-current-handoff \
    admin-install-current-handoff
  run_install_check_self_check_case \
    system-missing false false true false false false false false false \
    system-app-missing,tis-not-ready,running-host-missing,admin-required \
    run-install-handoff-and-admin-install \
    run-install-handoff-and-admin-install,select-or-readd-inputia-in-system-settings,restart-inputia-host-after-install
  run_install_handoff_self_check_case \
    handoff-ready true source123 false source123 false cdhash123 cdhash123 true true true \
    none
  run_install_handoff_self_check_case \
    handoff-missing false source123 false source123 false cdhash123 cdhash123 true false true \
    missing-handoff,package-missing
  run_install_handoff_self_check_case \
    handoff-source-commit-stale true source123 false source122 false cdhash123 cdhash123 true true true \
    source-commit-mismatch
  run_install_handoff_self_check_case \
    handoff-worktree-dirty true source123 true source123 false cdhash123 cdhash123 true true true \
    worktree-dirty
  run_install_handoff_self_check_case \
    handoff-source-dirty true source123 false source123 true cdhash123 cdhash123 true true true \
    handoff-source-dirty
  run_install_handoff_self_check_case \
    handoff-build-stale true source123 false source123 false cdhash123 cdhash122 true true true \
    build-cdhash-mismatch
  run_install_handoff_self_check_case \
    handoff-pkg-verification-missing true source123 false source123 false cdhash123 cdhash123 false true true \
    pkg-verification-missing
  run_install_handoff_self_check_case \
    handoff-package-missing true source123 false source123 false cdhash123 cdhash123 true false true \
    package-missing
  run_install_handoff_self_check_case \
    handoff-package-sha-mismatch true source123 false source123 false cdhash123 cdhash123 true true false \
    package-sha-mismatch
  run_install_required_commands_self_check_case \
    no-commands none none
  run_install_required_commands_self_check_case \
    admin-install run-install-handoff-and-admin-install \
    runInstallHandoff,adminInstall,verify
  run_install_required_commands_self_check_case \
    admin-install-current-handoff admin-install-current-handoff \
    adminInstall,verify
  run_install_required_commands_self_check_case \
    admin-repair-restart run-install-handoff-and-admin-install,run-repair-tis-duplicates,restart-inputia-host-after-install \
    runInstallHandoff,adminInstall,repairTISDuplicates,awaitSystemInstall,verify
  run_install_required_commands_self_check_case \
    admin-current-handoff-repair-restart admin-install-current-handoff,run-repair-tis-duplicates,restart-inputia-host-after-install \
    adminInstall,repairTISDuplicates,awaitSystemInstall,verify
  run_install_required_commands_self_check_case \
    system-install run-install-system \
    runInstallSystem,verify
  run_install_required_commands_self_check_case \
    repair-and-restart run-repair-tis-duplicates,restart-inputia-host-after-install \
    repairTISDuplicates,awaitSystemInstall,verify
  run_install_required_commands_self_check_case \
    settings-and-restart select-or-readd-inputia-in-system-settings,restart-inputia-host-after-install \
    openKeyboardSettings,awaitSystemInstall,verify
  run_install_next_step_self_check_case \
    ready none true none
  run_install_next_step_self_check_case \
    stale-handoff run-install-handoff-and-admin-install false run-install-handoff
  run_install_next_step_self_check_case \
    current-admin-handoff admin-install-current-handoff,run-repair-tis-duplicates,restart-inputia-host-after-install true apply-current-handoff
  run_install_next_step_self_check_case \
    current-repair-only run-repair-tis-duplicates,restart-inputia-host-after-install true apply-current-handoff
  run_install_next_step_self_check_case \
    system-install run-install-system true run-install-system
  run_install_next_step_self_check_case \
    settings-manual select-or-readd-inputia-in-system-settings true open-keyboard-settings
  echo "installHandoffFreshnessSelfCheck=true"
  echo "installRequiredCommandsSelfCheck=true"
  echo "installNextStepSelfCheck=true"
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
build_source_commit="$(app_source_commit "$BUILD_APP")"
system_source_commit="$(app_source_commit "$SYSTEM_APP")"
build_source_dirty="$(app_source_dirty "$BUILD_APP")"
system_source_dirty="$(app_source_dirty "$SYSTEM_APP")"
build_cdhash="$(app_cdhash "$BUILD_APP")"
system_cdhash="$(app_cdhash "$SYSTEM_APP")"
echo "buildApp=$BUILD_APP"
echo "buildVersion=${build_version:-unknown}"
echo "buildSourceCommit=${build_source_commit:-unknown}"
echo "buildSourceDirty=${build_source_dirty:-unknown}"
echo "buildCDHash=${build_cdhash:-unknown}"
echo "systemApp=$SYSTEM_APP"
echo "systemExists=$([[ -d "$SYSTEM_APP" ]] && echo true || echo false)"
echo "systemVersion=${system_version:-unknown}"
echo "systemSourceCommit=${system_source_commit:-unknown}"
echo "systemSourceDirty=${system_source_dirty:-unknown}"
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
build_settings_source_commit="$(app_source_commit "$BUILD_SETTINGS_APP")"
system_settings_source_commit="$(app_source_commit "$SYSTEM_SETTINGS_APP")"
build_settings_source_dirty="$(app_source_dirty "$BUILD_SETTINGS_APP")"
system_settings_source_dirty="$(app_source_dirty "$SYSTEM_SETTINGS_APP")"
build_settings_expected_host_cdhash="$(plist_value "$BUILD_SETTINGS_APP/Contents/Info.plist" InputiaExpectedHostCDHash)"
system_settings_expected_host_cdhash="$(plist_value "$SYSTEM_SETTINGS_APP/Contents/Info.plist" InputiaExpectedHostCDHash)"
echo "buildSettingsVersion=${build_settings_version:-unknown}"
echo "buildSettingsSourceCommit=${build_settings_source_commit:-unknown}"
echo "buildSettingsSourceDirty=${build_settings_source_dirty:-unknown}"
echo "buildSettingsExpectedHostCDHash=${build_settings_expected_host_cdhash:-unknown}"
echo "systemSettingsApp=$SYSTEM_SETTINGS_APP"
echo "systemSettingsExists=$([[ -d "$SYSTEM_SETTINGS_APP" ]] && echo true || echo false)"
echo "systemSettingsVersion=${system_settings_version:-unknown}"
echo "systemSettingsSourceCommit=${system_settings_source_commit:-unknown}"
echo "systemSettingsSourceDirty=${system_settings_source_dirty:-unknown}"
echo "systemSettingsExpectedHostCDHash=${system_settings_expected_host_cdhash:-unknown}"
if [[ -n "${build_settings_version:-}" &&
  "$system_settings_version" == "$build_settings_version" &&
  -n "${build_cdhash:-}" &&
  "$build_settings_expected_host_cdhash" == "$build_cdhash" &&
  "$system_settings_expected_host_cdhash" == "$build_cdhash" ]]; then
  settings_matches_build=true
else
  settings_matches_build=false
fi
echo "settingsMatchesBuild=$settings_matches_build"

section "install handoff"
current_source_commit="$(git_value unknown rev-parse --short=12 HEAD)"
current_source_dirty="$(git_dirty_state)"
handoff_source_commit="$(handoff_value sourceCommit)"
handoff_source_dirty="$(handoff_value sourceDirty)"
handoff_build_cdhash="$(handoff_value buildCDHash)"
handoff_package_path="$(handoff_value packagePath)"
handoff_package_sha256="$(handoff_value packageSHA256)"
handoff_pkg_verification="$(handoff_value pkgVerificationPassed)"
handoff_exists="$([[ -f "$HANDOFF_PATH" ]] && echo true || echo false)"
handoff_package_exists=false
handoff_package_sha_matches=true
echo "installHandoffPath=$HANDOFF_PATH"
echo "installHandoffExists=$handoff_exists"
echo "installHandoffSourceCommit=${handoff_source_commit:-unknown}"
echo "installHandoffCurrentSourceCommit=$current_source_commit"
echo "installHandoffSourceDirty=${handoff_source_dirty:-unknown}"
echo "installHandoffCurrentSourceDirty=$current_source_dirty"
echo "installHandoffBuildCDHash=${handoff_build_cdhash:-unknown}"
echo "installHandoffPackagePath=${handoff_package_path:-unknown}"
echo "installHandoffPackageSHA256=${handoff_package_sha256:-unknown}"
echo "installHandoffPkgVerificationPassed=${handoff_pkg_verification:-unknown}"
if [[ -n "$handoff_package_path" && -f "$handoff_package_path" ]]; then
  handoff_package_exists=true
  actual_package_sha256="$(sha256 "$handoff_package_path")"
  echo "installHandoffActualPackageSHA256=${actual_package_sha256:-unknown}"
  if [[ -n "$handoff_package_sha256" && "$actual_package_sha256" != "$handoff_package_sha256" ]]; then
    handoff_package_sha_matches=false
  fi
fi
handoff_block_reasons="$(
  install_handoff_block_reasons \
    "$handoff_exists" \
    "$current_source_commit" \
    "$current_source_dirty" \
    "$handoff_source_commit" \
    "$handoff_source_dirty" \
    "${build_cdhash:-}" \
    "$handoff_build_cdhash" \
    "$handoff_pkg_verification" \
    "$handoff_package_exists" \
    "$handoff_package_sha_matches"
)"
if [[ "$handoff_block_reasons" == "none" ]]; then
  handoff_current=true
  echo "installHandoffCurrent=true"
  echo "installHandoffRequiredAction=none"
else
  handoff_current=false
  echo "installHandoffCurrent=false"
  echo "installHandoffRequiredAction=run-install-handoff"
fi
echo "installHandoffBlockReasons=$handoff_block_reasons"

section "tis"
tis_output="$(INPUTIA_TIS_INCLUDE_MENU_READINESS=0 "$ROOT_DIR/tis-readiness.sh" "$SYSTEM_APP" 2>&1 || true)"
printf '%s\n' "$tis_output" | /usr/bin/awk '
  /^app=|^appExists=|^buildCDHash=|^appCDHash=|^appMatchesBuild=|^expectedTISModeID=|^tis.targetEnabledMatches=|^tis.targetInstalledMatches=|^tis.targetDuplicateMatches=|^tis.targetSourceCount=|^tis.targetEnabledSourceCount=|^tis.targetEnabledUniqueFingerprintCount=|^tis.targetEnabledDuplicateFingerprintCount=|^tis.targetEnabledDuplicateFingerprint=|^tis.targetInstalledSourceCount=|^tis.targetInstalledUniqueFingerprintCount=|^tis.targetInstalledDuplicateFingerprintCount=|^tis.targetInstalledDuplicateFingerprint=|^tis.targetSource\.[0-9]+\.|^tis.hansIconMatchesApp=|^tis.hansEnabled=|^tis.hansSelectable=|^tis.hansSelected=|^tis.currentID=|^tis.currentMatchesTarget=|^tis.menuReadiness=|^tis.menuBlockReason=|^tis.readinessBlockReason=|^tis.requiredAction=|^tisReadiness=/ { print }
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
  required_actions=none
else
  echo "installCheckRequiredAction=$(install_required_action "$block_reasons" "$handoff_current")"
  required_actions="$(install_required_actions "$block_reasons" "$handoff_current")"
  echo "installCheckRequiredActions=$required_actions"
fi
print_install_required_commands "$required_actions" "$handoff_package_path"
print_install_next_step "$required_actions" "$handoff_current"
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
