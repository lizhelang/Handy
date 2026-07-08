#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
HANDOFF_PATH="${INPUTIA_INSTALL_HANDOFF_PATH:-$ROOT_DIR/build/install-handoff.txt}"

section() {
  printf '\n== %s ==\n' "$1"
}

value_from_output() {
  local output="$1"
  local key="$2"
  /usr/bin/awk -F= -v key="$key" '$1 == key { print $2; found = 1; exit } END { if (!found) print "unknown" }' <<<"$output"
}

contains_action() {
  local actions="$1"
  local action="$2"
  [[ ",$actions," == *",$action,"* ]]
}

line_number() {
  local output="$1"
  local pattern="$2"
  /usr/bin/awk -v pattern="$pattern" '
    index($0, pattern) { print NR; found = 1; exit }
    END { if (!found) print 0 }
  ' <<<"$output"
}

quote() {
  /usr/bin/python3 - "$1" <<'PY'
import shlex
import sys

print(shlex.quote(sys.argv[1]))
PY
}

run_install_check() {
  if [[ -n "${INPUTIA_APPLY_INSTALL_CHECK_FOR_TEST:-}" ]]; then
    printf '%s\n' "$INPUTIA_APPLY_INSTALL_CHECK_FOR_TEST"
    return 0
  fi
  "$ROOT_DIR/install-check.sh" 2>&1 || true
}

run_final_install_check() {
  if [[ -n "${INPUTIA_APPLY_FINAL_INSTALL_CHECK_FOR_TEST:-}" ]]; then
    printf '%s\n' "$INPUTIA_APPLY_FINAL_INSTALL_CHECK_FOR_TEST"
    return "${INPUTIA_APPLY_FINAL_INSTALL_CHECK_RC_FOR_TEST:-0}"
  fi
  "$ROOT_DIR/install-check.sh" 2>&1
}

run_repair_tis_duplicates() {
  if [[ "${INPUTIA_APPLY_REPAIR_TIS_FOR_TEST:-0}" == "1" ]]; then
    echo "applyCurrentHandoffRepairTISForTest=true"
    return 0
  fi
  INPUTIA_REPAIR_TIS_DUPLICATES=1 "$ROOT_DIR/repair-tis-duplicates.sh"
}

run_await_system_install() {
  if [[ "${INPUTIA_APPLY_AWAIT_INSTALL_FOR_TEST:-0}" == "1" ]]; then
    echo "applyCurrentHandoffAwaitForTest=true"
    return 0
  fi
  "$ROOT_DIR/await-system-install.sh"
}

print_install_check_summary() {
  local label="$1"
  local output="$2"
  for key in \
    installHandoffCurrent \
    installHandoffBlockReasons \
    installHandoffPackagePath \
    installCheckBlockReasons \
    installCheckRequiredAction \
    installCheckRequiredActions \
    installCheckPassed; do
    echo "$label.$key=$(value_from_output "$output" "$key")"
  done
}

admin_install_command() {
  local pkg_path="$1"
  echo "cd $(quote "$ROOT_DIR") && INPUTIA_ALLOW_ADMIN_PROMPT=1 ./apply-current-handoff.sh"
  echo "sudo /usr/sbin/installer -pkg $(quote "$pkg_path") -target /"
}

run_admin_installer() {
  local pkg_path="$1"

  if [[ ! -f "$pkg_path" ]]; then
    echo "applyCurrentHandoffReady=false reason=missing-pkg path=$pkg_path"
    exit 10
  fi

  echo "applyCurrentHandoffInstallerPackage=$pkg_path"
  if [[ "${INPUTIA_APPLY_ADMIN_INSTALL_FOR_TEST:-0}" == "1" ]]; then
    echo "applyCurrentHandoffAdminInstallForTest=true"
    return 0
  elif [[ "${INPUTIA_APPLY_FORCE_ADMIN_REQUIRED_FOR_TEST:-0}" == "1" ]]; then
    echo "applyCurrentHandoffReady=false reason=admin-required"
    echo "applyCurrentHandoffRequiredAction=rerun-with-admin-prompt"
    echo "applyCurrentHandoffCommand=$(admin_install_command "$pkg_path" | /usr/bin/head -n 1)"
    exit 12
  elif [[ "$EUID" == "0" ]]; then
    /usr/sbin/installer -pkg "$pkg_path" -target /
  elif [[ -n "${INPUTIA_SUDO_PASSWORD:-}" ]]; then
    /usr/bin/printf '%s\n' "$INPUTIA_SUDO_PASSWORD" |
      /usr/bin/sudo -S -p '' /usr/sbin/installer -pkg "$pkg_path" -target /
  elif /usr/bin/sudo -n true >/dev/null 2>&1; then
    /usr/bin/sudo -n /usr/sbin/installer -pkg "$pkg_path" -target /
  elif [[ "${INPUTIA_ALLOW_ADMIN_PROMPT:-0}" == "1" ]]; then
    /usr/bin/sudo /usr/sbin/installer -pkg "$pkg_path" -target /
  else
    echo "applyCurrentHandoffReady=false reason=admin-required"
    echo "applyCurrentHandoffRequiredAction=rerun-with-admin-prompt"
    echo "applyCurrentHandoffCommand=$(admin_install_command "$pkg_path" | /usr/bin/head -n 1)"
    exit 12
  fi
}

if [[ "${INPUTIA_APPLY_CURRENT_HANDOFF_SELF_CHECK:-0}" == "1" ]]; then
  sample_actions="admin-install-current-handoff,run-repair-tis-duplicates,restart-inputia-host-after-install"
  if ! contains_action "$sample_actions" "admin-install-current-handoff"; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-admin-action-match"
    exit 1
  fi
  if ! contains_action "$sample_actions" "run-repair-tis-duplicates"; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-repair-action-match"
    exit 1
  fi
  command_sample="$(admin_install_command "/tmp/Inputia InputMethod.pkg")"
  command_sample="${command_sample%%$'\n'*}"
  if [[ "$command_sample" != *"INPUTIA_ALLOW_ADMIN_PROMPT=1 ./apply-current-handoff.sh"* ]]; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-admin-prompt-command"
    exit 1
  fi
  self_check_root="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-apply-handoff-self-check.XXXXXX")"
  self_check_handoff="$self_check_root/install-handoff.txt"
  self_check_pkg="$self_check_root/InputiaInputMethod.pkg"
  : > "$self_check_handoff"
  : > "$self_check_pkg"
  no_admin_output="$(
    INPUTIA_APPLY_CURRENT_HANDOFF_SELF_CHECK=0 \
      INPUTIA_INSTALL_HANDOFF_PATH="$self_check_handoff" \
      INPUTIA_APPLY_FORCE_ADMIN_REQUIRED_FOR_TEST=1 \
      INPUTIA_APPLY_INSTALL_CHECK_FOR_TEST=$'installHandoffCurrent=true\ninstallHandoffBlockReasons=none\ninstallHandoffPackagePath='"$self_check_pkg"$'\ninstallCheckBlockReasons=system-cdhash-mismatch,admin-required\ninstallCheckRequiredAction=admin-install-current-handoff\ninstallCheckRequiredActions=admin-install-current-handoff,run-repair-tis-duplicates,restart-inputia-host-after-install\ninstallCheckPassed=false\n' \
      "$0" 2>&1 || true
  )"
  /bin/rm -rf "$self_check_root" >/dev/null 2>&1 || true
  if ! /usr/bin/grep -q '^applyCurrentHandoffReady=false reason=admin-required$' <<<"$no_admin_output"; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-no-admin-ready-marker"
    exit 1
  fi
  if ! /usr/bin/grep -q '^applyCurrentHandoffRequiredAction=rerun-with-admin-prompt$' <<<"$no_admin_output"; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-no-admin-required-action"
    exit 1
  fi
  if ! /usr/bin/grep -q '^applyCurrentHandoffCommand=.*INPUTIA_ALLOW_ADMIN_PROMPT=1 ./apply-current-handoff.sh' <<<"$no_admin_output"; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-no-admin-command"
    exit 1
  fi
  success_root="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-apply-handoff-success-self-check.XXXXXX")"
  success_handoff="$success_root/install-handoff.txt"
  success_pkg="$success_root/InputiaInputMethod.pkg"
  : > "$success_handoff"
  : > "$success_pkg"
  success_output="$(
    INPUTIA_APPLY_CURRENT_HANDOFF_SELF_CHECK=0 \
      INPUTIA_INSTALL_HANDOFF_PATH="$success_handoff" \
      INPUTIA_APPLY_ADMIN_INSTALL_FOR_TEST=1 \
      INPUTIA_APPLY_REPAIR_TIS_FOR_TEST=1 \
      INPUTIA_APPLY_AWAIT_INSTALL_FOR_TEST=1 \
      INPUTIA_APPLY_INSTALL_CHECK_FOR_TEST=$'installHandoffCurrent=true\ninstallHandoffBlockReasons=none\ninstallHandoffPackagePath='"$success_pkg"$'\ninstallCheckBlockReasons=system-cdhash-mismatch,tis-duplicate-matches,running-cdhash-mismatch,admin-required\ninstallCheckRequiredAction=admin-install-current-handoff\ninstallCheckRequiredActions=admin-install-current-handoff,run-repair-tis-duplicates,restart-inputia-host-after-install\ninstallCheckPassed=false\n' \
      INPUTIA_APPLY_FINAL_INSTALL_CHECK_FOR_TEST=$'installCheckPassed=true\ninstallCheckRequiredAction=none\ninstallCheckRequiredActions=none\ninstallCheckNextStep=none\n' \
      INPUTIA_APPLY_FINAL_INSTALL_CHECK_RC_FOR_TEST=0 \
      "$0" 2>&1
  )"
  /bin/rm -rf "$success_root" >/dev/null 2>&1 || true
  for marker in \
    applyCurrentHandoffAdminInstallForTest=true \
    applyCurrentHandoffAdminInstallPassed=true \
    applyCurrentHandoffRepairTISForTest=true \
    applyCurrentHandoffRepairTISPassed=true \
    applyCurrentHandoffAwaitForTest=true \
    applyCurrentHandoffPassed=true; do
    if ! /usr/bin/grep -q "^$marker$" <<<"$success_output"; then
      echo "applyCurrentHandoffSelfCheck=false reason=missing-success-marker marker=$marker"
      exit 1
    fi
  done
  admin_line="$(line_number "$success_output" "applyCurrentHandoffAdminInstallForTest=true")"
  repair_line="$(line_number "$success_output" "applyCurrentHandoffRepairTISForTest=true")"
  await_line="$(line_number "$success_output" "applyCurrentHandoffAwaitForTest=true")"
  final_line="$(line_number "$success_output" "== final install check ==")"
  passed_line="$(line_number "$success_output" "applyCurrentHandoffPassed=true")"
  if (( admin_line == 0 || repair_line <= admin_line || await_line <= repair_line || final_line <= await_line || passed_line <= final_line )); then
    echo "applyCurrentHandoffSelfCheck=false reason=success-chain-order-mismatch"
    echo "applyCurrentHandoffSelfCheckOrder=admin:$admin_line repair:$repair_line await:$await_line final:$final_line passed:$passed_line"
    exit 1
  fi
  final_failure_output="$(
    INPUTIA_APPLY_CURRENT_HANDOFF_SELF_CHECK=0 \
      INPUTIA_APPLY_FINAL_INSTALL_CHECK_FOR_TEST=$'installCheckPassed=false\ninstallCheckRequiredAction=restart-inputia-host-after-install\ninstallCheckRequiredActions=restart-inputia-host-after-install\ninstallCheckNextStep=apply-current-handoff\n' \
      INPUTIA_APPLY_FINAL_INSTALL_CHECK_RC_FOR_TEST=1 \
      "$0" --final-check-self-test 2>&1 || true
  )"
  if ! /usr/bin/grep -q '^applyCurrentHandoffPassed=false reason=final-install-check-failed$' <<<"$final_failure_output"; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-final-failure-marker"
    exit 1
  fi
  if ! /usr/bin/grep -q '^applyCurrentHandoffRequiredAction=restart-inputia-host-after-install$' <<<"$final_failure_output"; then
    echo "applyCurrentHandoffSelfCheck=false reason=missing-final-required-action"
    exit 1
  fi
  echo "applyCurrentHandoffSelfCheck=true"
  exit 0
fi

if [[ "${1:-}" == "--final-check-self-test" ]]; then
  section "final install check"
  set +e
  final_check="$(run_final_install_check)"
  final_check_rc=$?
  set -e
  printf '%s\n' "$final_check"
  final_passed="$(value_from_output "$final_check" "installCheckPassed")"
  if [[ "$final_check_rc" == "0" && "$final_passed" == "true" ]]; then
    echo "applyCurrentHandoffPassed=true"
    exit 0
  fi
  echo "applyCurrentHandoffPassed=false reason=final-install-check-failed"
  echo "applyCurrentHandoffFinalInstallCheckExit=$final_check_rc"
  echo "applyCurrentHandoffRequiredAction=$(value_from_output "$final_check" "installCheckRequiredAction")"
  echo "applyCurrentHandoffRequiredActions=$(value_from_output "$final_check" "installCheckRequiredActions")"
  echo "applyCurrentHandoffNextStep=$(value_from_output "$final_check" "installCheckNextStep")"
  exit 13
fi

section "policy"
echo "validationTier=install-apply"
echo "touchesMenuBar=false"
echo "opensGUI=false"
echo "changesSystemInputSource=true"
echo "checksNotarization=false"

section "preflight"
echo "installHandoffPath=$HANDOFF_PATH"
if [[ ! -f "$HANDOFF_PATH" ]]; then
  echo "applyCurrentHandoffReady=false reason=missing-handoff"
  echo "applyCurrentHandoffRequiredAction=run-install-handoff"
  echo "applyCurrentHandoffCommand=cd $(quote "$ROOT_DIR") && ./install-handoff.sh"
  exit 11
fi

initial_check="$(run_install_check)"
print_install_check_summary "initial" "$initial_check"

handoff_current="$(value_from_output "$initial_check" "installHandoffCurrent")"
required_actions="$(value_from_output "$initial_check" "installCheckRequiredActions")"
pkg_path="$(value_from_output "$initial_check" "installHandoffPackagePath")"

if [[ "$handoff_current" != "true" ]]; then
  echo "applyCurrentHandoffReady=false reason=handoff-not-current"
  echo "applyCurrentHandoffRequiredAction=run-install-handoff"
  echo "applyCurrentHandoffCommand=cd $(quote "$ROOT_DIR") && ./install-handoff.sh"
  exit 11
fi

if [[ "$required_actions" == "none" ]]; then
  echo "applyCurrentHandoffReady=true reason=already-current"
  echo "applyCurrentHandoffPassed=true"
  exit 0
fi

section "admin install"
if contains_action "$required_actions" "admin-install-current-handoff"; then
  run_admin_installer "$pkg_path"
  echo "applyCurrentHandoffAdminInstallPassed=true"
elif contains_action "$required_actions" "run-install-handoff-and-admin-install"; then
  echo "applyCurrentHandoffReady=false reason=handoff-not-current"
  echo "applyCurrentHandoffRequiredAction=run-install-handoff"
  exit 11
else
  echo "applyCurrentHandoffAdminInstallSkipped=true"
fi

after_install_check="$(run_install_check)"
print_install_check_summary "afterInstall" "$after_install_check"
after_install_actions="$(value_from_output "$after_install_check" "installCheckRequiredActions")"

section "tis duplicate repair"
if contains_action "$after_install_actions" "run-repair-tis-duplicates"; then
  run_repair_tis_duplicates
  echo "applyCurrentHandoffRepairTISPassed=true"
else
  echo "applyCurrentHandoffRepairTISSkipped=true"
fi

section "await system install"
run_await_system_install

section "final install check"
set +e
final_check="$(run_final_install_check)"
final_check_rc=$?
set -e
printf '%s\n' "$final_check"
final_passed="$(value_from_output "$final_check" "installCheckPassed")"
if [[ "$final_check_rc" == "0" && "$final_passed" == "true" ]]; then
  echo "applyCurrentHandoffPassed=true"
else
  echo "applyCurrentHandoffPassed=false reason=final-install-check-failed"
  echo "applyCurrentHandoffFinalInstallCheckExit=$final_check_rc"
  echo "applyCurrentHandoffRequiredAction=$(value_from_output "$final_check" "installCheckRequiredAction")"
  echo "applyCurrentHandoffRequiredActions=$(value_from_output "$final_check" "installCheckRequiredActions")"
  echo "applyCurrentHandoffNextStep=$(value_from_output "$final_check" "installCheckNextStep")"
  exit 13
fi
