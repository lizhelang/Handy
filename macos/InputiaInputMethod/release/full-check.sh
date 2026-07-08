#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
MENU_CACHE="${TMPDIR:-/tmp}/inputia-menu-readiness.$$.$RANDOM.cache"
export INPUTIA_VERIFICATION_OWNER_PID="${INPUTIA_VERIFICATION_OWNER_PID:-$$}"
export INPUTIA_MENU_READINESS_CACHE_FILE="$MENU_CACHE"

section() {
  printf '\n== %s ==\n' "$1"
}

emit_test_output() {
  local variable_name="$1"
  if [[ -n "${!variable_name+x}" ]]; then
    printf '%s\n' "${!variable_name}"
    return 0
  fi
  return 1
}

run_build_pkg() {
  if emit_test_output INPUTIA_FULL_CHECK_BUILD_PKG_OUTPUT_FOR_TEST; then
    return 0
  fi
  "$ROOT_DIR/build-pkg.sh"
}

run_verify_pkg() {
  if emit_test_output INPUTIA_FULL_CHECK_VERIFY_PKG_OUTPUT_FOR_TEST; then
    return 0
  fi
  "$ROOT_DIR/verify-pkg.sh"
}

run_install_check() {
  if emit_test_output INPUTIA_FULL_CHECK_INSTALL_CHECK_OUTPUT_FOR_TEST; then
    return 0
  fi
  "$ROOT_DIR/install-check.sh" 2>&1 || true
}

run_notarization_readiness() {
  if emit_test_output INPUTIA_FULL_CHECK_NOTARIZATION_OUTPUT_FOR_TEST; then
    return 0
  fi
  "$ROOT_DIR/notarization-readiness.sh" "$ROOT_DIR/build/InputiaInputMethod.app" 2>&1
}

enable_heavy_checks() {
  export INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1
  export INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1
  export INPUTIA_TIS_INCLUDE_MENU_READINESS=1
  export INPUTIA_STATUS_INCLUDE_MENU_READINESS=1
  export INPUTIA_STATUS_INCLUDE_GUI_SMOKE_READINESS=1
  export INPUTIA_RUN_UI_SMOKE=1
  echo "fullCheckHeavyOptInEnabled=true"
}

run_menu_readiness() {
  if [[ "${INPUTIA_FULL_CHECK_MENU_READINESS_FOR_TEST:-0}" == "1" ]]; then
    echo "fullCheckMenuReadinessForTest=true"
    echo "fullCheckMenuReadinessOptIn=${INPUTIA_MENU_READINESS_ALLOW_AXPRESS:-0}"
    echo "fullCheckMenuReadinessCacheFile=${INPUTIA_MENU_READINESS_CACHE_FILE:-}"
    if [[ "${INPUTIA_MENU_READINESS_ALLOW_AXPRESS:-0}" != "1" ]]; then
      echo "fullCheckMenuReadinessForTest=false reason=missing-menu-opt-in"
      return 31
    fi
    if [[ -z "${INPUTIA_MENU_READINESS_CACHE_FILE:-}" ]]; then
      echo "fullCheckMenuReadinessForTest=false reason=missing-menu-cache"
      return 32
    fi
    return 0
  fi
  "$ROOT_DIR/menu-readiness.sh"
}

run_post_install_regression() {
  if [[ "${INPUTIA_FULL_CHECK_POST_INSTALL_FOR_TEST:-0}" == "1" ]]; then
    echo "fullCheckPostInstallForTest=true"
    echo "fullCheckPostInstallGuiOptIn=${INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK:-0}"
    echo "fullCheckPostInstallUiSmoke=${INPUTIA_RUN_UI_SMOKE:-0}"
    echo "fullCheckPostInstallMenuCacheFile=${INPUTIA_MENU_READINESS_CACHE_FILE:-}"
    if [[ "${INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK:-0}" != "1" ||
      "${INPUTIA_RUN_UI_SMOKE:-0}" != "1" ]]; then
      echo "fullCheckPostInstallForTest=false reason=missing-gui-opt-in"
      return 33
    fi
    if [[ -z "${INPUTIA_MENU_READINESS_CACHE_FILE:-}" ]]; then
      echo "fullCheckPostInstallForTest=false reason=missing-menu-cache"
      return 34
    fi
    return 0
  fi
  "$ROOT_DIR/post-install-regression.sh" "$APP"
}

cleanup() {
  /bin/rm -f "$MENU_CACHE" >/dev/null 2>&1 || true
}

require_self_check_output() {
  local output="$1"
  local pattern="$2"
  local reason="$3"
  if ! /usr/bin/grep -q "$pattern" <<<"$output"; then
    echo "fullCheckSelfCheck=false reason=$reason"
    exit 90
  fi
}

require_self_check_no_output() {
  local output="$1"
  local pattern="$2"
  local reason="$3"
  if /usr/bin/grep -q "$pattern" <<<"$output"; then
    echo "fullCheckSelfCheck=false reason=$reason"
    exit 91
  fi
}

run_self_check_case() {
  local label="$1"
  local expected_rc="$2"
  shift 2

  local output rc
  set +e
  output="$("$@" 2>&1)"
  rc=$?
  set -e
  printf '%s\n' "$output" | /usr/bin/sed "s/^/fullCheckSelfCheck case=$label /"
  echo "fullCheckSelfCheck case=$label rc=$rc"
  if [[ "$rc" != "$expected_rc" ]]; then
    echo "fullCheckSelfCheck=false case=$label reason=unexpected-rc expected=$expected_rc actual=$rc"
    exit 92
  fi
  FULL_CHECK_SELF_CHECK_OUTPUT="$output"
}

run_self_check() {
  local temp_dir
  temp_dir="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-full-check-self.XXXXXX")"
  trap "/bin/rm -rf '$temp_dir' >/dev/null 2>&1 || true" EXIT

  run_self_check_case install-gate 12 \
    /usr/bin/env TMPDIR="$temp_dir" \
    INPUTIA_FULL_CHECK_SELF_CHECK=0 \
    INPUTIA_FULL_CHECK_BUILD_PKG_OUTPUT_FOR_TEST="fullCheckBuildPkgForTest=true" \
    INPUTIA_FULL_CHECK_VERIFY_PKG_OUTPUT_FOR_TEST="fullCheckVerifyPkgForTest=true" \
    INPUTIA_FULL_CHECK_INSTALL_CHECK_OUTPUT_FOR_TEST=$'installCheckPassed=false\n' \
    "$BASH_SOURCE" "$APP"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^releaseFullCheckPassed=false reason=install-check-not-ready$' "install-gate-missing-failure-marker"
  require_self_check_no_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^== notarization readiness ==$' "install-gate-reached-notarization"
  require_self_check_no_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckHeavyOptInEnabled=true$' "install-gate-enabled-heavy-checks"
  require_self_check_no_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckMenuReadinessForTest=true$' "install-gate-ran-menu-readiness"

  run_self_check_case notary-gate 11 \
    /usr/bin/env TMPDIR="$temp_dir" \
    INPUTIA_FULL_CHECK_SELF_CHECK=0 \
    INPUTIA_FULL_CHECK_BUILD_PKG_OUTPUT_FOR_TEST="fullCheckBuildPkgForTest=true" \
    INPUTIA_FULL_CHECK_VERIFY_PKG_OUTPUT_FOR_TEST="fullCheckVerifyPkgForTest=true" \
    INPUTIA_FULL_CHECK_INSTALL_CHECK_OUTPUT_FOR_TEST=$'installCheckPassed=true\n' \
    INPUTIA_FULL_CHECK_NOTARIZATION_OUTPUT_FOR_TEST=$'inputiaGatekeeperReady=true\ninputiaNotarySubmissionReady=false\n' \
    "$BASH_SOURCE" "$APP"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^releaseFullCheckPassed=false reason=notary-submission-not-ready$' "notary-gate-missing-failure-marker"
  require_self_check_no_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckHeavyOptInEnabled=true$' "notary-gate-enabled-heavy-checks"
  require_self_check_no_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckMenuReadinessForTest=true$' "notary-gate-ran-menu-readiness"

  run_self_check_case success 0 \
    /usr/bin/env TMPDIR="$temp_dir" \
    INPUTIA_FULL_CHECK_SELF_CHECK=0 \
    INPUTIA_FULL_CHECK_BUILD_PKG_OUTPUT_FOR_TEST="fullCheckBuildPkgForTest=true" \
    INPUTIA_FULL_CHECK_VERIFY_PKG_OUTPUT_FOR_TEST="fullCheckVerifyPkgForTest=true" \
    INPUTIA_FULL_CHECK_INSTALL_CHECK_OUTPUT_FOR_TEST=$'installCheckPassed=true\n' \
    INPUTIA_FULL_CHECK_NOTARIZATION_OUTPUT_FOR_TEST=$'inputiaGatekeeperReady=true\ninputiaNotarySubmissionReady=true\n' \
    INPUTIA_FULL_CHECK_MENU_READINESS_FOR_TEST=1 \
    INPUTIA_FULL_CHECK_POST_INSTALL_FOR_TEST=1 \
    "$BASH_SOURCE" "$APP"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckHeavyOptInEnabled=true$' "success-missing-heavy-opt-in"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckMenuReadinessForTest=true$' "success-missing-menu-readiness"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckMenuReadinessOptIn=1$' "success-menu-opt-in-not-set"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckPostInstallForTest=true$' "success-missing-post-install"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckPostInstallGuiOptIn=1$' "success-gui-opt-in-not-set"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^fullCheckPostInstallUiSmoke=1$' "success-ui-smoke-not-set"
  require_self_check_output "$FULL_CHECK_SELF_CHECK_OUTPUT" '^releaseFullCheckPassed=true$' "success-missing-pass-marker"

  echo "fullCheckSelfCheck=true"
}

if [[ "${INPUTIA_FULL_CHECK_SELF_CHECK:-0}" == "1" ]]; then
  run_self_check
  exit 0
fi

trap cleanup EXIT

section "policy"
echo "validationTier=release/full-check"
echo "touchesMenuBar=true"
echo "opensGUI=true"
echo "changesSystemInputSource=false"
echo "checksNotarization=true"
echo "menuReadinessCacheFile=$MENU_CACHE"

section "pkg"
run_build_pkg
run_verify_pkg

section "install readiness"
install_output="$(run_install_check)"
printf '%s\n' "$install_output"
if ! /usr/bin/grep -q '^installCheckPassed=true$' <<<"$install_output"; then
  echo "releaseFullCheckPassed=false reason=install-check-not-ready"
  exit 12
fi

section "notarization readiness"
notarization_output="$(run_notarization_readiness)"
printf '%s\n' "$notarization_output"
if ! /usr/bin/grep -q '^inputiaGatekeeperReady=true$' <<<"$notarization_output"; then
  echo "releaseFullCheckPassed=false reason=gatekeeper-not-ready"
  exit 10
fi
if ! /usr/bin/grep -q '^inputiaNotarySubmissionReady=true$' <<<"$notarization_output"; then
  echo "releaseFullCheckPassed=false reason=notary-submission-not-ready"
  exit 11
fi

enable_heavy_checks

section "menu readiness"
run_menu_readiness

section "postinstall and GUI smoke"
run_post_install_regression

section "result"
echo "releaseFullCheckPassed=true"
