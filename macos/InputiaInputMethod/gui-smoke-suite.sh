#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
USER_DEFAULT_APP="$HOME/Library/Input Methods/InputiaInputMethod.app"
DEFAULT_APP="/Library/Input Methods/InputiaInputMethod.app"
if [[ -d "$USER_DEFAULT_APP" ]]; then
  DEFAULT_APP="$USER_DEFAULT_APP"
fi
APP="${1:-$DEFAULT_APP}"
READINESS_SCRIPT="$ROOT_DIR/gui-smoke-readiness.sh"
POST_INSTALL_REGRESSION="$ROOT_DIR/post-install-regression.sh"

readiness_output() {
  if (( ${+INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST} )); then
    printf '%s\n' "$INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST"
    return 0
  fi

  env INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=0 \
    INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=0 \
    "$READINESS_SCRIPT" "$APP"
}

readiness_line_value() {
  local line="$1"
  local key="$2"
  /usr/bin/awk -v key="$key" '
    {
      for (i = 1; i <= NF; i++) {
        split($i, part, "=")
        if (part[1] == key) {
          print part[2]
          exit
        }
      }
    }
  ' <<<"$line"
}

run_post_install_regression() {
  if (( ${+INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST} )); then
    echo "guiSmokeSuitePostInstallForTest=true rc=$INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST"
    return "$INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST"
  fi

  env INPUTIA_RUN_UI_SMOKE=1 \
    INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=0 \
    INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=0 \
    "$POST_INSTALL_REGRESSION" "$APP"
}

run_gui_smoke_suite() {
  local output status_line block_reasons_line ready reason block_reasons post_install_rc
  output="$(readiness_output 2>&1)"
  printf '%s\n' "$output" | /usr/bin/sed 's/^/guiSmokeSuiteReadiness: /'

  status_line="$(/usr/bin/grep '^guiSmokeReadinessReady=' <<<"$output" | /usr/bin/tail -n 1 || true)"
  if [[ -z "$status_line" ]]; then
    echo "guiSmokeSuiteReady=false reason=readiness-output-missing"
    echo "guiSmokeSuiteBlockReasons=readiness-output-missing"
    echo "guiSmokeSuiteWouldRun=false"
    return 12
  fi

  ready="$(readiness_line_value "$status_line" "guiSmokeReadinessReady")"
  reason="$(readiness_line_value "$status_line" "reason")"
  reason="${reason:-unknown}"
  block_reasons_line="$(/usr/bin/grep '^guiSmokeReadinessBlockReasons=' <<<"$output" | /usr/bin/tail -n 1 || true)"
  block_reasons="$(readiness_line_value "$block_reasons_line" "guiSmokeReadinessBlockReasons")"
  block_reasons="${block_reasons:-$reason}"

  if [[ "$ready" != "true" ]]; then
    echo "guiSmokeSuiteReady=false reason=$reason"
    echo "guiSmokeSuiteBlockReasons=$block_reasons"
    echo "guiSmokeSuiteWouldRun=false"
    return 12
  fi

  if [[ "$reason" != "none" || "$block_reasons" != "none" ]]; then
    echo "guiSmokeSuiteReady=false reason=readiness-inconsistent"
    echo "guiSmokeSuiteBlockReasons=$block_reasons"
    echo "guiSmokeSuiteWouldRun=false"
    return 12
  fi

  echo "guiSmokeSuiteReady=true reason=$reason"
  echo "guiSmokeSuiteBlockReasons=$block_reasons"
  echo "guiSmokeSuiteWouldRun=true"
  if [[ "${INPUTIA_GUI_SMOKE_SUITE_SKIP_RUN_FOR_TEST:-0}" == "1" ]]; then
    echo "guiSmokeSuiteRunSkipped=true reason=self-check"
    echo "guiSmokeSuitePassed=true"
    return 0
  fi

  set +e
  run_post_install_regression
  post_install_rc=$?
  set -e
  if [[ "$post_install_rc" != "0" ]]; then
    echo "guiSmokeSuitePassed=false reason=post-install-regression-failed rc=$post_install_rc"
    return "$post_install_rc"
  fi
  echo "guiSmokeSuitePassed=true"
}

run_gui_smoke_suite_self_check_case() {
  local label="$1"
  local readiness="$2"
  local expected_rc="$3"
  local expected_marker="$4"
  local output rc

  set +e
  output="$(
    INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST="$readiness" \
      INPUTIA_GUI_SMOKE_SUITE_SKIP_RUN_FOR_TEST=1 \
      run_gui_smoke_suite 2>&1
  )"
  rc=$?
  set -e

  printf '%s\n' "$output" | /usr/bin/sed "s/^/guiSmokeSuiteSelfCheck case=$label /"
  echo "guiSmokeSuiteSelfCheck case=$label rc=$rc"
  if [[ "$rc" != "$expected_rc" ]]; then
    echo "guiSmokeSuiteSelfCheck=false case=$label reason=unexpected-rc expected=$expected_rc actual=$rc"
    exit 1
  fi
  if ! /usr/bin/grep -q "$expected_marker" <<<"$output"; then
    echo "guiSmokeSuiteSelfCheck=false case=$label reason=missing-marker marker=$expected_marker"
    exit 1
  fi
}

if [[ "${INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK:-0}" == "1" ]]; then
  run_gui_smoke_suite_self_check_case \
    missing \
    "" \
    12 \
    "guiSmokeSuiteReady=false reason=readiness-output-missing"
  run_gui_smoke_suite_self_check_case \
    blocked \
    $'guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready\nguiSmokeReadinessReady=false reason=admin-required' \
    12 \
    "guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready"
  run_gui_smoke_suite_self_check_case \
    ready \
    $'guiSmokeReadinessBlockReasons=none\nguiSmokeReadinessReady=true reason=none' \
    0 \
    "guiSmokeSuiteRunSkipped=true reason=self-check"
  run_gui_smoke_suite_self_check_case \
    inconsistent \
    $'guiSmokeReadinessBlockReasons=tis-not-ready\nguiSmokeReadinessReady=true reason=none' \
    12 \
    "guiSmokeSuiteReady=false reason=readiness-inconsistent"
  set +e
  failure_output="$(
    INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST=$'guiSmokeReadinessBlockReasons=none\nguiSmokeReadinessReady=true reason=none' \
      INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23 \
      run_gui_smoke_suite 2>&1
  )"
  failure_rc=$?
  set -e
  printf '%s\n' "$failure_output" | /usr/bin/sed "s/^/guiSmokeSuiteSelfCheck case=post-install-failure /"
  echo "guiSmokeSuiteSelfCheck case=post-install-failure rc=$failure_rc"
  if [[ "$failure_rc" != "23" ]]; then
    echo "guiSmokeSuiteSelfCheck=false case=post-install-failure reason=unexpected-rc expected=23 actual=$failure_rc"
    exit 1
  fi
  if ! /usr/bin/grep -q "guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23" <<<"$failure_output"; then
    echo "guiSmokeSuiteSelfCheck=false case=post-install-failure reason=missing-failure-marker"
    exit 1
  fi
  echo "guiSmokeSuiteSelfCheck=true"
  exit 0
fi

run_gui_smoke_suite
