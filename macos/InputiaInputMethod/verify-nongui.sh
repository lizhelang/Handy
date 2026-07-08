#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
SYSTEM_APP="/Library/Input Methods/InputiaInputMethod.app"
source "$ROOT_DIR/smoke-common.sh"

section() {
  printf '\n== %s ==\n' "$1"
}

run_and_prefix() {
  local prefix="$1"
  shift
  "$@" 2>&1 | /usr/bin/sed "s/^/$prefix/"
}

run_expect_rc() {
  local expected_rc="$1"
  local label="$2"
  shift 2

  local output rc
  set +e
  output="$("$@" 2>&1)"
  rc=$?
  set -e
  RUN_EXPECT_RC_OUTPUT="$output"
  RUN_EXPECT_RC_ACTUAL="$rc"
  printf '%s\n' "$output" | /usr/bin/sed "s/^/$label: /"
  echo "$label.rc=$rc"
  if [[ "$rc" != "$expected_rc" ]]; then
    echo "nonGuiVerificationPassed=false reason=${label}-unexpected-rc expected=$expected_rc actual=$rc"
    exit 1
  fi
}

run_allow_rc() {
  local allowed_rcs="$1"
  local label="$2"
  shift 2

  local output rc
  set +e
  output="$("$@" 2>&1)"
  rc=$?
  set -e
  RUN_EXPECT_RC_OUTPUT="$output"
  RUN_EXPECT_RC_ACTUAL="$rc"
  printf '%s\n' "$output" | /usr/bin/sed "s/^/$label: /"
  echo "$label.rc=$rc"
  if [[ ",$allowed_rcs," != *",$rc,"* ]]; then
    echo "nonGuiVerificationPassed=false reason=${label}-unexpected-rc expected-one-of=$allowed_rcs actual=$rc"
    exit 1
  fi
}

run_expect_rc_or_gui_block() {
  local expected_rc="$1"
  local gui_block_rcs="$2"
  local label="$3"
  shift 3

  run_allow_rc "$expected_rc,$gui_block_rcs" "$label" "$@"
  if [[ "$RUN_EXPECT_RC_ACTUAL" == "$expected_rc" ]]; then
    return 0
  fi
  require_output_regex \
    "$RUN_EXPECT_RC_OUTPUT" \
    'guiSmokeReady=false reason=(no-console-user|gui-bootstrap-unavailable|login-not-complete|screen-locked|frontmost-unavailable|loginwindow-frontmost|process-list-unavailable)' \
    "${label}-missing-gui-session-blocker"
  echo "${label}AcceptedBlockReason=gui-session-or-process-list"
}

run_allow_launchctl_env_probe() {
  local label="$1"
  shift

  local output rc
  set +e
  output="$("$@" 2>&1)"
  rc=$?
  set -e
  RUN_EXPECT_RC_OUTPUT="$output"
  RUN_EXPECT_RC_ACTUAL="$rc"
  printf '%s\n' "$output" | /usr/bin/sed "s/^/$label: /"
  echo "$label.rc=$rc"
  if [[ "$rc" == "0" ]]; then
    echo "${label}Ready=true"
    return 0
  fi
  require_output_regex \
    "$output" \
    'Reentrancy avoided|bootstrap|not found|Operation not permitted|Could not set environment' \
    "${label}-unexpected-launchctl-failure"
  echo "${label}Ready=false"
  return 1
}

require_output() {
  local output="$1"
  local pattern="$2"
  local reason="$3"
  if ! /usr/bin/grep -q "$pattern" <<<"$output"; then
    echo "nonGuiVerificationPassed=false reason=$reason"
    exit 1
  fi
}

require_output_regex() {
  local output="$1"
  local pattern="$2"
  local reason="$3"
  if ! /usr/bin/grep -Eq "$pattern" <<<"$output"; then
    echo "nonGuiVerificationPassed=false reason=$reason"
    exit 1
  fi
}

require_status_block_reason() {
  local output="$1"
  local block_reason="$2"
  local reason="$3"
  local block_line
  block_line="$(/usr/bin/grep '^statusGuiSmokeBlockReasons=' <<<"$output" | /usr/bin/tail -1 || true)"
  if [[ ",${block_line#statusGuiSmokeBlockReasons=}," != *",$block_reason,"* ]]; then
    echo "nonGuiVerificationPassed=false reason=$reason"
    printf '%s\n' "$block_line"
    exit 1
  fi
}

require_signature_block_if_rejected() {
  local output="$1"
  local reason="$2"
  local target_exists
  local signature_accepted
  target_exists="$(/usr/bin/awk -F= '$1 == "statusTargetExists" { print $2; found = 1; exit } END { if (!found) print "true" }' <<<"$output")"
  signature_accepted="$(/usr/bin/awk -F= '$1 == "statusSignatureAccepted" { print $2; found = 1; exit } END { if (!found) print "unknown" }' <<<"$output")"
  if [[ "$target_exists" == "true" && "$signature_accepted" != "true" ]]; then
    require_status_block_reason "$output" "signature-rejected" "$reason"
  fi
}

require_executable() {
  local path="$1"
  local reason="$2"
  if [[ ! -x "$path" ]]; then
    echo "nonGuiVerificationPassed=false reason=$reason path=$path"
    exit 1
  fi
}

process_match_by_ps() {
  local process_name="$1"
  /bin/ps -axo pid=,comm=,command= 2>/dev/null |
    /usr/bin/awk -v process_name="$process_name" '
      {
        command = (NF >= 3) ? substr($0, index($0, $3)) : ""
        launcher = (command ~ "^/bin/(zsh|bash|sh)( |$)" || command ~ "^/usr/bin/(sudo|awk|grep|sed)( |$)")
      }
      !launcher {
        if ($2 == process_name ||
          $3 == process_name ||
          $3 ~ ("/" process_name "$") ||
          command ~ ("^" process_name "([ ]|$)") ||
          command ~ ("/" process_name "([ ]|$)") ||
          command ~ (process_name "\\.app/Contents/MacOS/" process_name "([ ]|$)")) {
          found = 1
        }
      }
      END { exit found ? 0 : 1 }
    '
}

process_running() {
  local process_name="$1"
  local process_check_output process_check_rc
  set +e
  process_check_output="$(/usr/bin/pgrep -x "$process_name" 2>&1 >/dev/null)"
  process_check_rc=$?
  set -e
  if [[ "$process_check_rc" -eq 0 ]]; then
    return 0
  fi
  if process_match_by_ps "$process_name"; then
    return 0
  fi
  if /bin/ps -axo pid=,comm=,command= >/dev/null 2>&1; then
    return 1
  fi
  if [[ -n "$process_check_output" ]]; then
    return 2
  fi
  return 1
}

process_details() {
  local process_name="$1"
  /bin/ps -axo pid=,ppid=,etime=,comm=,command= |
    /usr/bin/awk -v process_name="$process_name" '
      $4 ~ ("(^|/)" process_name "$") { print }
    '
}

fake_process_visible_by_pid() {
  local process_name="$1"
  local fake_pid="$2"
  /bin/ps -p "$fake_pid" -o comm=,command= 2>/dev/null |
    /usr/bin/awk -v process_name="$process_name" '
      {
        command = (NF >= 2) ? substr($0, index($0, $2)) : ""
        launcher = (command ~ "^/bin/(zsh|bash|sh)( |$)" || command ~ "^/usr/bin/(sudo|awk|grep|sed)( |$)")
      }
      !launcher {
        if ($1 == process_name ||
          $2 == process_name ||
          $2 ~ ("/" process_name "$") ||
          command ~ ("^" process_name "([ ]|$)") ||
          command ~ ("/" process_name "([ ]|$)") ||
          command ~ (process_name "\\.app/Contents/MacOS/" process_name "([ ]|$)")) {
          found = 1
        }
      }
      END { exit found ? 0 : 1 }
    '
}

assert_process_not_running() {
  local process_name="$1"
  local reason="$2"
  local max_wait="${INPUTIA_PROCESS_WAIT_TICKS:-100}"
  local waited=0
  local quiet_ticks="${INPUTIA_PROCESS_QUIET_TICKS:-3}"
  local quiet=0
  while ((waited < max_wait)); do
    if ! process_running "$process_name"; then
      quiet=$((quiet + 1))
      if ((quiet >= quiet_ticks)); then
        return 0
      fi
    else
      quiet=0
    fi
    /bin/sleep 0.1
    waited=$((waited + 1))
  done
  if process_running "$process_name"; then
    echo "nonGuiVerificationPassed=false reason=$reason process=$process_name waitedTicks=$waited"
    process_details "$process_name"
    exit 1
  fi
}

current_input_source_id() {
  local executable="$BUILD_APP/Contents/MacOS/InputiaInputMethod"
  local output=""
  if [[ -x "$ROOT_DIR/build/inputia-tis-tool" ]]; then
    output="$("$ROOT_DIR/build/inputia-tis-tool" --dump-current-input-source 2>/dev/null || true)"
  elif [[ -x "$executable" ]]; then
    output="$("$executable" --dump-current-input-source 2>/dev/null || true)"
  else
    echo "unknown"
    return 0
  fi
  /usr/bin/awk -F= '$1 == "id" { print $2; found = 1; exit } END { if (!found) print "unknown" }' <<<"$output"
}

debug_events_env() {
  /bin/launchctl getenv INPUTIA_DEBUG_EVENTS 2>/dev/null || true
}

assert_current_source_unchanged() {
  local label="$1"
  local before="$2"
  local after="$3"
  echo "${label}.currentSourceBefore=${before:-unknown}"
  echo "${label}.currentSourceAfter=${after:-unknown}"
  if [[ -z "$before" || -z "$after" || "$before" == "unknown" || "$after" == "unknown" ]]; then
    echo "nonGuiVerificationPassed=false reason=${label}-current-input-source-unavailable"
    exit 1
  fi
  if [[ "$before" != "$after" ]]; then
    echo "nonGuiVerificationPassed=false reason=${label}-mutated-current-input-source"
    exit 1
  fi
}

assert_debug_env_unchanged() {
  local label="$1"
  local before="$2"
  local after="$3"
  echo "${label}.debugEnvBefore=${before:-unset}"
  echo "${label}.debugEnvAfter=${after:-unset}"
  if [[ "$before" != "$after" ]]; then
    echo "nonGuiVerificationPassed=false reason=${label}-mutated-debug-env"
    exit 1
  fi
}

bundle_state() {
  local path="$1"
  local version="missing"
  local cdhash="missing"
  if [[ -d "$path" ]]; then
    version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$path/Contents/Info.plist" 2>/dev/null || echo unknown)"
    cdhash="$(/usr/bin/codesign -dv --verbose=4 "$path" 2>&1 | /usr/bin/awk -F= '/^CDHash=/{print $2; found = 1; exit} END { if (!found) print "unknown" }')"
  elif [[ -e "$path" ]]; then
    version="non-bundle"
    cdhash="non-bundle"
  fi
  printf '%s|%s|%s\n' "$path" "$version" "$cdhash"
}

user_host_state() {
  bundle_state "$HOME/Library/Input Methods/InputiaInputMethod.app"
  bundle_state "$HOME/Library/Input Methods/IputiaInputMethod.app"
  bundle_state "$HOME/Applications/Inputia 设置.app"
}

assert_user_host_baseline_absent() {
  local current_state
  current_state="$(user_host_state)"
  if [[ "${INPUTIA_VERIFY_ALLOW_USER_HOST_BASELINE:-0}" == "1" ]]; then
    echo "verifyUserHostBaselineAllowed=true"
    printf '%s\n' "$current_state" | /usr/bin/sed 's/^/verifyUserHostBaseline: /'
    return
  fi
  if /usr/bin/awk -F'|' '$2 != "missing" || $3 != "missing" { found = 1 } END { exit found ? 0 : 1 }' <<<"$current_state"; then
    echo "nonGuiVerificationPassed=false reason=user-host-baseline-present"
    printf '%s\n' "$current_state" | /usr/bin/sed 's/^/verifyUserHostBaseline: /'
    exit 21
  fi
  echo "verifyUserHostBaselineAbsent=true"
}

assert_no_user_host() {
  local label="$1"
  local current_state
  current_state="$(user_host_state)"
  if [[ "$current_state" != "$VERIFY_USER_HOST_BASELINE" ]]; then
    echo "nonGuiVerificationPassed=false reason=${label}-mutated-user-host-state"
    printf '%s\n' "$VERIFY_USER_HOST_BASELINE" | /usr/bin/sed "s/^/${label}.userHostBaseline: /"
    printf '%s\n' "$current_state" | /usr/bin/sed "s/^/${label}.userHostCurrent: /"
    exit 1
  fi
  echo "${label}.userHostUnchanged=true"
}

VERIFY_ORIGINAL_DEBUG_EVENTS="$(debug_events_env)"
assert_user_host_baseline_absent
VERIFY_USER_HOST_BASELINE="$(user_host_state)"
VERIFY_LOCK_DIR="/tmp/inputia-verify-nongui.lock"
VERIFY_LOCK_REAL_DIR="/private/tmp/inputia-verify-nongui.lock"
TMP_RESIDUE_ROOT="/private/tmp"
VERIFY_LOCK_ACQUIRED=0
VERIFY_TEMP_FILES=()
VERIFY_TEMP_DIRS=()
VERIFY_FAKE_PROCESS_PIDS=()
VERIFY_POST_INSTALL_USER_CONFLICT_ROOT="$(/usr/bin/mktemp -d "/tmp/inputia-post-install-user-conflict.XXXXXX")"
VERIFY_TEMP_DIRS+=("$VERIFY_POST_INSTALL_USER_CONFLICT_ROOT")
VERIFY_POST_INSTALL_USER_APP="$VERIFY_POST_INSTALL_USER_CONFLICT_ROOT/InputiaInputMethod.app"
VERIFY_POST_INSTALL_USER_LEGACY_APP="$VERIFY_POST_INSTALL_USER_CONFLICT_ROOT/IputiaInputMethod.app"
VERIFY_POST_INSTALL_USER_SETTINGS_APP="$VERIFY_POST_INSTALL_USER_CONFLICT_ROOT/Inputia 设置.app"

restore_verify_debug_env() {
  if [[ -n "$VERIFY_ORIGINAL_DEBUG_EVENTS" ]]; then
    /bin/launchctl setenv INPUTIA_DEBUG_EVENTS "$VERIFY_ORIGINAL_DEBUG_EVENTS" >/dev/null 2>&1 || true
  else
    /bin/launchctl unsetenv INPUTIA_DEBUG_EVENTS >/dev/null 2>&1 || true
  fi
}

release_verify_lock() {
  if [[ "$VERIFY_LOCK_ACQUIRED" == "1" ]]; then
    /bin/rm -rf "$VERIFY_LOCK_DIR" >/dev/null 2>&1 || true
    VERIFY_LOCK_ACQUIRED=0
  fi
}

cleanup_verify_temp_files() {
  local temp_file
  for temp_file in ${VERIFY_TEMP_FILES[@]+"${VERIFY_TEMP_FILES[@]}"}; do
    [[ -n "$temp_file" ]] && /bin/rm -f "$temp_file" >/dev/null 2>&1 || true
  done
}

cleanup_verify_temp_dirs() {
  local temp_dir
  for temp_dir in ${VERIFY_TEMP_DIRS[@]+"${VERIFY_TEMP_DIRS[@]}"}; do
    case "$temp_dir" in
      /tmp/inputia-*|/private/tmp/inputia-*)
        /bin/rm -rf "$temp_dir" >/dev/null 2>&1 || true
        ;;
    esac
  done
}

cleanup_verify_fake_processes() {
  local fake_pid
  for fake_pid in ${VERIFY_FAKE_PROCESS_PIDS[@]+"${VERIFY_FAKE_PROCESS_PIDS[@]}"}; do
    if [[ -n "$fake_pid" ]] && /bin/kill -0 "$fake_pid" >/dev/null 2>&1; then
      /bin/kill "$fake_pid" >/dev/null 2>&1 || true
      /bin/sleep 0.1
      /bin/kill -9 "$fake_pid" >/dev/null 2>&1 || true
    fi
    [[ -n "$fake_pid" ]] && wait "$fake_pid" >/dev/null 2>&1 || true
  done
}

cleanup_verify() {
  restore_verify_debug_env
  cleanup_verify_fake_processes
  cleanup_verify_temp_files
  cleanup_verify_temp_dirs
  release_verify_lock
}
trap cleanup_verify EXIT

cleanup_stale_verify_residue() {
  if [[ "${INPUTIA_VERIFY_SKIP_STALE_RESIDUE_CLEANUP:-0}" == "1" ]]; then
    echo "staleVerifyResidueCleanup=skipped"
    return 0
  fi

  local stale_path stale_count=0
  while IFS= read -r stale_path; do
    case "$stale_path" in
      /private/tmp/inputia-*|/tmp/inputia-*)
        /bin/rm -rf "$stale_path" >/dev/null 2>&1 || true
        stale_count=$((stale_count + 1))
        ;;
    esac
  done < <(
    /usr/bin/find "$TMP_RESIDUE_ROOT" -maxdepth 1 \
      ! -path "$VERIFY_LOCK_DIR" \
      ! -path "$VERIFY_LOCK_REAL_DIR" \
      \( -name 'inputia-textedit-*' \
        -o -name 'inputia-safari-typing-*' \
        -o -name 'inputia-safari-enter*' \
        -o -name 'inputia-textedit-command-*' \
        -o -name 'inputia-clipboard-recall-*' \
        -o -name 'inputia-safari-command-*' \
        -o -name 'inputia-safari-input-source-test.*' \
        -o -name 'inputia-hitoolbox-preference.*' \
        -o -name 'inputia-safari-source-select.*' \
        -o -name 'inputia-safari-focused-select.*' \
        -o -name 'inputia-safari-diagnose-*' \
        -o -name 'inputia-debug-event-*' \
        -o -name 'inputia-pkg-verify.*' \
        -o -name 'inputia-launchservices-*.log' \
        -o -name 'inputia-install-user.*' \) \
      -print 2>/dev/null
  )
  echo "staleVerifyResidueCleaned=$stale_count"
}

start_fake_existing_process() {
  local process_name="$1"
  local fake_pid
  /bin/zsh -c "exec -a '$process_name' /bin/sleep 60" >/dev/null 2>&1 &
  fake_pid=$!
  INPUTIA_FAKE_EXISTING_PID="$fake_pid"
  VERIFY_FAKE_PROCESS_PIDS+=("$fake_pid")
  local waited=0
  while ! fake_process_visible_by_pid "$process_name" "$fake_pid" && ((waited < 40)); do
    /bin/sleep 0.05
    waited=$((waited + 1))
  done
  if ! fake_process_visible_by_pid "$process_name" "$fake_pid"; then
    echo "nonGuiVerificationPassed=false reason=fake-existing-process-not-visible process=$process_name pid=$fake_pid"
    /bin/ps -p "$fake_pid" -o pid=,comm=,command= 2>/dev/null || true
    exit 1
  fi
  echo "fakeExistingProcessStarted=true process=$process_name pid=$fake_pid waitedTicks=$waited"
}

stop_fake_existing_process() {
  local process_name="$1"
  local fake_pid="$2"
  if /bin/kill -0 "$fake_pid" >/dev/null 2>&1; then
    /bin/kill "$fake_pid" >/dev/null 2>&1 || true
  fi
  wait "$fake_pid" >/dev/null 2>&1 || true
  local remaining_pids=()
  local candidate_pid
  for candidate_pid in ${VERIFY_FAKE_PROCESS_PIDS[@]+"${VERIFY_FAKE_PROCESS_PIDS[@]}"}; do
    [[ "$candidate_pid" != "$fake_pid" ]] && remaining_pids+=("$candidate_pid")
  done
  if ((${#remaining_pids[@]} == 0)); then
    VERIFY_FAKE_PROCESS_PIDS=()
  else
    VERIFY_FAKE_PROCESS_PIDS=("${remaining_pids[@]}")
  fi
  assert_process_not_running "$process_name" "fake-existing-process-left-running"
  echo "fakeExistingProcessStopped=true process=$process_name pid=$fake_pid"
}

acquire_verify_lock() {
  if /bin/mkdir "$VERIFY_LOCK_DIR" >/dev/null 2>&1; then
    VERIFY_LOCK_ACQUIRED=1
    echo "$$" >"$VERIFY_LOCK_DIR/pid"
    echo "verifyLockAcquired=true"
    return 0
  fi

  local existing_pid=""
  if [[ -f "$VERIFY_LOCK_DIR/pid" ]]; then
    existing_pid="$(/bin/cat "$VERIFY_LOCK_DIR/pid" 2>/dev/null || true)"
  fi
  if [[ -n "$existing_pid" ]] && /bin/kill -0 "$existing_pid" >/dev/null 2>&1; then
    echo "nonGuiVerificationPassed=false reason=verify-already-running lock=$VERIFY_LOCK_DIR"
    echo "verifyLockOwnerPid=$existing_pid"
    exit 20
  fi

  echo "verifyLockStale=true path=$VERIFY_LOCK_DIR pid=${existing_pid:-unknown}"
  /bin/rm -rf "$VERIFY_LOCK_DIR" >/dev/null 2>&1 || true
  if /bin/mkdir "$VERIFY_LOCK_DIR" >/dev/null 2>&1; then
    VERIFY_LOCK_ACQUIRED=1
    echo "$$" >"$VERIFY_LOCK_DIR/pid"
    echo "verifyLockAcquired=true"
    return 0
  fi

  echo "nonGuiVerificationPassed=false reason=verify-lock-acquire-failed lock=$VERIFY_LOCK_DIR"
  exit 20
}

acquire_verify_lock

cleanup_stale_verify_residue

TEXTEDIT_PREEXISTING=false
SAFARI_PREEXISTING=false
if process_running TextEdit; then
  TEXTEDIT_PREEXISTING=true
fi
if process_running Safari; then
  SAFARI_PREEXISTING=true
fi

compile_quoted_applescript_blocks() {
  local label="$1"
  local source_file="$2"
  local script_file compiled_file
  script_file="$(/usr/bin/mktemp "/tmp/inputia-${label}-applescript.XXXXXX")"
  compiled_file="$(/usr/bin/mktemp "/tmp/inputia-${label}-applescript.XXXXXX.scpt")"
  VERIFY_TEMP_FILES+=("$script_file" "$compiled_file")
  /usr/bin/awk '
    /<<'\''APPLESCRIPT'\''/ { active = 1; found = 1; next }
    /^APPLESCRIPT$/ { active = 0; print ""; next }
    active { print }
    END { if (!found) exit 2 }
  ' "$source_file" >"$script_file"
  if /usr/bin/osacompile -o "$compiled_file" "$script_file" >/dev/null 2>&1; then
    echo "appleScriptCompileOK=true file=$label"
  else
    echo "nonGuiVerificationPassed=false reason=applescript-compile file=$label"
    /usr/bin/osacompile -o "$compiled_file" "$script_file"
    /bin/rm -f "$script_file" "$compiled_file"
    exit 1
  fi
  /bin/rm -f "$script_file" "$compiled_file"
}

compile_safari_applescript_block() {
  local label="$1"
  local source_file="$2"
  local script_file compiled_file
  script_file="$(/usr/bin/mktemp "/tmp/inputia-${label}-applescript.XXXXXX")"
  compiled_file="$(/usr/bin/mktemp "/tmp/inputia-${label}-applescript.XXXXXX.scpt")"
  VERIFY_TEMP_FILES+=("$script_file" "$compiled_file")
  /usr/bin/awk '
    /<<APPLESCRIPT/ { active = 1; found = 1; next }
    /^APPLESCRIPT$/ { active = 0; print ""; next }
    active {
      if ($0 == "set testURL to \"$url\"") {
        print "set testURL to \"data:text/html;charset=utf-8,%3Cinput%3E\""
        next
      }
      if ($0 == "$keys") {
        print "    key code 45"
        print "    delay 0.06"
        print "    key code 34"
        print "    delay 0.06"
        print "    key code 49"
        next
      }
      print
    }
    END { if (!found) exit 2 }
  ' "$source_file" >"$script_file"
  if /usr/bin/osacompile -o "$compiled_file" "$script_file" >/dev/null 2>&1; then
    echo "appleScriptCompileOK=true file=$label"
  else
    echo "nonGuiVerificationPassed=false reason=applescript-compile file=$label"
    /usr/bin/osacompile -o "$compiled_file" "$script_file"
    /bin/rm -f "$script_file" "$compiled_file"
    exit 1
  fi
  /bin/rm -f "$script_file" "$compiled_file"
}

verify_cleanup_permission_contract() {
  /usr/bin/python3 - "$ROOT_DIR" <<'PY'
import pathlib
import plistlib
import sys

root = pathlib.Path(sys.argv[1])
verify_nongui_text = (root / "verify-nongui.sh").read_text()

def require(condition, reason):
    if not condition:
        print(f"cleanupPermissionContract=false reason={reason}")
        raise SystemExit(1)

common = (root / "smoke-common.sh").read_text()
require("INPUTIA_TEXTEDIT_CLEANUP_ALLOWED" in common, "common-missing-textedit-cleanup-gate")
require("INPUTIA_SAFARI_CLEANUP_ALLOWED" in common, "common-missing-safari-cleanup-gate")
require("inputia_prepare_debug_event_log()" in common, "common-missing-debug-event-log-prepare")
require("debugEventLogPrepare=false" in common, "common-missing-debug-event-log-prepare-failure-marker")
require("reason=not-regular-file" in common, "common-missing-debug-event-log-prepare-not-regular-reason")
require("debugEventLogPrepare=true" in common, "common-missing-debug-event-log-prepare-success-marker")
require("inputia_assert_debug_event_log_clean()" in common, "common-missing-debug-event-log-clean-assert")
require("debugEventLogClean=false" in common, "common-missing-debug-event-log-dirty-marker")
require("debugEventLogClean=true" in common, "common-missing-debug-event-log-clean-marker")
require("debug-event-log-not-clean" in common, "common-missing-debug-event-log-dirty-reason")
require("INPUTIA_PROCESS_RUNNING_FOR_TEST" in common, "common-missing-process-running-test-override")
require("INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST" in common, "common-missing-process-ignore-real-test-override")
require("inputia_require_process_not_running()" in common, "common-missing-process-preflight-helper")
require('allow_var" != "-"' in common, "common-missing-process-preflight-no-allow-sentinel")
require("process-list-unavailable" in common, "common-missing-process-list-unavailable-marker")
require("processListAvailable=false" in common, "common-missing-process-list-availability-marker")
require("inputia_cleanup_smoke_files()" in common, "common-missing-smoke-file-cleanup-helper")
require("INPUTIA_KEEP_SMOKE_LOGS" in common, "common-missing-keep-smoke-logs-gate")
require("smokeTempCleanup=skipped" in common, "common-missing-smoke-file-cleanup-skip-marker")
require("inputia_wait_process_exit()" in common, "common-missing-process-exit-wait")
require("inputia_wait_process_exit TextEdit" in common, "common-missing-textedit-exit-wait")
require("inputia_wait_process_exit Safari" in common, "common-missing-safari-exit-wait")
require("textEditCleanupFailed=process-still-running" in common, "common-missing-textedit-cleanup-failure")
require("safariCleanupFailed=process-still-running" in common, "common-missing-safari-cleanup-failure")
require("inputia_clipboard_info_restorable_reason()" in common, "common-missing-clipboard-info-classifier")
require("inputia_current_clipboard_info()" in common, "common-missing-clipboard-info-provider")
require("inputia_try_write_clipboard_text()" in common, "common-missing-safe-clipboard-write-helper")
require("pasteboard-unavailable" in common, "common-missing-pasteboard-unavailable-marker")
require("inputia_try_set_debug_events_env()" in common, "common-missing-safe-debug-env-set-helper")
require("inputia_set_debug_events_env_or_exit()" in common, "common-missing-debug-env-required-helper")
require("launchctl-env-unavailable" in common, "common-missing-launchctl-env-unavailable-marker")
require("INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST" in common, "common-missing-clipboard-info-test-override-gate")
require("INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST" in common, "common-missing-clipboard-info-test-override")
require("inputia_require_text_clipboard_restorable()" in common, "common-missing-text-clipboard-restorable-helper")
require("INPUTIA_ALLOW_NON_TEXT_CLIPBOARD_SMOKE" in common, "common-missing-non-text-clipboard-override")
require("non-text-clipboard" in common, "common-missing-non-text-clipboard-rejection")
require("text-restorable" in common, "common-missing-text-restorable-classification")
require("missing-text-clipboard" in common, "common-missing-missing-text-clipboard-classification")
require("inputia_run_with_timeout()" in common, "common-missing-timeout-helper")
require("preexec_fn=os.setsid" in common, "common-timeout-helper-missing-process-group")
require("os.killpg(process.pid, signal.SIGTERM)" in common, "common-timeout-helper-missing-process-group-term")
require("os.killpg(process.pid, signal.SIGKILL)" in common, "common-timeout-helper-missing-process-group-kill")
require("inputia_restore_previous_input_source()" in common, "common-missing-input-source-restore-helper")
require("INPUTIA_PREVIOUS_INPUT_SOURCE_ID=" in common, "common-missing-previous-input-source-capture")
require("input-source-capture-failed" in common, "common-missing-input-source-capture-failure-gate")
require("selectCurrentMatchesTarget=true" in common, "common-missing-select-target-confirmation")
require("inputSourceRestore=true id=$previous_id" in common, "common-missing-input-source-restore-confirmation")
require("inputSourceRestore=false expected=$previous_id actual=${current_id:-unknown}" in common, "common-missing-input-source-restore-failure-marker")
require("return 1" in common[common.find("inputia_restore_previous_input_source()"):common.find("inputia_select_input_source_or_exit()")], "common-input-source-restore-failure-not-fatal")

previous_source_index = common.find("INPUTIA_PREVIOUS_INPUT_SOURCE_ID=")
previous_source_failure_index = common.find("input-source-capture-failed")
select_inputia_index = common.find("--select-inputia-source-id")
select_confirm_index = common.find("selectCurrentMatchesTarget=true")
restore_helper_index = common.find("inputia_restore_previous_input_source()")
restore_current_index = common.find('current_id="$(inputia_current_input_source_id "$executable" "$tis_tool")"', restore_helper_index)
restore_select_index = common.find("--select-source-id \"$previous_id\"", restore_helper_index)
restore_confirm_index = common.find("inputSourceRestore=true id=$previous_id", restore_helper_index)
require(previous_source_index >= 0 and select_inputia_index > previous_source_index, "common-previous-input-source-captured-after-select")
require(previous_source_failure_index > previous_source_index and previous_source_failure_index < select_inputia_index, "common-input-source-capture-failure-after-select")
require(select_confirm_index > select_inputia_index, "common-select-confirmation-before-select")
require(restore_helper_index >= 0, "common-missing-input-source-restore-helper-order")
require(restore_current_index > restore_helper_index, "common-restore-missing-current-source-check")
require(restore_select_index > restore_current_index, "common-restore-select-before-current-check")
require(restore_confirm_index > restore_select_index, "common-restore-confirm-before-select")

textedit_cleanup_index = common.find("inputia_cleanup_textedit_if_started()")
safari_cleanup_index = common.find("inputia_cleanup_safari_if_started()")
smoke_file_cleanup_index = common.find("inputia_cleanup_smoke_files()")
require(textedit_cleanup_index >= 0, "common-missing-textedit-cleanup-helper")
require(safari_cleanup_index >= 0, "common-missing-safari-cleanup-helper")
require(smoke_file_cleanup_index > textedit_cleanup_index, "common-missing-smoke-file-cleanup-after-app-cleanup")
textedit_cleanup_block = common[textedit_cleanup_index:safari_cleanup_index]
safari_cleanup_block = common[safari_cleanup_index:smoke_file_cleanup_index]
for marker in (
    'INPUTIA_TEXTEDIT_PREFLIGHT:-}" == "not-running"',
    'INPUTIA_TEXTEDIT_CLEANUP_ALLOWED:-0}" == "1"',
    'tell application "TextEdit" to quit saving no',
    'inputia_wait_process_exit TextEdit',
    'textEditCleanupFailed=process-still-running',
):
    require(marker in textedit_cleanup_block, f"common-textedit-cleanup-missing-{marker}")
for marker in (
    'INPUTIA_SAFARI_PREFLIGHT:-}" == "not-running"',
    'INPUTIA_SAFARI_CLEANUP_ALLOWED:-0}" == "1"',
    'tell application "Safari" to quit',
    'inputia_wait_process_exit Safari',
    'safariCleanupFailed=process-still-running',
):
    require(marker in safari_cleanup_block, f"common-safari-cleanup-missing-{marker}")

contracts = [
    ("smoke-textedit.sh", "INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1", "inputia_select_input_source_or_exit", "results=\"$(inputia_run_with_timeout"),
    ("smoke-textedit-command-shortcuts.sh", "INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1", "inputia_select_input_source_or_exit", "results=\"$(inputia_run_with_timeout"),
    ("smoke-clipboard-recall.sh", "INPUTIA_TEXTEDIT_CLEANUP_ALLOWED=1", "inputia_select_input_source_or_exit", "result=\"$(inputia_run_with_timeout"),
    ("smoke-safari-typing.sh", "INPUTIA_SAFARI_CLEANUP_ALLOWED=1", "inputia_select_input_source_or_exit", "results=\"$(inputia_run_with_timeout"),
    ("smoke-safari-command-shortcuts.sh", "INPUTIA_SAFARI_CLEANUP_ALLOWED=1", "inputia_select_input_source_or_exit", "results=\"$(inputia_run_with_timeout"),
    ("smoke-safari-enter.sh", "INPUTIA_SAFARI_CLEANUP_ALLOWED=1", "inputia_select_input_source_or_exit", "results=\"$(inputia_run_with_timeout"),
    ("diagnose-safari-input-source.sh", "INPUTIA_SAFARI_CLEANUP_ALLOWED=1", "inputia_select_input_source_or_exit", "SAFARI_DIAGNOSE_WINDOW_ID=\"$(inputia_run_with_timeout"),
]

for filename, marker, after, before in contracts:
    text = (root / filename).read_text()
    after_index = text.find(after)
    marker_index = text.find(marker)
    before_index = text.find(before)
    require(after_index >= 0, f"{filename}-missing-select-gate")
    require(marker_index >= 0, f"{filename}-missing-cleanup-marker")
    require(before_index >= 0, f"{filename}-missing-target-osascript")
    require(after_index < marker_index < before_index, f"{filename}-cleanup-marker-order")

cleanup_status_contracts = [
    ("smoke-textedit.sh", "cleanup_textedit_smoke()"),
    ("smoke-textedit-command-shortcuts.sh", "cleanup_textedit_command_smoke()"),
    ("smoke-clipboard-recall.sh", "cleanup_smoke()"),
    ("smoke-safari-typing.sh", "cleanup_smoke()"),
    ("smoke-safari-command-shortcuts.sh", "cleanup_smoke()"),
    ("smoke-safari-enter.sh", "cleanup_smoke()"),
    ("diagnose-safari-input-source.sh", "cleanup_diagnosis()"),
]

for filename, cleanup_name in cleanup_status_contracts:
    text = (root / filename).read_text()
    cleanup_index = text.find(cleanup_name)
    trap_index = text.find("trap ", cleanup_index)
    cleanup_block = text[cleanup_index:trap_index]
    require(cleanup_index >= 0, f"{filename}-missing-cleanup-function")
    require('local cleanup_status=0' in cleanup_block, f"{filename}-cleanup-missing-status-aggregate")
    require('|| cleanup_status=1' in cleanup_block, f"{filename}-cleanup-missing-failure-aggregation")
    require('return "$cleanup_status"' in cleanup_block, f"{filename}-cleanup-missing-aggregate-return")
    require(cleanup_block.count('|| cleanup_status=1') >= 2, f"{filename}-cleanup-does-not-aggregate-multiple-steps")

preflight_contracts = [
    ("smoke-textedit.sh", "inputia_require_textedit_idle", "inputia_select_input_source_or_exit"),
    ("smoke-textedit-command-shortcuts.sh", "inputia_require_textedit_idle", "inputia_select_input_source_or_exit"),
    ("smoke-clipboard-recall.sh", "inputia_require_textedit_idle", "inputia_select_input_source_or_exit"),
    ("smoke-safari-typing.sh", "inputia_require_safari_idle", "inputia_select_input_source_or_exit"),
    ("smoke-safari-command-shortcuts.sh", "inputia_require_safari_idle", "inputia_select_input_source_or_exit"),
    ("smoke-safari-enter.sh", "inputia_require_safari_idle", "inputia_select_input_source_or_exit"),
    ("diagnose-safari-input-source.sh", "inputia_require_safari_idle", "inputia_select_input_source_or_exit"),
]

for filename, preflight, select_call in preflight_contracts:
    text = (root / filename).read_text()
    preflight_index = text.find(preflight)
    select_index = text.find(select_call)
    require(preflight_index >= 0, f"{filename}-missing-existing-app-preflight")
    require(select_index > preflight_index, f"{filename}-existing-app-preflight-after-select")

unique_tmp_contracts = {
    "smoke-textedit.sh": (
        "/tmp/inputia-textedit-select.$$.log",
        "/tmp/inputia-textedit-restore.$$.log",
        "/tmp/inputia-textedit-osascript.$$.applescript",
    ),
    "smoke-textedit-command-shortcuts.sh": (
        "/tmp/inputia-textedit-command-select.$$.log",
        "/tmp/inputia-textedit-command-restore.$$.log",
        "/tmp/inputia-textedit-command-osascript.$$.applescript",
    ),
    "smoke-clipboard-recall.sh": (
        "/tmp/inputia-clipboard-recall-events.$$.log",
        "/tmp/inputia-clipboard-recall-select.$$.log",
        "/tmp/inputia-clipboard-recall-restore.$$.log",
        "/tmp/inputia-clipboard-recall-osascript.$$.applescript",
    ),
    "smoke-safari-typing.sh": (
        "/tmp/inputia-safari-typing-test.$$.url",
        "/tmp/inputia-safari-typing-select.$$.log",
        "/tmp/inputia-safari-typing-restore.$$.log",
        "/tmp/inputia-safari-typing-osascript.$$.applescript",
    ),
    "smoke-safari-command-shortcuts.sh": (
        "/tmp/inputia-safari-command-test.$$.url",
        "/tmp/inputia-safari-command-select.$$.log",
        "/tmp/inputia-safari-command-restore.$$.log",
        "/tmp/inputia-safari-command-osascript.$$.applescript",
    ),
    "smoke-safari-enter.sh": (
        "/tmp/inputia-safari-enter.$$.log",
        "/tmp/inputia-safari-enter-test.$$.url",
        "/tmp/inputia-safari-enter-select.$$.log",
        "/tmp/inputia-safari-enter-restore.$$.log",
        "/tmp/inputia-safari-enter-osascript.$$.applescript",
    ),
    "diagnose-safari-input-source.sh": (
        "/tmp/inputia-safari-input-source-test.$$.url",
        "/tmp/inputia-hitoolbox-preference.$$.txt",
        "/tmp/inputia-safari-source-select.$$.log",
        "/tmp/inputia-safari-focused-select.$$.log",
        "/tmp/inputia-safari-diagnose-restore.$$.log",
        "/tmp/inputia-safari-diagnose-osascript.$$.applescript",
    ),
}

for filename, required_paths in unique_tmp_contracts.items():
    text = (root / filename).read_text()
    for path in required_paths:
        require(path in text, f"{filename}-missing-unique-temp-{pathlib.Path(path).name}")

for filename in ("smoke-safari-typing.sh", "smoke-safari-command-shortcuts.sh", "smoke-safari-enter.sh"):
    text = (root / filename).read_text()
    require("on closeSmokeWindow(smokeWindowId)" in text, f"{filename}-missing-window-close-handler")
    require("set smokeWindowId to id of front window" in text, f"{filename}-missing-window-id-capture")
    require("close window id smokeWindowId" in text, f"{filename}-missing-window-id-close")
    require("OSASCRIPT_FILE=" in text, f"{filename}-missing-osascript-temp-file")
    require("inputia_run_with_timeout" in text, f"{filename}-missing-osascript-timeout")

diagnose_text = (root / "diagnose-safari-input-source.sh").read_text()
require("close_safari_diagnose_window" in diagnose_text, "diagnose-safari-missing-window-close-helper")
require("SAFARI_DIAGNOSE_WINDOW_ID" in diagnose_text, "diagnose-safari-missing-window-id-variable")
require("return id of front window" in diagnose_text, "diagnose-safari-missing-window-id-capture")
require("close window id smokeWindowId" in diagnose_text, "diagnose-safari-missing-window-id-close")
require("OSASCRIPT_FILE=" in diagnose_text, "diagnose-safari-missing-osascript-temp-file")
require("inputia_run_with_timeout safari-diagnose-osascript" in diagnose_text, "diagnose-safari-missing-osascript-timeout")
require("INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK" in diagnose_text, "diagnose-safari-missing-cleanup-self-check")
require("safariDiagnoseCleanupSelfCheck=true phase=after-temp-write" in diagnose_text, "diagnose-safari-missing-cleanup-self-check-marker")
require("INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK_RC" in diagnose_text, "diagnose-safari-missing-cleanup-self-check-rc-override")
diagnose_cleanup_self_check_index = diagnose_text.find("INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK")
diagnose_select_index = diagnose_text.find("inputia_select_input_source_or_exit")
diagnose_trap_index = diagnose_text.find("trap cleanup_diagnosis EXIT")
require(diagnose_trap_index < diagnose_cleanup_self_check_index < diagnose_select_index, "diagnose-safari-cleanup-self-check-order")

tis_readiness_text = (root / "tis-readiness.sh").read_text()
require("app_assessment()" in tis_readiness_text, "tis-readiness-missing-app-assessment-helper")
require("return 0" in tis_readiness_text[tis_readiness_text.find("cdhash()"):tis_readiness_text.find("app_assessment()")], "tis-readiness-cdhash-missing-safe-return")
require("appExists=" in tis_readiness_text, "tis-readiness-missing-app-exists-output")
require("tis.readinessBlockReason=app-missing" in tis_readiness_text, "tis-readiness-missing-app-missing-reason")
require("tis.requiredAction=install-inputia-app" in tis_readiness_text, "tis-readiness-missing-app-install-action")
require("tis.environmentRequiredAction=repair-current-user-directory-service" in tis_readiness_text, "tis-readiness-missing-environment-required-action")
require("appSignatureAccepted=" in tis_readiness_text, "tis-readiness-missing-signature-accepted-output")
require("tis.readinessBlockReason=signature-rejected" in tis_readiness_text, "tis-readiness-missing-signature-rejected-reason")
require("tis.requiredAction=sign-with-accepted-identity" in tis_readiness_text, "tis-readiness-missing-signature-required-action")
require("tis_value_for_icon()" in tis_readiness_text, "tis-readiness-missing-icon-filtered-value-helper")
require("tis_count_for_icon()" in tis_readiness_text, "tis-readiness-missing-icon-filtered-count-helper")
require("tis.targetEnabledMatches=" in tis_readiness_text, "tis-readiness-missing-target-enabled-count-output")
require("tis.targetInstalledMatches=" in tis_readiness_text, "tis-readiness-missing-target-installed-count-output")
require("target-source-not-installed" in tis_readiness_text, "tis-readiness-missing-target-source-not-installed-reason")
require("target_enabled_matches" in tis_readiness_text, "tis-readiness-does-not-use-target-enabled-matches")
require("tisToolFallback=installed-host" in tis_readiness_text, "tis-readiness-missing-installed-host-fallback")
require("missing-tis-tool-and-host" in tis_readiness_text, "tis-readiness-missing-tool-and-host-reason")

status_text = (root / "status.sh").read_text()
require("return 0" in status_text[status_text.find("plist_value()"):status_text.find("app_version()")], "status-plist-value-missing-safe-return")
require("return 0" in status_text[status_text.find("app_cdhash()"):status_text.find("app_assessment()")], "status-cdhash-missing-safe-return")
require("statusGuiSmokeReady=false" in status_text, "status-missing-gui-smoke-not-ready-output")

info_plist = plistlib.loads((root / "Info.plist").read_bytes())
require(info_plist.get("CFBundleDisplayName") == "Inputia", "info-plist-display-name-is-not-inputia")
require(info_plist.get("CFBundleName") == "Inputia", "info-plist-bundle-name-is-not-inputia")
mode_dict = (
    info_plist.get("ComponentInputModeDict", {})
    .get("tsInputModeListKey", {})
    .get("com.inputia.inputmethod.Inputia.Main", {})
)
require(mode_dict.get("TISInputSourceID") == "com.inputia.inputmethod.Inputia.Main", "info-plist-main-mode-id-mismatch")
for localization in ("en.lproj", "zh-Hans.lproj", "zh-Hant.lproj"):
    localized_info = (root / "Resources" / localization / "InfoPlist.strings").read_text()
    require('"CFBundleDisplayName" = "Inputia";' in localized_info, f"{localization}-display-name-not-inputia")
    require('"CFBundleName" = "Inputia";' in localized_info, f"{localization}-bundle-name-not-inputia")
    require('"com.inputia.inputmethod.Inputia" = "Inputia";' in localized_info, f"{localization}-parent-name-not-inputia")
    require('"com.inputia.inputmethod.Inputia.Main" = "Inputia";' in localized_info, f"{localization}-main-mode-name-not-inputia")
    for forbidden_name_suffix in ("Inputia 简体", "Inputia 繁体", "Inputia Simplified", "Inputia Traditional"):
        require(forbidden_name_suffix not in localized_info, f"{localization}-inputia-name-has-script-suffix")

settings_window_text = (root / "Sources" / "InputiaInputMethod" / "InputiaSettingsWindow.swift").read_text()
require('var chineseScript: String?' in settings_window_text, "settings-window-missing-chinese-script-field")
require('var scriptToggleShortcut: String?' in settings_window_text, "settings-window-missing-script-shortcut-field")
require('case chineseScript = "chinese_script"' in settings_window_text, "settings-window-missing-chinese-script-json-key")
require('case scriptToggleShortcut = "script_toggle_shortcut"' in settings_window_text, "settings-window-missing-script-shortcut-json-key")
require('chineseScript: "simplified"' in settings_window_text, "settings-window-missing-default-simplified-script")
require('scriptToggleShortcut: "control_shift_s"' in settings_window_text, "settings-window-missing-default-script-shortcut")
require('NSSegmentedControl(labels: ["简体", "繁体"]' in settings_window_text, "settings-window-missing-script-segment-control")
require('return labeledRow(title: "中文字形", control: chineseScriptSegment)' in settings_window_text, "settings-window-missing-script-row")
require('return labeledRow(title: "简繁切换", control: scriptShortcutPopup)' in settings_window_text, "settings-window-missing-script-shortcut-row")
require('next.chineseScript = chineseScriptSegment.selectedSegment == 1 ? "traditional" : "simplified"' in settings_window_text, "settings-window-does-not-save-script-choice")
require('next.scriptToggleShortcut = (scriptShortcutPopup.selectedItem?.representedObject as? String) ?? "control_shift_s"' in settings_window_text, "settings-window-does-not-save-script-shortcut")

import_signing_text = (root / "import-signing-identity.sh").read_text()
require("INPUTIA_P12_PASSWORD" in import_signing_text, "import-signing-missing-p12-password-env")
require("INPUTIA_SIGNING_P12_PASSWORD" in import_signing_text, "import-signing-missing-alt-p12-password-env")
require("missing-p12-password" in import_signing_text, "import-signing-missing-password-gate")
require("security import" in import_signing_text, "import-signing-missing-security-import")
require("-P \"$P12_PASSWORD\"" in import_signing_text, "import-signing-does-not-pass-explicit-password")
require("run_with_timeout" in import_signing_text, "import-signing-missing-timeout")
require("signingIdentityImportTimeoutSeconds" in import_signing_text, "import-signing-missing-timeout-output")
require("security find-identity -v -p codesigning" in import_signing_text, "import-signing-missing-identity-verification")
require("grep -F \"$IDENTITY\"" in import_signing_text, "import-signing-missing-exact-identity-check")
require("codesign_probe()" in import_signing_text, "import-signing-missing-codesign-probe")
require("signingIdentityCodesignProbe=true" in import_signing_text, "import-signing-missing-codesign-probe-success")
require("signingIdentityCodesignProbe=false" in import_signing_text, "import-signing-missing-codesign-probe-failure")
require("codesign-probe-failed" in import_signing_text, "import-signing-missing-codesign-probe-failed-reason")
require("unlock_keychain_with_password()" in import_signing_text, "import-signing-missing-keychain-unlock-helper")
require("set_key_partition_list_with_password()" in import_signing_text, "import-signing-missing-key-partition-helper")
require("signingIdentityKeyPartitionRepair=skipped reason=missing-keychain-password" in import_signing_text, "import-signing-missing-existing-identity-repair-skip")
require("set_key_partition_list_with_password || exit 15" in import_signing_text, "import-signing-existing-identity-does-not-repair-key-partition")
require("--timestamp=none" in import_signing_text, "import-signing-codesign-probe-missing-no-timestamp")
require("signingIdentityImportVerified=true" in import_signing_text, "import-signing-missing-verified-output")
require("security import \"$P12_PATH\"" in import_signing_text, "import-signing-does-not-import-configured-p12")
require("INPUTIA_ALLOW_REJECTED_SIGNATURE" not in import_signing_text, "import-signing-uses-rejected-signature-override")

require("user_host_state()" in verify_nongui_text, "verify-nongui-missing-user-host-baseline-helper")
require("VERIFY_USER_HOST_BASELINE" in verify_nongui_text, "verify-nongui-missing-user-host-baseline")
require("userHostUnchanged=true" in verify_nongui_text, "verify-nongui-missing-user-host-unchanged-output")
require("VERIFY_POST_INSTALL_USER_CONFLICT_ROOT" in verify_nongui_text, "verify-nongui-missing-post-install-user-conflict-root")
require("VERIFY_POST_INSTALL_USER_APP" in verify_nongui_text, "verify-nongui-missing-post-install-user-app-isolation")
require("VERIFY_POST_INSTALL_USER_LEGACY_APP" in verify_nongui_text, "verify-nongui-missing-post-install-user-legacy-isolation")
require("VERIFY_POST_INSTALL_USER_SETTINGS_APP" in verify_nongui_text, "verify-nongui-missing-post-install-user-settings-isolation")
require("INPUTIA_USER_APP=\"$VERIFY_POST_INSTALL_USER_APP\"" in verify_nongui_text, "verify-nongui-post-install-missing-user-app-env")
require("INPUTIA_USER_LEGACY_APP=\"$VERIFY_POST_INSTALL_USER_LEGACY_APP\"" in verify_nongui_text, "verify-nongui-post-install-missing-user-legacy-env")
require("INPUTIA_USER_SETTINGS_APP=\"$VERIFY_POST_INSTALL_USER_SETTINGS_APP\"" in verify_nongui_text, "verify-nongui-post-install-missing-user-settings-env")

clipboard_text = (root / "smoke-clipboard-recall.sh").read_text()
clipboard_restore_fn_index = clipboard_text.find("restore_clipboard()")
clipboard_restore_call_index = clipboard_text.find("restore_clipboard", clipboard_restore_fn_index + 1)
restore_index = clipboard_text.find('if [[ "$CLIPBOARD_CHANGED" == "1"')
clipboard_restorable_index = clipboard_text.find("inputia_require_text_clipboard_restorable")
clipboard_inputia_preflight_index = clipboard_text.find('inputia_require_process_not_running \\\n  "InputiaInputMethod" "clipboardRecallSmokeReady"')
require('"inputia-host-running" "-"' in clipboard_text, "clipboard-inputia-host-preflight-allows-override")
clipboard_select_index = clipboard_text.find("inputia_select_input_source_or_exit")
clipboard_prepare_debug_index = clipboard_text.find('inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"')
clipboard_clean_debug_index = clipboard_text.find('inputia_assert_debug_event_log_clean "$EVENT_LOG" "clipboardRecallSmokeReady" 13')
pbpaste_index = clipboard_text.find('ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste', clipboard_select_index)
pbcopy_index = clipboard_text.find('/usr/bin/printf \'%s\' "$expected" | /usr/bin/pbcopy', pbpaste_index)
changed_index = clipboard_text.find("CLIPBOARD_CHANGED=1", pbcopy_index)
clipboard_trap_index = clipboard_text.find("trap cleanup_smoke EXIT", clipboard_restore_call_index)
clipboard_osascript_index = clipboard_text.find("inputia_run_with_timeout", changed_index)
require(clipboard_restore_fn_index >= 0, "clipboard-missing-clipboard-restore-function")
require(clipboard_restore_call_index > clipboard_restore_fn_index, "clipboard-missing-clipboard-restore-call")
require(pbpaste_index >= 0, "clipboard-missing-original-capture")
require(pbcopy_index >= 0, "clipboard-missing-test-copy")
require(changed_index >= 0, "clipboard-missing-changed-flag")
require(restore_index >= 0, "clipboard-missing-conditional-restore")
require(clipboard_restorable_index >= 0, "clipboard-missing-text-restorable-gate")
require(clipboard_inputia_preflight_index >= 0, "clipboard-missing-inputia-host-preflight")
require(clipboard_select_index >= 0, "clipboard-missing-select-gate")
require(clipboard_prepare_debug_index >= 0, "clipboard-missing-debug-event-log-prepare")
require(clipboard_clean_debug_index >= 0, "clipboard-missing-debug-event-log-clean-assert")
require(clipboard_trap_index > clipboard_restore_call_index, "clipboard-missing-cleanup-trap-after-restore")
require(clipboard_osascript_index > changed_index, "clipboard-changed-after-osascript")
require(
    clipboard_restore_fn_index < restore_index < clipboard_inputia_preflight_index < clipboard_restorable_index < clipboard_restore_call_index < clipboard_trap_index < clipboard_prepare_debug_index < clipboard_clean_debug_index < clipboard_select_index < pbpaste_index < pbcopy_index < changed_index < clipboard_osascript_index,
    "clipboard-restore-order",
)
clipboard_cleanup_self_check_index = clipboard_text.find("INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK")
clipboard_cleanup_self_check_marker_index = clipboard_text.find("clipboardRecallCleanupSelfCheck=true phase=$self_check_phase")
require(clipboard_cleanup_self_check_index > clipboard_trap_index, "clipboard-cleanup-self-check-before-trap")
require(clipboard_cleanup_self_check_index < clipboard_select_index, "clipboard-cleanup-self-check-after-select")
require(clipboard_cleanup_self_check_marker_index > clipboard_cleanup_self_check_index, "clipboard-cleanup-self-check-missing-marker")
require('self_check_phase="after-clipboard-write"' in clipboard_text, "clipboard-cleanup-self-check-missing-write-phase")
require('self_check_phase="pasteboard-unavailable"' in clipboard_text, "clipboard-cleanup-self-check-missing-pasteboard-phase")
require("INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK_RC" in clipboard_text, "clipboard-cleanup-self-check-missing-rc-override")
require("on clearInputiaState()" in clipboard_text, "clipboard-missing-state-clear-handler")
require("key code 53" in clipboard_text, "clipboard-missing-escape-state-clear")
require("state-clear-leaked-text:" in clipboard_text, "clipboard-missing-state-clear-assertion")
require("clipboardRecallClearedText=" in clipboard_text, "clipboard-missing-cleared-text-output")
require("clipboardRecallSmokePassed=false reason=state-clear-leaked-text" in clipboard_text, "clipboard-missing-cleared-text-shell-assertion")
require("on resetClipboardRecallEventLog(eventLogPath)" in clipboard_text, "clipboard-missing-event-log-reset-handler")
require("set eof eventLogFile to 0" in clipboard_text, "clipboard-event-log-reset-does-not-truncate")
require("clipboardRecallEventLogResetAfterStateClear=" in clipboard_text, "clipboard-missing-event-log-reset-output")
require("clipboardRecallSmokePassed=false reason=event-log-reset-after-state-clear-failed" in clipboard_text, "clipboard-missing-event-log-reset-shell-assertion")
reset_event_log_index = clipboard_text.find("my resetClipboardRecallEventLog(eventLogPath)")
state_clear_assert_index = clipboard_text.find('if clearedText is not "" then error "state-clear-leaked-text:"')
reset_failure_assert_index = clipboard_text.find('if not eventLogResetAfterStateClear then error "clipboard-recall-event-log-reset-failed"')
pretrigger_guard_index = clipboard_text.find("my assertNoClipboardRecallBeforeTrigger(eventLogPath)")
trigger_index = clipboard_text.find("key code 9 using {control down, shift down}")
require(
    state_clear_assert_index >= 0
    and reset_event_log_index > state_clear_assert_index
    and reset_failure_assert_index > reset_event_log_index
    and pretrigger_guard_index > reset_failure_assert_index
    and trigger_index > pretrigger_guard_index,
    "clipboard-event-log-reset-order",
)
require("textEditWasRunningBefore=" in clipboard_text, "clipboard-missing-textedit-running-output")
require("textEditCleanupSucceeded=" in clipboard_text, "clipboard-missing-textedit-cleanup-output")
require("clipboardRecallSmokePassed=false reason=textedit-cleanup-failed" in clipboard_text, "clipboard-missing-textedit-cleanup-shell-assertion")
require("set cleanupSucceeded to true" in clipboard_text, "clipboard-cleanup-missing-success-flag")
require("set cleanupSucceeded to false" in clipboard_text, "clipboard-cleanup-missing-failure-flag")
require("return cleanupSucceeded" in clipboard_text, "clipboard-cleanup-missing-return-status")
require("on waitForClipboardRecallShown(eventLogPath)" in clipboard_text, "clipboard-missing-recall-shown-wait")
require("clipboardRecallShown" in clipboard_text, "clipboard-missing-shown-event-check")
require("clipboardRecallCommit index=0" in clipboard_text, "clipboard-missing-commit-event-check")
require("clipboardRecallCommit index=0 text=$expected" in clipboard_text, "clipboard-missing-commit-text-check")
require("on assertNoClipboardRecallBeforeTrigger(eventLogPath)" in clipboard_text, "clipboard-missing-pretrigger-recall-guard")
require("clipboard-recall-shown-before-trigger" in clipboard_text, "clipboard-missing-pretrigger-shown-error")
require("clipboard-recall-commit-before-trigger" in clipboard_text, "clipboard-missing-pretrigger-commit-error")
require(pretrigger_guard_index >= 0 and trigger_index > pretrigger_guard_index, "clipboard-pretrigger-guard-after-trigger")
require("on assertNoClipboardRecallCommitBeforeSelection(eventLogPath)" in clipboard_text, "clipboard-missing-preselection-commit-guard")
require("clipboard-recall-commit-before-selection" in clipboard_text, "clipboard-missing-preselection-commit-error")
shown_wait_index = clipboard_text.find("my waitForClipboardRecallShown(eventLogPath)")
preselection_guard_index = clipboard_text.find("my assertNoClipboardRecallCommitBeforeSelection(eventLogPath)")
selection_index = clipboard_text.find("key code 49", preselection_guard_index)
require(
    shown_wait_index >= 0 and preselection_guard_index > shown_wait_index and selection_index > preselection_guard_index,
    "clipboard-preselection-commit-guard-order",
)

textedit_text = (root / "smoke-textedit.sh").read_text()
require("on clearInputiaState()" in textedit_text, "textedit-missing-state-clear-handler")
require("key code 53" in textedit_text, "textedit-missing-escape-state-clear")
require("on assertDocumentCleared(docRef, labelName)" in textedit_text, "textedit-missing-state-clear-counting-assertion")
require("set stateClearPasses to stateClearPasses + 1" in textedit_text, "textedit-missing-state-clear-pass-counter")
require("textEditStateClearPasses=" in textedit_text, "textedit-missing-state-clear-pass-output")
require("textEditSmokePassed=false step=state-clear-count" in textedit_text, "textedit-missing-state-clear-count-shell-assertion")
require("state-clear-leaked-text:" in textedit_text, "textedit-missing-state-clear-assertion")
require("focus-lost:" in textedit_text, "textedit-missing-focus-lost-assertion")
require("quit saving no" in textedit_text, "textedit-missing-quit-saving-no")
require("arrowCandidateResult=" in textedit_text, "textedit-missing-arrow-candidate-result")
require('"arrow-down-commit"' in textedit_text, "textedit-missing-arrow-down-case")
require("key code 125" in textedit_text, "textedit-missing-arrow-down-key")
require("set docRef to make new document" in textedit_text, "textedit-missing-doc-ref-capture")
require("close docRef saving no" in textedit_text, "textedit-missing-doc-ref-close")
require('set text of docRef to ""' in textedit_text, "textedit-missing-doc-ref-clear")
require("set clearedText to text of docRef" in textedit_text, "textedit-missing-doc-ref-clear-assertion")
require("set resultText to text of docRef" in textedit_text, "textedit-missing-doc-ref-result-read")
require("set englishText to text of docRef" in textedit_text, "textedit-missing-doc-ref-shift-english-read")
require("set chineseText to text of docRef" in textedit_text, "textedit-missing-doc-ref-shift-chinese-read")
shift_chinese_clear_index = textedit_text.find('my assertDocumentCleared(docRef, "shift-chinese")')
shift_chinese_toggle_index = textedit_text.find("key down shift", shift_chinese_clear_index)
require(shift_chinese_clear_index >= 0 and shift_chinese_toggle_index > shift_chinese_clear_index, "textedit-shift-chinese-clear-not-before-toggle")
require("text of front document" not in textedit_text, "textedit-uses-front-document-text")
require("OSASCRIPT_FILE=" in textedit_text, "textedit-missing-osascript-temp-file")
require("inputia_run_with_timeout textedit-osascript" in textedit_text, "textedit-missing-osascript-timeout")
require("set cleanupSucceeded to true" in textedit_text, "textedit-cleanup-missing-success-flag")
require("set cleanupSucceeded to false" in textedit_text, "textedit-cleanup-missing-failure-flag")
require("return cleanupSucceeded" in textedit_text, "textedit-cleanup-missing-return-status")
require("textEditCleanupSucceeded=" in textedit_text, "textedit-missing-cleanup-output")
require("textEditSmokePassed=false step=textedit-cleanup-failed" in textedit_text, "textedit-missing-cleanup-shell-assertion")
require("INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK" in textedit_text, "textedit-missing-cleanup-self-check")
require("textEditCleanupSelfCheck=true phase=after-temp-write" in textedit_text, "textedit-missing-cleanup-self-check-marker")
require("INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK_RC" in textedit_text, "textedit-missing-cleanup-self-check-rc-override")
textedit_cleanup_self_check_index = textedit_text.find("INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK")
textedit_select_index = textedit_text.find("inputia_select_input_source_or_exit")
textedit_trap_index = textedit_text.find("trap cleanup_textedit_smoke EXIT")
require(textedit_trap_index < textedit_cleanup_self_check_index < textedit_select_index, "textedit-cleanup-self-check-order")

textedit_command_text = (root / "smoke-textedit-command-shortcuts.sh").read_text()
require("on clearInputiaState()" in textedit_command_text, "textedit-command-missing-state-clear-handler")
require("key code 53" in textedit_command_text, "textedit-command-missing-escape-state-clear")
require("on assertSourceBeforeCopy(docRef, sourceText)" in textedit_command_text, "textedit-command-missing-source-before-copy-guard")
require("textedit-command-state-clear-leaked-text:" in textedit_command_text, "textedit-command-missing-state-clear-leak-error")
textedit_command_guard_index = textedit_command_text.find("my assertSourceBeforeCopy(docRef, sourceText)")
textedit_command_a_index = textedit_command_text.find("key code 0 using {command down}")
require(textedit_command_guard_index >= 0 and textedit_command_a_index > textedit_command_guard_index, "textedit-command-source-guard-after-command-a")
require("focus-lost:" in textedit_command_text, "textedit-command-missing-focus-lost-assertion")
require("quit saving no" in textedit_command_text, "textedit-command-missing-quit-saving-no")
require("set docRef to make new document" in textedit_command_text, "textedit-command-missing-doc-ref-capture")
require("close docRef saving no" in textedit_command_text, "textedit-command-missing-doc-ref-close")
require("key code 0 using {command down}" in textedit_command_text, "textedit-command-missing-command-a")
require("key code 8 using {command down}" in textedit_command_text, "textedit-command-missing-command-c")
require("key code 9 using {command down}" in textedit_command_text, "textedit-command-missing-command-v")
require("set cleanupSucceeded to my cleanupDoc(docRef)" in textedit_command_text, "textedit-command-cleanup-missing-doc-status")
require("set cleanupSucceeded to false" in textedit_command_text, "textedit-command-cleanup-missing-failure-flag")
require("return cleanupSucceeded" in textedit_command_text, "textedit-command-cleanup-missing-return-status")
require("textEditCleanupSucceeded=" in textedit_command_text, "textedit-command-missing-cleanup-output")
require("textEditCommandShortcutSmokePassed=false step=textedit-cleanup-failed" in textedit_command_text, "textedit-command-missing-cleanup-shell-assertion")
require("commandSelectAllCopiedText=" in textedit_command_text, "textedit-command-missing-copy-result")
require("commandPasteResult=" in textedit_command_text, "textedit-command-missing-paste-result")
require("restore_clipboard" in textedit_command_text, "textedit-command-missing-clipboard-restore")
require("OSASCRIPT_FILE=" in textedit_command_text, "textedit-command-missing-osascript-temp-file")
require("inputia_run_with_timeout textedit-command-osascript" in textedit_command_text, "textedit-command-missing-osascript-timeout")
require("INPUTIA_TEXTEDIT_COMMAND_CLEANUP_SELF_CHECK" in textedit_command_text, "textedit-command-missing-cleanup-self-check")
require("textEditCommandCleanupSelfCheck=true phase=$self_check_phase" in textedit_command_text, "textedit-command-missing-cleanup-self-check-marker")
require('self_check_phase="after-clipboard-write"' in textedit_command_text, "textedit-command-cleanup-self-check-missing-write-phase")
require('self_check_phase="pasteboard-unavailable"' in textedit_command_text, "textedit-command-cleanup-self-check-missing-pasteboard-phase")
require("INPUTIA_TEXTEDIT_COMMAND_CLEANUP_SELF_CHECK_RC" in textedit_command_text, "textedit-command-missing-cleanup-self-check-rc-override")

safari_command_text = (root / "smoke-safari-command-shortcuts.sh").read_text()
require("on clearInputiaState()" in safari_command_text, "safari-command-missing-state-clear-handler")
require("key code 53" in safari_command_text, "safari-command-missing-escape-state-clear")
require("on assertCommandSourceBeforeCopy(smokeWindowId)" in safari_command_text, "safari-command-missing-source-before-copy-guard")
require("safari-command-state-clear-leaked-title:" in safari_command_text, "safari-command-missing-state-clear-leak-error")
require("safariCommandStateClearBeforeCopy=true" in safari_command_text, "safari-command-missing-state-clear-output")
require("safariCommandShortcutSmokePassed=false step=state-clear-before-copy" in safari_command_text, "safari-command-missing-state-clear-shell-assertion")
safari_command_guard_index = safari_command_text.find("my assertCommandSourceBeforeCopy(smokeWindowId)")
safari_command_output_index = safari_command_text.find("safariCommandStateClearBeforeCopy=true")
safari_command_a_index = safari_command_text.find("key code 0 using {command down}")
require(safari_command_guard_index >= 0 and safari_command_output_index > safari_command_guard_index and safari_command_a_index > safari_command_guard_index, "safari-command-source-guard-after-command-a")
require("key code 0 using {command down}" in safari_command_text, "safari-command-missing-command-a")
require("key code 8 using {command down}" in safari_command_text, "safari-command-missing-command-c")
require("key code 9 using {command down}" in safari_command_text, "safari-command-missing-command-v")
require("commandSelectAllCopiedText=" in safari_command_text, "safari-command-missing-copy-result")
require("commandPasteTitle=" in safari_command_text, "safari-command-missing-paste-result")
require("restore_clipboard" in safari_command_text, "safari-command-missing-clipboard-restore")
require("inputia_run_with_timeout safari-command-osascript" in safari_command_text, "safari-command-missing-osascript-timeout")
require("INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK" in safari_command_text, "safari-command-missing-cleanup-self-check")
require("safariCommandCleanupSelfCheck=true phase=$self_check_phase" in safari_command_text, "safari-command-missing-cleanup-self-check-marker")
require('self_check_phase="after-clipboard-write"' in safari_command_text, "safari-command-cleanup-self-check-missing-write-phase")
require('self_check_phase="pasteboard-unavailable"' in safari_command_text, "safari-command-cleanup-self-check-missing-pasteboard-phase")
require("INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK_RC" in safari_command_text, "safari-command-missing-cleanup-self-check-rc-override")

clipboard_mutation_contracts = [
    ("textedit-command", textedit_command_text, "/usr/bin/printf '' | /usr/bin/pbcopy"),
    ("safari-command", safari_command_text, "/usr/bin/printf '' | /usr/bin/pbcopy"),
]

for label, text, pbcopy_marker in clipboard_mutation_contracts:
    restore_fn_index = text.find("restore_clipboard()")
    restore_call_index = text.find("restore_clipboard", restore_fn_index + 1)
    clipboard_restorable_index = text.find("inputia_require_text_clipboard_restorable")
    select_index = text.find("inputia_select_input_source_or_exit")
    pbpaste_index = text.find('ORIGINAL_CLIPBOARD="$(/usr/bin/pbpaste', select_index)
    pbcopy_index = text.find(pbcopy_marker, pbpaste_index)
    changed_index = text.find("CLIPBOARD_CHANGED=1", pbcopy_index)
    trap_index = text.find("trap ", restore_call_index)
    osascript_index = text.find("inputia_run_with_timeout", changed_index)
    require(restore_fn_index >= 0, f"{label}-missing-clipboard-restore-function")
    require(restore_call_index > restore_fn_index, f"{label}-missing-clipboard-restore-call")
    require(clipboard_restorable_index >= 0, f"{label}-missing-text-restorable-gate")
    require(trap_index > restore_call_index, f"{label}-missing-cleanup-trap-after-restore")
    require(select_index >= 0, f"{label}-missing-select-gate")
    require(pbpaste_index >= 0, f"{label}-missing-original-clipboard-capture")
    require(pbcopy_index >= 0, f"{label}-missing-test-clipboard-write")
    require(changed_index >= 0, f"{label}-missing-clipboard-changed-flag")
    require(osascript_index > changed_index, f"{label}-clipboard-changed-after-osascript")
    require(restore_fn_index < clipboard_restorable_index < restore_call_index < trap_index < select_index < pbpaste_index < pbcopy_index < changed_index < osascript_index, f"{label}-clipboard-restore-order")

safari_typing_text = (root / "smoke-safari-typing.sh").read_text()
require("on clearInputiaState()" in safari_typing_text, "safari-typing-missing-state-clear-handler")
require("key code 53" in safari_typing_text, "safari-typing-missing-escape-state-clear")
require("on assertEmptyBeforeTyping(smokeWindowId)" in safari_typing_text, "safari-typing-missing-empty-before-typing-guard")
require("safari-typing-state-clear-leaked-title:" in safari_typing_text, "safari-typing-missing-state-clear-leak-error")
require("safariTypingStateClearBeforeTyping=true" in safari_typing_text, "safari-typing-missing-state-clear-output")
require("safariTypingSmokePassed=false reason=state-clear-before-typing-missing" in safari_typing_text, "safari-typing-missing-state-clear-shell-assertion")
safari_typing_guard_index = safari_typing_text.find("my assertEmptyBeforeTyping(smokeWindowId)")
safari_typing_output_index = safari_typing_text.find("safariTypingStateClearBeforeTyping=true")
safari_typing_keys_index = safari_typing_text.find("$keys")
require(safari_typing_guard_index >= 0 and safari_typing_output_index > safari_typing_guard_index and safari_typing_keys_index > safari_typing_guard_index, "safari-typing-empty-guard-after-typing")
require("INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK" in safari_typing_text, "safari-typing-missing-cleanup-self-check")
require("safariTypingCleanupSelfCheck=true phase=after-temp-write" in safari_typing_text, "safari-typing-missing-cleanup-self-check-marker")
require("INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK_RC" in safari_typing_text, "safari-typing-missing-cleanup-self-check-rc-override")
safari_typing_cleanup_self_check_index = safari_typing_text.find("INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK")
safari_typing_select_index = safari_typing_text.find("inputia_select_input_source_or_exit")
safari_typing_trap_index = safari_typing_text.find("trap cleanup_smoke EXIT")
require(safari_typing_trap_index < safari_typing_cleanup_self_check_index < safari_typing_select_index, "safari-typing-cleanup-self-check-order")

safari_enter_text = (root / "smoke-safari-enter.sh").read_text()
require("on clearInputiaState()" in safari_enter_text, "safari-enter-missing-state-clear-handler")
require("key code 53" in safari_enter_text, "safari-enter-missing-escape-state-clear")
require("on assertNoRawCommitBeforeTyping(eventLogPath)" in safari_enter_text, "safari-enter-missing-pretyping-commit-guard")
require("safari-enter-raw-commit-before-typing" in safari_enter_text, "safari-enter-missing-pretyping-commit-error")
require("on resetSafariEnterEventLog(eventLogPath)" in safari_enter_text, "safari-enter-missing-event-log-reset-handler")
require("set eof eventLogFile to 0" in safari_enter_text, "safari-enter-event-log-reset-does-not-truncate")
require("safariEnterEventLogResetAfterStateClear=" in safari_enter_text, "safari-enter-missing-event-log-reset-output")
require("safariEnterSmokePassed=false reason=event-log-reset-after-state-clear-failed" in safari_enter_text, "safari-enter-missing-event-log-reset-shell-assertion")
reset_safari_enter_event_log_index = safari_enter_text.find("my resetSafariEnterEventLog(eventLogPath)")
pretyping_guard_index = safari_enter_text.find("my assertNoRawCommitBeforeTyping(eventLogPath)")
safari_enter_key_index = safari_enter_text.find("key code 0")
require(
    reset_safari_enter_event_log_index >= 0
    and pretyping_guard_index > reset_safari_enter_event_log_index
    and safari_enter_key_index > pretyping_guard_index,
    "safari-enter-event-log-reset-order",
)
require(pretyping_guard_index >= 0 and safari_enter_key_index > pretyping_guard_index, "safari-enter-pretyping-guard-after-typing")
require("INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK" in safari_enter_text, "safari-enter-missing-cleanup-self-check")
require("safariEnterCleanupSelfCheck=true phase=$self_check_phase" in safari_enter_text, "safari-enter-missing-cleanup-self-check-marker")
require('self_check_phase="after-temp-write"' in safari_enter_text, "safari-enter-cleanup-self-check-missing-temp-phase")
require("launchctl-env-unavailable" in safari_enter_text, "safari-enter-cleanup-self-check-missing-launchctl-phase")
require("INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK_RC" in safari_enter_text, "safari-enter-missing-cleanup-self-check-rc-override")

safari_diagnose_text = (root / "diagnose-safari-input-source.sh").read_text()

def require_error_cleanup(label, text, anchor, cleanup_markers, rethrow_marker="error errMsg number errNum"):
    anchor_index = text.find(anchor)
    require(anchor_index >= 0, f"{label}-missing-error-cleanup-anchor")
    error_index = text.find("on error errMsg number errNum", anchor_index)
    require(error_index > anchor_index, f"{label}-missing-error-handler")
    rethrow_index = text.find(rethrow_marker, error_index + len("on error errMsg number errNum"))
    require(rethrow_index > error_index, f"{label}-missing-error-rethrow")
    error_block = text[error_index:rethrow_index]
    for marker in cleanup_markers:
        require(marker in error_block, f"{label}-error-handler-missing-{marker}")

require_error_cleanup(
    "textedit-run-case",
    textedit_text,
    "on runCase(caseName, caseKind)",
    ("cleanupDoc(docRef)",),
    "error caseName &",
)
require_error_cleanup(
    "textedit-shift-case",
    textedit_text,
    "on runShiftCase()",
    ("cleanupDoc(docRef)",),
    'error "shift:" &',
)
require_error_cleanup(
    "textedit-top-level",
    textedit_text,
    "set outputLines to {}",
    ("my cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId)",),
)
require_error_cleanup(
    "textedit-command-top-level",
    textedit_command_text,
    "try\n  tell application \"TextEdit\"",
    ("cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, docRef)",),
)
require_error_cleanup(
    "clipboard-top-level",
    clipboard_text,
    "try\n  tell application \"TextEdit\"",
    ("cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, smokeDocument)",),
)
for label, text in (
    ("safari-typing-top-level", safari_typing_text),
    ("safari-command-top-level", safari_command_text),
    ("safari-enter-top-level", safari_enter_text),
):
    require_error_cleanup(
        label,
        text,
        "try\n  tell application \"Safari\"",
        ("closeSmokeWindow(smokeWindowId)", "restoreFrontmost(previousBundleId)"),
    )

for label, text, success_marker in (
    ("safari-typing", safari_typing_text, "safariTypingSmokePassed=false reason=smoke-window-not-closed"),
    ("safari-command", safari_command_text, "safariCommandShortcutSmokePassed=false step=smoke-window-close"),
    ("safari-enter", safari_enter_text, "safariEnterSmokePassed=false reason=smoke-window-not-closed"),
):
    require('return "true"' in text, f"{label}-close-window-missing-success-return")
    require('return "false"' in text, f"{label}-close-window-missing-failure-return")
    require("safariSmokeWindowClosed=" in text, f"{label}-missing-smoke-window-closed-output")
    require(success_marker in text, f"{label}-missing-smoke-window-closed-assertion")

for marker in (
    "SAFARI_DIAGNOSE_WINDOW_ID",
    "PREVIOUS_BUNDLE_ID",
    "close_safari_diagnose_window",
    'tell application "Safari" to close window id smokeWindowId',
    'tell application id previousBundleId to activate',
    "safariDiagnoseWindowClosed=",
    "safariInputSourceDiagnosisPassed=false reason=diagnose-window-not-closed",
):
    require(marker in safari_diagnose_text, f"safari-diagnose-missing-{marker}")
safari_diagnose_open_index = safari_diagnose_text.find("SAFARI_DIAGNOSE_WINDOW_ID=")
safari_diagnose_close_index = safari_diagnose_text.find('window_closed="$(close_safari_diagnose_window)"')
safari_diagnose_clear_index = safari_diagnose_text.find('SAFARI_DIAGNOSE_WINDOW_ID=""', safari_diagnose_close_index)
safari_diagnose_marker_index = safari_diagnose_text.find("safariDiagnoseWindowClosed=", safari_diagnose_clear_index)
require(
    safari_diagnose_open_index >= 0
    and safari_diagnose_close_index > safari_diagnose_open_index
    and safari_diagnose_clear_index > safari_diagnose_close_index
    and safari_diagnose_marker_index > safari_diagnose_clear_index,
    "safari-diagnose-window-close-order",
)

require("set smokeDocument to missing value" in clipboard_text, "clipboard-missing-doc-ref-init")
require("set smokeDocument to make new document" in clipboard_text, "clipboard-missing-doc-ref-capture")
require('set text of smokeDocument to ""' in clipboard_text, "clipboard-missing-doc-ref-clear")
require("set clearedText to text of smokeDocument" in clipboard_text, "clipboard-missing-doc-ref-clear-assertion")
require("set resultText to text of smokeDocument" in clipboard_text, "clipboard-missing-doc-ref-result-read")
require("text of front document" not in clipboard_text, "clipboard-uses-front-document-text")
require("close smokeDocument saving no" in clipboard_text, "clipboard-missing-doc-ref-close")
require("quit saving no" in clipboard_text, "clipboard-missing-quit-saving-no")
require("focus-lost:" in clipboard_text, "clipboard-missing-focus-lost-assertion")
require("cleanupTextEdit(textEditWasRunning, preexistingTextEditDocumentCount, previousBundleId, smokeDocument)" in clipboard_text, "clipboard-missing-doc-ref-cleanup-call")
require("OSASCRIPT_FILE=" in clipboard_text, "clipboard-missing-osascript-temp-file")
require("inputia_run_with_timeout clipboard-recall-osascript" in clipboard_text, "clipboard-missing-osascript-timeout")

for filename in ("smoke-clipboard-recall.sh", "smoke-safari-enter.sh"):
    text = (root / filename).read_text()
    inputia_preflight_index = text.find('inputia_require_process_not_running \\\n  "InputiaInputMethod"')
    capture_index = text.find("inputia_capture_debug_events_env")
    trap_index = text.find("trap cleanup_smoke EXIT")
    select_index = text.find("inputia_select_input_source_or_exit")
    prepare_index = text.find('inputia_prepare_debug_event_log "$EVENT_LOG" "$EVENT_LOG_PROVIDED"')
    clean_assert_index = text.find('inputia_assert_debug_event_log_clean "$EVENT_LOG"')
    setenv_index = text.find("inputia_set_debug_events_env_or_exit", clean_assert_index)
    killall_index = text.find("/usr/bin/killall InputiaInputMethod", setenv_index)
    provided_index = text.find('if [[ -n "$EVENT_LOG_PROVIDED" ]]')
    provided_cleanup_index = text.find('inputia_cleanup_smoke_files', provided_index)
    generated_cleanup_index = text.find('"$EVENT_LOG"', provided_cleanup_index)
    require(inputia_preflight_index >= 0, f"{filename}-missing-inputia-host-preflight")
    require('"inputia-host-running" "-"' in text, f"{filename}-inputia-host-preflight-allows-override")
    require("INPUTIA_HOST_SMOKE_ALLOW_EXISTING" not in text, f"{filename}-inputia-host-preflight-has-allow-override")
    require(capture_index >= 0, f"{filename}-missing-debug-env-capture")
    require(trap_index >= 0, f"{filename}-missing-cleanup-trap")
    require(select_index >= 0, f"{filename}-missing-select-before-debug")
    require(prepare_index >= 0, f"{filename}-missing-debug-log-prepare")
    require(clean_assert_index > prepare_index, f"{filename}-missing-debug-log-clean-assert")
    require(setenv_index >= 0, f"{filename}-missing-debug-setenv-helper")
    require(killall_index >= 0, f"{filename}-missing-host-restart")
    require(inputia_preflight_index < select_index < setenv_index < killall_index, f"{filename}-host-preflight-order")
    require('/bin/rm -f "$EVENT_LOG"' not in text, f"{filename}-unconditional-event-log-delete")
    require(capture_index < trap_index < prepare_index < clean_assert_index < select_index < setenv_index < killall_index, f"{filename}-debug-env-order")
    require(provided_index >= 0, f"{filename}-missing-provided-event-log-branch")
    require(provided_cleanup_index >= 0, f"{filename}-missing-provided-cleanup")
    require(generated_cleanup_index > provided_cleanup_index, f"{filename}-missing-generated-event-log-cleanup")

verify_system_text = (root / "verify-system.sh").read_text()
current_source_section = verify_system_text.find('section "current source"')
tis_current_index = verify_system_text.find('"$TIS_TOOL" --dump-current-input-source')
host_current_index = verify_system_text.find('run_inputia dumpCurrentInputSource')
require(current_source_section >= 0, "verify-system-missing-current-source-section")
require(tis_current_index > current_source_section, "verify-system-missing-tis-current-source")
require(host_current_index > tis_current_index, "verify-system-missing-host-current-source-fallback")
require("TEMP_FILES=()" in verify_system_text, "verify-system-missing-temp-file-registry")
require('TEMP_FILES+=("$output")' in verify_system_text, "verify-system-missing-run-inputia-temp-cleanup-registration")
require('TEMP_FILES+=("$launchservices_output")' in verify_system_text, "verify-system-missing-launchservices-temp-cleanup-registration")

install_system_text = (root / "install-system.sh").read_text()
install_menu_restart_index = install_system_text.find("killall TextInputMenuAgent")
install_systemui_restart_index = install_system_text.find("killall SystemUIServer")
install_post_refresh_register_index = install_system_text.find("inputia-register-after-refresh")
install_post_refresh_normalize_index = install_system_text.find("inputia-normalize-after-refresh")
install_manual_add_index = install_system_text.find("systemInstallTISReady=false reason=manual-add-required")
install_manual_add_action_index = install_system_text.find("systemInstallRequiredAction=add-input-source-in-system-settings")
install_preflight_index = install_system_text.find("require_no_verification_processes")
install_build_index = install_system_text.find('"$ROOT_DIR/build.sh"')
install_kill_host_index = install_system_text.find("killall InputiaInputMethod")
require("assess_app()" in install_system_text, "install-system-missing-app-assessment-helper")
require("INPUTIA_ALLOW_REJECTED_SIGNATURE" in install_system_text, "install-system-missing-rejected-signature-override")
require("systemInstallInputiaUsable=false reason=signature-rejected" in install_system_text, "install-system-missing-signature-rejected-gate")
require("systemInstallRequiredAction=sign-with-accepted-identity" in install_system_text, "install-system-missing-signature-required-action")
require("systemInstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts" in install_system_text, "install-system-missing-signature-hint")
require("systemInstallAction=stop-before-tis-registration" in install_system_text, "install-system-signature-rejection-does-not-stop-before-tis-registration")
require("systemInstallTISReady=false reason=manual-add-required" in install_system_text, "install-system-missing-manual-add-readiness-output")
require("systemInstallNextStep=System Settings > Keyboard > Text Input > Edit > Add Inputia" in install_system_text, "install-system-missing-manual-add-next-step")
require(install_menu_restart_index >= 0, "install-system-missing-textinputmenuagent-restart")
require(install_systemui_restart_index > install_menu_restart_index, "install-system-missing-systemui-restart")
require(install_post_refresh_register_index > install_systemui_restart_index, "install-system-missing-post-refresh-register")
require(install_post_refresh_normalize_index < 0, "install-system-still-runs-post-refresh-normalize")
require(install_manual_add_index > install_post_refresh_register_index, "install-system-missing-manual-add-tis-output")
require(install_manual_add_action_index > install_manual_add_index, "install-system-missing-manual-add-required-action")
require("detect_verification_processes()" in install_system_text, "install-system-missing-verification-process-detection")
require("systemInstallReady=false reason=verification-running" in install_system_text, "install-system-missing-verification-running-output")
require("systemInstallBlockingProcess:" in install_system_text, "install-system-missing-blocking-process-output")
require("gui-smoke-suite" in install_system_text, "install-system-missing-gui-smoke-suite-process-guard")
require("INPUTIA_INSTALL_PREFLIGHT_SELF_CHECK" in install_system_text, "install-system-missing-preflight-self-check")
require("systemInstallPreflightSelfCheck=true" in install_system_text, "install-system-missing-preflight-self-check-success")
require(
    install_preflight_index >= 0
    and install_build_index > install_preflight_index
    and install_kill_host_index > install_preflight_index,
    "install-system-mutates-before-verification-preflight",
)

install_user_text = (root / "install-user.sh").read_text()
require('TMP_ROOT="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-install-user.XXXXXX")"' in install_user_text, "install-user-missing-tmp-root")
require('/bin/rm -rf "$TMP_ROOT"' in install_user_text, "install-user-missing-tmp-root-cleanup")
require("trap cleanup EXIT" in install_user_text, "install-user-missing-cleanup-trap")
require("INPUTIA_INSTALL_USER_SKIP_BUILD" in install_user_text, "install-user-missing-skip-build-mode")
require("userInstallBuild=skipped reason=using-existing-build" in install_user_text, "install-user-missing-skip-build-output")
require("userInstallRequiredAction=run-build-before-skip-build-install" in install_user_text, "install-user-missing-skip-build-required-action")
require("detect_verification_processes()" in install_user_text, "install-user-missing-verification-process-detection")
require("userInstallReady=false reason=verification-running" in install_user_text, "install-user-missing-verification-running-output")
require("userInstallBlockingProcess:" in install_user_text, "install-user-missing-blocking-process-output")
require("INPUTIA_INSTALL_USER_PREFLIGHT_SELF_CHECK" in install_user_text, "install-user-missing-preflight-self-check")
require("userInstallPreflightSelfCheck=true" in install_user_text, "install-user-missing-preflight-self-check-success")
install_user_preflight_index = install_user_text.find("require_no_verification_processes")
install_user_build_index = install_user_text.find('if "$ROOT_DIR/build.sh"')
install_user_kill_index = install_user_text.find("killall InputiaInputMethod")
require(install_user_preflight_index >= 0, "install-user-missing-preflight-call")
require(install_user_build_index > install_user_preflight_index, "install-user-build-before-verification-preflight")
require(install_user_kill_index > install_user_preflight_index, "install-user-kill-before-verification-preflight")

build_script_text = (root / "build.sh").read_text()
require("detect_verification_processes()" in build_script_text, "build-missing-verification-process-detection")
require("require_no_verification_processes" in build_script_text, "build-missing-verification-process-preflight")
require("buildReady=false reason=verification-running" in build_script_text, "build-missing-verification-running-output")
require("buildBlockingProcess:" in build_script_text, "build-missing-blocking-process-output")
require("gui-smoke-suite" in build_script_text, "build-missing-gui-smoke-suite-process-guard")
require("INPUTIA_BUILD_PREFLIGHT_SELF_CHECK" in build_script_text, "build-missing-preflight-self-check")
require("buildPreflightSelfCheck=true" in build_script_text, "build-missing-preflight-self-check-success")
require("buildSigned=false reason=codesign-failed target=input-method" in build_script_text, "build-missing-codesign-failed-input-method-output")
require("buildSigned=false reason=codesign-failed target=settings-launcher" in build_script_text, "build-missing-codesign-failed-settings-output")
require('exit 31' in build_script_text, "build-missing-input-method-codesign-fail-exit")
require('exit 32' in build_script_text, "build-missing-settings-codesign-fail-exit")
build_preflight_index = build_script_text.find("require_no_verification_processes")
build_remove_index = build_script_text.find('rm -rf "$APP_DIR" "$SETTINGS_APP_DIR"')
require(
    build_preflight_index >= 0 and build_remove_index > build_preflight_index,
    "build-removes-app-before-verification-preflight",
)

build_pkg_text = (root / "build-pkg.sh").read_text()
require("detect_verification_processes()" in build_pkg_text, "build-pkg-missing-verification-process-detection")
require("require_no_verification_processes" in build_pkg_text, "build-pkg-missing-verification-process-preflight")
require("buildPkgReady=false reason=verification-running" in build_pkg_text, "build-pkg-missing-verification-running-output")
require("buildPkgBlockingProcess:" in build_pkg_text, "build-pkg-missing-blocking-process-output")
require("gui-smoke-suite" in build_pkg_text, "build-pkg-missing-gui-smoke-suite-process-guard")
require("INPUTIA_BUILD_PKG_PREFLIGHT_SELF_CHECK" in build_pkg_text, "build-pkg-missing-preflight-self-check")
require("buildPkgPreflightSelfCheck=true" in build_pkg_text, "build-pkg-missing-preflight-self-check-success")
require("require_pkg_sign_identity_if_requested()" in build_pkg_text, "build-pkg-missing-sign-identity-preflight")
require("buildPkgSignIdentityRequested=true" in build_pkg_text, "build-pkg-missing-sign-identity-request-output")
require("buildPkgSignIdentityValid=false reason=missing-developer-id-installer-identity" in build_pkg_text, "build-pkg-missing-sign-identity-missing-output")
require("buildPkgReady=false reason=missing-pkg-sign-identity" in build_pkg_text, "build-pkg-missing-sign-identity-ready-output")
require("buildPkgRequiredAction=import-developer-id-installer-identity" in build_pkg_text, "build-pkg-missing-sign-identity-required-action")
require("buildPkgSignIdentityValid=false reason=not-developer-id-installer" in build_pkg_text, "build-pkg-missing-wrong-sign-identity-output")
require("buildPkgSigned=false reason=productsign-failed" in build_pkg_text, "build-pkg-missing-productsign-failed-output")
require("buildPkgSigned=false reason=unsigned-local-package" in build_pkg_text, "build-pkg-missing-unsigned-local-output")
build_pkg_preflight_index = build_pkg_text.find("require_no_verification_processes")
build_pkg_sign_preflight_index = build_pkg_text.find("require_pkg_sign_identity_if_requested")
build_pkg_build_index = build_pkg_text.find('"$ROOT_DIR/build.sh"')
build_pkg_remove_index = build_pkg_text.find('rm -rf "$PKG_SCRIPTS_DIR" "$DIST_DIR"')
require(build_pkg_preflight_index >= 0, "build-pkg-missing-preflight-call")
require(build_pkg_sign_preflight_index > build_pkg_preflight_index, "build-pkg-sign-preflight-before-process-preflight")
require(build_pkg_build_index > build_pkg_sign_preflight_index, "build-pkg-build-before-sign-preflight")
require(build_pkg_build_index > build_pkg_preflight_index, "build-pkg-build-before-verification-preflight")
require(build_pkg_remove_index > build_pkg_preflight_index, "build-pkg-removes-dist-before-verification-preflight")

notarize_app_text = (root / "notarize-app.sh").read_text()
require("INPUTIA_NOTARIZE_APP_PREFLIGHT_ONLY" in notarize_app_text, "notarize-app-missing-preflight-only-mode")
require("Developer ID Application:" in notarize_app_text, "notarize-app-missing-developer-id-signature-check")
require("notarizeAppRequiredAction=rebuild-with-developer-id-application" in notarize_app_text, "notarize-app-missing-developer-id-required-action")
require("xcrun notarytool history --keychain-profile" in notarize_app_text, "notarize-app-missing-notary-profile-check")
require("xcrun notarytool submit" in notarize_app_text, "notarize-app-missing-notary-submit")
require("--keychain-profile" in notarize_app_text, "notarize-app-missing-keychain-profile-submit")
require("--wait" in notarize_app_text, "notarize-app-missing-submit-wait")
require("xcrun stapler staple" in notarize_app_text, "notarize-app-missing-staple")
require("xcrun stapler validate" in notarize_app_text, "notarize-app-missing-staple-validate")
require("spctl --assess --type execute" in notarize_app_text, "notarize-app-missing-post-staple-spctl")

notarize_pkg_text = (root / "notarize-pkg.sh").read_text()
require("INPUTIA_NOTARIZE_PKG_PREFLIGHT_ONLY" in notarize_pkg_text, "notarize-pkg-missing-preflight-only-mode")
require("Developer ID Installer:" in notarize_pkg_text, "notarize-pkg-missing-developer-id-installer-check")
require("notarizePkgRequiredAction=rebuild-pkg-with-developer-id-installer" in notarize_pkg_text, "notarize-pkg-missing-developer-id-installer-required-action")
require("pkgutil --check-signature" in notarize_pkg_text, "notarize-pkg-missing-pkg-signature-check")
require("spctl --assess --type install" in notarize_pkg_text, "notarize-pkg-missing-install-assessment")
require("xcrun notarytool history --keychain-profile" in notarize_pkg_text, "notarize-pkg-missing-notary-profile-check")
require("xcrun notarytool submit" in notarize_pkg_text, "notarize-pkg-missing-notary-submit")
require("--keychain-profile" in notarize_pkg_text, "notarize-pkg-missing-keychain-profile-submit")
require("--wait" in notarize_pkg_text, "notarize-pkg-missing-submit-wait")
require("xcrun stapler staple" in notarize_pkg_text, "notarize-pkg-missing-staple")
require("xcrun stapler validate" in notarize_pkg_text, "notarize-pkg-missing-staple-validate")

open_settings_text = (root / "open-settings.sh").read_text()
require("detect_verification_processes()" in open_settings_text, "open-settings-missing-verification-process-detection")
require("require_no_verification_processes" in open_settings_text, "open-settings-missing-verification-process-preflight")
require("openSettingsReady=false reason=verification-running" in open_settings_text, "open-settings-missing-verification-running-output")
require("openSettingsBlockingProcess:" in open_settings_text, "open-settings-missing-blocking-process-output")
require("gui-smoke-suite" in open_settings_text, "open-settings-missing-gui-smoke-suite-process-guard")
require("INPUTIA_OPEN_SETTINGS_PREFLIGHT_SELF_CHECK" in open_settings_text, "open-settings-missing-preflight-self-check")
require("openSettingsPreflightSelfCheck=true" in open_settings_text, "open-settings-missing-preflight-self-check-success")
open_settings_preflight_index = open_settings_text.find("require_no_verification_processes")
open_settings_first_open_index = open_settings_text.find("/usr/bin/open -n")
require(open_settings_preflight_index >= 0, "open-settings-missing-preflight-call")
require(open_settings_first_open_index > open_settings_preflight_index, "open-settings-opens-before-verification-preflight")

open_installer_text = (root / "open-installer.sh").read_text()
require("detect_verification_processes()" in open_installer_text, "open-installer-missing-verification-process-detection")
require("require_no_verification_processes" in open_installer_text, "open-installer-missing-verification-process-preflight")
require("openInstallerReady=false reason=verification-running" in open_installer_text, "open-installer-missing-verification-running-output")
require("openInstallerBlockingProcess:" in open_installer_text, "open-installer-missing-blocking-process-output")
require("gui-smoke-suite" in open_installer_text, "open-installer-missing-gui-smoke-suite-process-guard")
require("INPUTIA_OPEN_INSTALLER_PREFLIGHT_SELF_CHECK" in open_installer_text, "open-installer-missing-preflight-self-check")
require("openInstallerPreflightSelfCheck=true" in open_installer_text, "open-installer-missing-preflight-self-check-success")
open_installer_preflight_index = open_installer_text.find("require_no_verification_processes")
open_installer_build_index = open_installer_text.find('"$ROOT_DIR/build-pkg.sh"')
open_installer_open_index = open_installer_text.find('/usr/bin/open "$pkg_path"')
require(open_installer_preflight_index >= 0, "open-installer-missing-preflight-call")
require(open_installer_build_index > open_installer_preflight_index, "open-installer-build-before-verification-preflight")
require(open_installer_open_index > open_installer_preflight_index, "open-installer-opens-before-verification-preflight")

uninstall_system_text = (root / "uninstall-system.sh").read_text()
require("detect_verification_processes()" in uninstall_system_text, "uninstall-system-missing-verification-process-detection")
require("require_no_verification_processes" in uninstall_system_text, "uninstall-system-missing-verification-process-preflight")
require("systemUninstallReady=false reason=verification-running" in uninstall_system_text, "uninstall-system-missing-verification-running-output")
require("systemUninstallBlockingProcess:" in uninstall_system_text, "uninstall-system-missing-blocking-process-output")
require("gui-smoke-suite" in uninstall_system_text, "uninstall-system-missing-gui-smoke-suite-process-guard")
require("INPUTIA_UNINSTALL_PREFLIGHT_SELF_CHECK" in uninstall_system_text, "uninstall-system-missing-preflight-self-check")
require("systemUninstallPreflightSelfCheck=true" in uninstall_system_text, "uninstall-system-missing-preflight-self-check-success")
uninstall_system_preflight_index = uninstall_system_text.find("require_no_verification_processes")
uninstall_system_disable_index = uninstall_system_text.find("--disable-input-source")
uninstall_system_rm_index = uninstall_system_text.find("/bin/zsh -c \"$remove_command\"")
uninstall_system_admin_index = uninstall_system_text.find("with administrator privileges")
uninstall_system_kill_index = uninstall_system_text.find("killall InputiaInputMethod")
require(uninstall_system_preflight_index >= 0, "uninstall-system-missing-preflight-call")
require(uninstall_system_disable_index > uninstall_system_preflight_index, "uninstall-system-disables-before-verification-preflight")
require(uninstall_system_rm_index > uninstall_system_preflight_index, "uninstall-system-removes-before-verification-preflight")
require(uninstall_system_admin_index > uninstall_system_preflight_index, "uninstall-system-prompts-admin-before-verification-preflight")
require(uninstall_system_kill_index > uninstall_system_preflight_index, "uninstall-system-kills-before-verification-preflight")

uninstall_user_text = (root / "uninstall-user.sh").read_text()
require("detect_verification_processes()" in uninstall_user_text, "uninstall-user-missing-verification-process-detection")
require("require_no_verification_processes" in uninstall_user_text, "uninstall-user-missing-verification-process-preflight")
require("userUninstallReady=false reason=verification-running" in uninstall_user_text, "uninstall-user-missing-verification-running-output")
require("userUninstallBlockingProcess:" in uninstall_user_text, "uninstall-user-missing-blocking-process-output")
require("gui-smoke-suite" in uninstall_user_text, "uninstall-user-missing-gui-smoke-suite-process-guard")
require("INPUTIA_UNINSTALL_USER_PREFLIGHT_SELF_CHECK" in uninstall_user_text, "uninstall-user-missing-preflight-self-check")
require("userUninstallPreflightSelfCheck=true" in uninstall_user_text, "uninstall-user-missing-preflight-self-check-success")
uninstall_user_preflight_index = uninstall_user_text.find("require_no_verification_processes")
uninstall_user_disable_index = uninstall_user_text.find("--disable-input-source")
uninstall_user_rm_index = uninstall_user_text.find('rm -rf "$APP" "$LEGACY_APP" "$SETTINGS_APP"')
uninstall_user_kill_index = uninstall_user_text.find("killall InputiaInputMethod")
require(uninstall_user_preflight_index >= 0, "uninstall-user-missing-preflight-call")
require(uninstall_user_disable_index > uninstall_user_preflight_index, "uninstall-user-disables-before-verification-preflight")
require(uninstall_user_rm_index > uninstall_user_preflight_index, "uninstall-user-removes-before-verification-preflight")
require(uninstall_user_kill_index > uninstall_user_preflight_index, "uninstall-user-kills-before-verification-preflight")

gui_readiness_text = (root / "gui-smoke-readiness.sh").read_text()
require("signature-rejected" in gui_readiness_text, "gui-readiness-missing-signature-rejected-block")
require("tis.blockReason=" in gui_readiness_text, "gui-readiness-missing-tis-block-reason-output")
require("INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK" in gui_readiness_text, "gui-readiness-missing-self-check")
require("guiSmokeReadinessSelfCheck=true" in gui_readiness_text, "gui-readiness-missing-self-check-success")
require("readiness_reason()" in gui_readiness_text, "gui-readiness-missing-reason-function")
require("pkg-not-ready" in gui_readiness_text, "gui-readiness-missing-pkg-block")
require("admin-required" in gui_readiness_text, "gui-readiness-missing-admin-block")
require("target-cdhash-mismatch" in gui_readiness_text, "gui-readiness-missing-target-block")
require("app-missing" in gui_readiness_text, "gui-readiness-missing-app-missing-block")
require("settings-version-mismatch" in gui_readiness_text, "gui-readiness-missing-settings-block")
require("tis-not-ready" in gui_readiness_text, "gui-readiness-missing-tis-block")
require("user-host-conflict" in gui_readiness_text, "gui-readiness-missing-user-host-conflict-block")
require("INPUTIA_USER_APP_FOR_TEST" in gui_readiness_text, "gui-readiness-missing-user-app-test-override")
require("INPUTIA_USER_SETTINGS_APP_FOR_TEST" in gui_readiness_text, "gui-readiness-missing-user-settings-test-override")
require("inputia-host-running" in gui_readiness_text, "gui-readiness-missing-inputia-host-block")
require("userHostConflict=" in gui_readiness_text, "gui-readiness-missing-user-host-conflict-output")
require("inputiaHostPreflight=" in gui_readiness_text, "gui-readiness-missing-inputia-host-preflight-output")
require("guiSmokeReadinessReady=false reason=" in gui_readiness_text, "gui-readiness-missing-ready-output")
require("readiness_block_reasons()" in gui_readiness_text, "gui-readiness-missing-block-reasons-function")
require("guiSmokeReadinessBlockReasons=" in gui_readiness_text, "gui-readiness-missing-block-reasons-output")
require("guiSmokeReadinessSelfCheck blockReasons=" in gui_readiness_text, "gui-readiness-missing-block-reasons-self-check")
require("guiSmokeReadinessSelfCheck allBlockReasons=" in gui_readiness_text, "gui-readiness-missing-all-block-reasons-self-check")
require("duplicate=admin-required" in gui_readiness_text, "gui-readiness-missing-block-reason-dedupe-check")
require("frontmost_app_name()" in gui_readiness_text, "gui-readiness-missing-frontmost-timeout-helper")
require("timeout=2" in gui_readiness_text, "gui-readiness-frontmost-missing-timeout")
require('frontmost_app="$(frontmost_app_name)"' in gui_readiness_text, "gui-readiness-frontmost-does-not-use-timeout-helper")

status_text = (root / "status.sh").read_text()
require("statusTargetExists=" in status_text, "status-missing-target-exists-summary")
require("statusSignatureAccepted=" in status_text, "status-missing-signature-accepted-summary")
require("statusSigningRequiredAction=sign-with-accepted-identity" in status_text, "status-missing-signature-required-action")
require("statusUserDirectoryRequiredAction=repair-current-user-directory-service" in status_text, "status-missing-user-directory-required-action")
require("statusEnvironmentRequiredAction=repair-current-user-directory-service" in status_text, "status-missing-environment-required-action")
require("signature-rejected" in status_text, "status-missing-signature-rejected-block")
require("app-missing" in status_text, "status-missing-app-missing-block")
require("user-host-conflict" in status_text, "status-missing-user-host-conflict-block")
require("INPUTIA_USER_APP_FOR_TEST" in status_text, "status-missing-user-app-test-override")
require("INPUTIA_SYSTEM_APP_FOR_TEST" in status_text, "status-missing-system-app-test-override")
require("INPUTIA_USER_SETTINGS_APP_FOR_TEST" in status_text, "status-missing-user-settings-test-override")
require("statusUserHostConflict=" in status_text, "status-missing-user-host-conflict-summary")

gui_suite_text = (root / "gui-smoke-suite.sh").read_text()
require("guiSmokeSuiteReadiness: " in gui_suite_text, "gui-suite-missing-readiness-prefix")
require("guiSmokeSuiteReady=false reason=" in gui_suite_text, "gui-suite-missing-block-output")
require("guiSmokeSuiteBlockReasons=" in gui_suite_text, "gui-suite-missing-block-reasons-output")
require("guiSmokeReadinessBlockReasons=" in gui_suite_text, "gui-suite-missing-readiness-block-reasons-parser")
require("readiness-inconsistent" in gui_suite_text, "gui-suite-missing-inconsistent-readiness-gate")
require('if [[ "$reason" != "none" || "$block_reasons" != "none" ]]' in gui_suite_text, "gui-suite-does-not-require-none-reason-and-block-reasons")
require("guiSmokeSuiteWouldRun=false" in gui_suite_text, "gui-suite-missing-would-not-run-output")
require("guiSmokeSuiteWouldRun=true" in gui_suite_text, "gui-suite-missing-would-run-output")
require("run_post_install_regression()" in gui_suite_text, "gui-suite-missing-post-install-wrapper")
require("INPUTIA_RUN_UI_SMOKE=1" in gui_suite_text, "gui-suite-missing-post-install-ui-smoke-delegation")
require('"$POST_INSTALL_REGRESSION" "$APP"' in gui_suite_text, "gui-suite-missing-post-install-ui-smoke-delegation")
require("INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK" in gui_suite_text, "gui-suite-missing-self-check")
require("guiSmokeSuiteSelfCheck=true" in gui_suite_text, "gui-suite-missing-self-check-success")
gui_suite_run_index = gui_suite_text.find("run_gui_smoke_suite()")
gui_suite_self_check_case_index = gui_suite_text.find("run_gui_smoke_suite_self_check_case()")
gui_suite_run_block = gui_suite_text[gui_suite_run_index:gui_suite_self_check_case_index]
gui_suite_post_install_wrapper_index = gui_suite_text.find("run_post_install_regression()")
gui_suite_post_install_wrapper_end_index = gui_suite_text.find("run_gui_smoke_suite()", gui_suite_post_install_wrapper_index)
gui_suite_post_install_wrapper_block = gui_suite_text[gui_suite_post_install_wrapper_index:gui_suite_post_install_wrapper_end_index]
gui_suite_readiness_index = gui_suite_run_block.find("readiness_output")
gui_suite_ready_gate_index = gui_suite_run_block.find("guiSmokeSuiteReady=true")
gui_suite_post_install_index = gui_suite_run_block.find("run_post_install_regression")
gui_suite_post_install_env_index = gui_suite_post_install_wrapper_block.find("INPUTIA_RUN_UI_SMOKE=1")
gui_suite_post_install_target_index = gui_suite_post_install_wrapper_block.find('"$POST_INSTALL_REGRESSION" "$APP"', gui_suite_post_install_env_index)
require(
    gui_suite_run_index >= 0
    and gui_suite_self_check_case_index > gui_suite_run_index
    and gui_suite_readiness_index >= 0
    and gui_suite_ready_gate_index > gui_suite_readiness_index
    and gui_suite_post_install_index > gui_suite_ready_gate_index,
    "gui-suite-runs-post-install-before-readiness-gate",
)
require(
    gui_suite_post_install_wrapper_index >= 0
    and gui_suite_post_install_wrapper_end_index > gui_suite_post_install_wrapper_index
    and gui_suite_post_install_env_index >= 0
    and gui_suite_post_install_target_index > gui_suite_post_install_env_index,
    "gui-suite-post-install-target-missing",
)

await_system_text = (root / "await-system-install.sh").read_text()
require("app_signature_accepted()" in await_system_text, "await-system-missing-signature-assessment-helper")
require("tis.appExists=" in await_system_text, "await-system-missing-app-exists-tis-output")
require("tis.appSignatureAccepted=" in await_system_text, "await-system-missing-signature-accepted-tis-output")
require("app-missing" in await_system_text, "await-system-missing-app-missing-block")
require("signature-rejected" in await_system_text, "await-system-missing-signature-rejected-block")
require("systemInstallTargetMatchesBuild=" in await_system_text, "await-system-missing-target-match-summary")
require("systemInstallTISReady=false reason=target-cdhash-mismatch" in await_system_text, "await-system-missing-timeout-cdhash-reason")
require("systemInstallTISReady=false reason=$last_tis_block_reason" in await_system_text, "await-system-missing-timeout-tis-reason")
require("uiSmokeBlockReasons=" in await_system_text, "await-system-missing-ui-block-reasons-output")
require("append_block_reason()" in await_system_text, "await-system-missing-ui-block-reason-dedupe-helper")
target_gate_index = await_system_text.find('append_block_reason "$block_reasons" "target-cdhash-mismatch"')
tis_gate_index = await_system_text.find('append_block_reason "$block_reasons" "$tis_block_reason"')
user_host_gate_index = await_system_text.find('append_block_reason "$block_reasons" "user-host-conflict"')
target_echo_index = await_system_text.find('uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=$block_reasons')
tis_echo_index = await_system_text.find('uiSmokeBlockReason=$tis_block_reason uiSmokeBlockReasons=$block_reasons')
user_host_echo_index = await_system_text.find('uiSmokeBlockReason=user-host-conflict uiSmokeBlockReasons=$block_reasons')
gui_session_function_index = await_system_text.find('gui_session_block_reason()')
gui_session_call_index = await_system_text.find('gui_block_reason="$(gui_session_block_reason)"')
process_preflight_index = await_system_text.find('textedit_state="$(process_preflight TextEdit)"')
actual_cdhash_before_ui_index = await_system_text.find('actual_cdhash="$(cdhash "$APP" || true)"')
ui_status_call_index = await_system_text.find('ui_smoke_status_line "$last_target_matches_build" "$tis_line"')
require(target_gate_index >= 0, "await-system-missing-ui-target-mismatch-gate")
require(tis_gate_index > target_gate_index, "await-system-missing-ui-tis-specific-gate")
require(user_host_gate_index > tis_gate_index, "await-system-missing-ui-user-host-conflict-gate")
require(target_echo_index > user_host_gate_index, "await-system-target-return-does-not-use-combined-reasons")
require(tis_echo_index > target_echo_index, "await-system-tis-return-does-not-use-combined-reasons")
require(user_host_echo_index > tis_echo_index, "await-system-user-host-return-does-not-use-combined-reasons")
require(gui_session_function_index >= 0, "await-system-missing-gui-session-block-function")
require(gui_session_call_index > user_host_echo_index, "await-system-missing-gui-session-gate-after-readiness")
require(process_preflight_index > gui_session_call_index, "await-system-checks-process-before-gui-session")
require(actual_cdhash_before_ui_index >= 0, "await-system-missing-actual-cdhash-before-ui-status")
require(ui_status_call_index > actual_cdhash_before_ui_index, "await-system-ui-status-before-target-match")
for reason in ("no-console-user", "gui-bootstrap-unavailable", "login-not-complete", "screen-locked", "frontmost-unavailable", "loginwindow-frontmost"):
    require(f'echo "{reason}"' in await_system_text, f"await-system-missing-gui-session-reason-{reason}")
require("INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST" in await_system_text, "await-system-missing-gui-session-test-override")
require("INPUTIA_AWAIT_PROCESS_RUNNING_FOR_TEST" in await_system_text, "await-system-missing-process-running-test-override")
require("INPUTIA_AWAIT_IGNORE_REAL_PROCESSES_FOR_TEST" in await_system_text, "await-system-missing-ignore-real-processes-test-override")
require("uiInputiaHostPreflight=" in await_system_text, "await-system-missing-inputia-host-preflight-output")
require("inputia-host-running" in await_system_text, "await-system-missing-inputia-host-running-reason")
require("USER_LEGACY_APP=" in await_system_text, "await-system-missing-user-legacy-app-path")
require("USER_SETTINGS_APP=" in await_system_text, "await-system-missing-user-settings-app-path")
require("user_host_conflict()" in await_system_text, "await-system-missing-user-host-conflict-helper")
require("INPUTIA_AWAIT_USER_HOST_CONFLICT_FOR_TEST" in await_system_text, "await-system-missing-user-host-conflict-test-override")
require("userHostConflict=" in await_system_text, "await-system-missing-user-host-conflict-output")
require("user-host-conflict" in await_system_text, "await-system-missing-user-host-conflict-reason")
require("INPUTIA_AWAIT_UI_STATUS_SELF_CHECK" in await_system_text, "await-system-missing-ui-status-self-check")
require("awaitUiStatusSelfCheck=true" in await_system_text, "await-system-missing-ui-status-self-check-success")
require("reason=target-and-tis" in await_system_text, "await-system-missing-target-and-tis-self-check")
require("reason=target-tis-userhost" in await_system_text, "await-system-missing-target-tis-userhost-self-check")
require("reason=textedit-already-running" in await_system_text, "await-system-missing-textedit-running-self-check")
require("reason=safari-already-running" in await_system_text, "await-system-missing-safari-running-self-check")
require("reason=inputia-host-running" in await_system_text, "await-system-missing-inputia-host-running-self-check")
require("reason=user-host-conflict" in await_system_text, "await-system-missing-user-host-conflict-self-check")
require("reason=textedit-allow" in await_system_text, "await-system-missing-textedit-allow-self-check")
require("reason=safari-allow" in await_system_text, "await-system-missing-safari-allow-self-check")

verify_pkg_text = (root / "verify-pkg.sh").read_text()
require("INPUTIA_POSTINSTALL_SELF_CHECK" in verify_pkg_text, "verify-pkg-missing-postinstall-self-check-contract")
require("postinstallSelfCheck:" in verify_pkg_text, "verify-pkg-missing-postinstall-self-check-output-prefix")
require("postinstall_self_check_output" in verify_pkg_text, "verify-pkg-missing-postinstall-self-check-capture")
require("postinstall-self-check-missing-case-$case_name" in verify_pkg_text, "verify-pkg-missing-postinstall-self-check-case-assert")
require("postinstall-self-check-missing-success" in verify_pkg_text, "verify-pkg-missing-postinstall-self-check-success-assert")

postinstall_text = (root / "Packaging/scripts/postinstall").read_text()
require("inputiaPostinstallSelfCheck=true" in postinstall_text, "postinstall-missing-self-check-success-output")
for case_name in ("ready", "missing", "id-mismatch", "not-enabled", "not-selectable", "icon-mismatch"):
    require(f'assert_tis_ready_case "{case_name}"' in postinstall_text, f"postinstall-missing-self-check-case-{case_name}")

smoke_preflight_text = (root / "smoke-preflight.sh").read_text()
require("inputia_require_process_not_running" in smoke_preflight_text, "smoke-preflight-missing-inputia-host-preflight-helper")
require('"InputiaInputMethod" "smokePreflightReady" 9' in smoke_preflight_text, "smoke-preflight-missing-inputia-host-preflight-args")
require('"inputia-host-running" "-"' in smoke_preflight_text, "smoke-preflight-missing-inputia-host-ready-block")
require("INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=0 inputia_require_textedit_idle" in smoke_preflight_text, "smoke-preflight-textedit-preflight-allows-existing")
require("INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=0 inputia_require_safari_idle" in smoke_preflight_text, "smoke-preflight-safari-preflight-allows-existing")
require('"$ROOT_DIR/tis-readiness.sh" "$APP"' in smoke_preflight_text, "smoke-preflight-must-delegate-tis-readiness")
require("tisReadiness=true" in smoke_preflight_text, "smoke-preflight-missing-tis-readiness-result-check")
require("smokePreflightReady=false reason=$smoke_reason" in smoke_preflight_text, "smoke-preflight-missing-specific-tis-reason")
smoke_preflight_cdhash_index = smoke_preflight_text.find('expected_cdhash="$(cdhash "$BUILD_APP")"')
smoke_preflight_textedit_index = smoke_preflight_text.find("inputia_require_textedit_idle")
smoke_preflight_safari_index = smoke_preflight_text.find("inputia_require_safari_idle")
smoke_preflight_inputia_index = smoke_preflight_text.find("inputia_require_process_not_running")
require(smoke_preflight_textedit_index >= 0 and smoke_preflight_textedit_index < smoke_preflight_cdhash_index, "smoke-preflight-textedit-after-cdhash")
require(smoke_preflight_safari_index >= 0 and smoke_preflight_safari_index < smoke_preflight_cdhash_index, "smoke-preflight-safari-after-cdhash")
require(smoke_preflight_inputia_index >= 0 and smoke_preflight_inputia_index < smoke_preflight_cdhash_index, "smoke-preflight-inputia-after-cdhash")

status_text = (root / "status.sh").read_text()
require("section \"gui smoke summary\"" in status_text, "status-missing-gui-smoke-summary-section")
require("statusGuiSmokeBlockReasons=" in status_text, "status-missing-gui-smoke-block-reasons-output")
require("statusGuiSmokeReady=false reason=" in status_text, "status-missing-gui-smoke-ready-output")
require("statusAdminInstallReady=" in status_text, "status-missing-admin-ready-output")
require("statusTISEnabledMatches=" in status_text, "status-missing-tis-enabled-summary")
require("statusGuiSessionBlockReason=" in status_text, "status-missing-gui-session-summary")
require("statusTextEditPreflight=" in status_text, "status-missing-textedit-preflight-summary")
require("statusSafariPreflight=" in status_text, "status-missing-safari-preflight-summary")
require("statusInputiaHostPreflight=" in status_text, "status-missing-inputia-host-preflight-summary")
require("inputia-host-running" in status_text, "status-missing-inputia-host-running-reason")
require("gui_session_block_reason()" in status_text, "status-missing-gui-session-helper")
require("frontmost_app_name()" in status_text, "status-missing-frontmost-timeout-helper")
require("timeout=2" in status_text, "status-frontmost-missing-timeout")
require('frontmost_app="$(frontmost_app_name)"' in status_text, "status-frontmost-does-not-use-timeout-helper")
require("textedit-already-running" in status_text, "status-missing-textedit-block-reason")
require("safari-already-running" in status_text, "status-missing-safari-block-reason")
require("target-cdhash-mismatch" in status_text, "status-missing-target-block-reason")
require("settings-version-mismatch" in status_text, "status-missing-settings-block-reason")
require("admin-required" in status_text, "status-missing-admin-block-reason")
require("tis-not-ready" in status_text, "status-missing-tis-block-reason")
require("user-directory-unavailable" in status_text, "status-missing-user-directory-block-reason")

post_install_regression_text = (root / "post-install-regression.sh").read_text()
require('"TextEdit" \\\n    "0" \\\n    "textedit-already-running"' in post_install_regression_text, "post-install-textedit-preflight-allows-existing")
require('"Safari" \\\n    "0" \\\n    "safari-already-running"' in post_install_regression_text, "post-install-safari-preflight-allows-existing")
require('"$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$APP"' in post_install_regression_text, "post-install-regression-missing-command-shortcut-smoke")
require('"$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$APP"' in post_install_regression_text, "post-install-regression-missing-safari-command-shortcut-smoke")
require("INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK" in post_install_regression_text, "post-install-missing-ui-preflight-self-check")
require("INPUTIA_UI_PROCESS_RUNNING_FOR_TEST" in post_install_regression_text, "post-install-missing-ui-process-test-override")
require("INPUTIA_UI_PROCESS_IGNORE_REAL_FOR_TEST" in post_install_regression_text, "post-install-missing-ui-process-ignore-real-test-override")
require('"InputiaInputMethod"' in post_install_regression_text, "post-install-missing-inputia-host-preflight")
require("inputia-host-running" in post_install_regression_text, "post-install-missing-inputia-host-running-reason")
require("postInstallUiPreflightSelfCheck=true" in post_install_regression_text, "post-install-missing-ui-preflight-self-check-success")
require('"$ROOT_DIR/tis-readiness.sh" "$APP"' in post_install_regression_text, "post-install-must-delegate-tis-readiness")
require("postInstallTISBlockReason=" in post_install_regression_text, "post-install-missing-tis-block-reason-output")
require("postInstallUiSmokeReady=false reason=signature-rejected" in post_install_regression_text, "post-install-missing-signature-rejected-ui-gate")
require("USER_SETTINGS_APP=" in post_install_regression_text, "post-install-missing-user-settings-app-path")
require("INPUTIA_USER_SETTINGS_APP" in post_install_regression_text, "post-install-missing-user-settings-env-override")
require("settingsPath=$USER_SETTINGS_APP" in post_install_regression_text, "post-install-missing-user-settings-conflict-output")
post_install_tis_not_ready_index = post_install_regression_text.find('postInstallUiSmokeReady=false reason=tis-not-ready')
post_install_ui_preflight_index = post_install_regression_text.find('section "UI smoke preflight"')
post_install_textedit_index = post_install_regression_text.find('"$ROOT_DIR/smoke-textedit.sh" "$APP"')
post_install_textedit_command_index = post_install_regression_text.find('"$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$APP"')
post_install_safari_diagnose_index = post_install_regression_text.find('"$ROOT_DIR/diagnose-safari-input-source.sh" "$APP"')
post_install_safari_typing_index = post_install_regression_text.find('"$ROOT_DIR/smoke-safari-typing.sh" "$APP"')
post_install_safari_command_index = post_install_regression_text.find('"$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$APP"')
post_install_safari_enter_index = post_install_regression_text.find('"$ROOT_DIR/smoke-safari-enter.sh" "$APP"')
post_install_clipboard_index = post_install_regression_text.find('"$ROOT_DIR/smoke-clipboard-recall.sh" "$APP"')
post_install_result_index = post_install_regression_text.find('section "result"')
post_install_passed_index = post_install_regression_text.find('postInstallRegressionPassed=true', post_install_result_index)
require(post_install_tis_not_ready_index >= 0, "post-install-missing-tis-not-ready-gate")
require(post_install_ui_preflight_index > post_install_tis_not_ready_index, "post-install-ui-preflight-before-tis-gate")
require(
    post_install_ui_preflight_index
    < post_install_textedit_index
    < post_install_textedit_command_index
    < post_install_safari_diagnose_index
    < post_install_safari_typing_index
    < post_install_safari_command_index
    < post_install_safari_enter_index
    < post_install_clipboard_index,
    "post-install-ui-smoke-order-drift",
)
require(
    post_install_result_index > post_install_clipboard_index
    and post_install_passed_index > post_install_result_index,
    "post-install-success-marker-before-ui-smoke-complete",
)
require('LOCK_DIR="${INPUTIA_POST_INSTALL_LOCK_DIR:-/tmp/inputia-post-install-regression.lock}"' in post_install_regression_text, "post-install-lock-not-fixed-to-tmp")
require("postInstallLockAcquired=true" in post_install_regression_text, "post-install-missing-lock-acquired-output")
require("postInstallLockStale=true" in post_install_regression_text, "post-install-missing-stale-lock-recovery")
require("postInstallRegressionReady=false reason=already-running" in post_install_regression_text, "post-install-missing-live-lock-rejection")
require("LOCK_HELD=0" in post_install_regression_text, "post-install-lock-release-does-not-reset-state")

verify_nongui_text = (root / "verify-nongui.sh").read_text()
require("current-input-source-unavailable" in verify_nongui_text, "verify-nongui-current-source-unknown-not-fatal")
require('$before" == "unknown"' in verify_nongui_text, "verify-nongui-before-unknown-not-rejected")
require('$after" == "unknown"' in verify_nongui_text, "verify-nongui-after-unknown-not-rejected")
require("INPUTIA_ALLOW_APPLESCRIPT_COMPILE_APP_LAUNCH" in verify_nongui_text, "verify-nongui-missing-applescript-compile-launch-gate")
require("appleScriptCompileSkipped=true reason=osacompile-may-launch-target-apps" in verify_nongui_text, "verify-nongui-missing-applescript-unsafe-skip")
require('VERIFY_LOCK_DIR="/tmp/inputia-verify-nongui.lock"' in verify_nongui_text, "verify-nongui-lock-not-fixed-to-tmp")
require('TMP_RESIDUE_ROOT="/private/tmp"' in verify_nongui_text, "verify-nongui-tmp-residue-root-not-private-tmp")
require("VERIFY_TEMP_FILES=()" in verify_nongui_text, "verify-nongui-missing-temp-file-registry")
require("VERIFY_TEMP_DIRS=()" in verify_nongui_text, "verify-nongui-missing-temp-dir-registry")
require("cleanup_verify_temp_files" in verify_nongui_text, "verify-nongui-missing-temp-file-cleanup")
require("cleanup_verify_temp_dirs" in verify_nongui_text, "verify-nongui-missing-temp-dir-cleanup")
require("assert_user_host_baseline_absent" in verify_nongui_text, "verify-nongui-missing-user-host-baseline-gate")
require("user-host-baseline-present" in verify_nongui_text, "verify-nongui-missing-user-host-baseline-present-reason")
require("INPUTIA_VERIFY_ALLOW_USER_HOST_BASELINE" in verify_nongui_text, "verify-nongui-missing-user-host-baseline-override")
require("verifyUserHostBaselineAbsent=true" in verify_nongui_text, "verify-nongui-missing-user-host-baseline-absent-output")
require('${VERIFY_TEMP_FILES[@]+"${VERIFY_TEMP_FILES[@]}"}' in verify_nongui_text, "verify-nongui-temp-cleanup-not-set-u-safe")
require('${VERIFY_TEMP_DIRS[@]+"${VERIFY_TEMP_DIRS[@]}"}' in verify_nongui_text, "verify-nongui-temp-dir-cleanup-not-set-u-safe")
require('VERIFY_TEMP_FILES+=("$script_file" "$compiled_file")' in verify_nongui_text, "verify-nongui-missing-applescript-temp-registration")
require('VERIFY_TEMP_FILES+=("$provided_event_log" "$generated_event_log")' in verify_nongui_text, "verify-nongui-missing-debug-event-temp-registration")
require('VERIFY_TEMP_DIRS+=("$post_install_active_lock_dir")' in verify_nongui_text, "verify-nongui-missing-active-lock-temp-dir-registration")
require("tempDirCleanupSelfCheck=true" in verify_nongui_text, "verify-nongui-missing-temp-dir-cleanup-self-check")
require("verifyLockOwnerPid=$existing_pid" in verify_nongui_text, "verify-nongui-missing-live-lock-owner-output")
require("verifyLockStale=true" in verify_nongui_text, "verify-nongui-missing-stale-lock-recovery")
require("verify-lock-acquire-failed" in verify_nongui_text, "verify-nongui-missing-lock-retry-failure-output")
require("guiSmokeReadinessInputiaHostGateSelfCheck=true" in verify_nongui_text, "verify-nongui-missing-readiness-inputia-host-gate")
require("gui-readiness-inputia-host-gate-missing-preflight" in verify_nongui_text, "verify-nongui-missing-readiness-inputia-host-preflight-assert")
require("gui-readiness-inputia-host-gate-missing-blocker" in verify_nongui_text, "verify-nongui-missing-readiness-inputia-host-blocker-assert")
require('section "GUI smoke suite current blocked gate"' in verify_nongui_text, "verify-nongui-missing-gui-suite-current-blocked-gate")
require("guiSmokeSuiteCurrentBlockedGateNoMutationPassed=true" in verify_nongui_text, "verify-nongui-missing-gui-suite-current-blocked-no-mutation-marker")
require("gui-suite-current-blocked-gate-missing-would-not-run" in verify_nongui_text, "verify-nongui-missing-gui-suite-current-blocked-would-not-run-assert")
require('INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=0' in gui_suite_text, "gui-suite-does-not-force-textedit-preflight")
require('INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=0' in gui_suite_text, "gui-suite-does-not-force-safari-preflight")
shortcut_self_check_text = (root / "Tools/InputiaShortcutSelfCheck.swift").read_text()
main_text = (root / "Sources/InputiaInputMethod/main.swift").read_text()
host_policy_text = (root / "Sources/InputiaInputMethod/InputiaHostTextPolicy.swift").read_text()
host_policy_self_check_text = (root / "Tools/InputiaHostTextPolicySelfCheck.swift").read_text()
require('recallClipboardMenuKeyEquivalent = ""' in host_policy_text, "host-policy-recall-clipboard-menu-key-equivalent-not-empty")
require("recallClipboardMenuHasNoCommandKeyEquivalent" in host_policy_self_check_text, "host-policy-missing-recall-clipboard-menu-self-check")
require('NSMenuItem(title: "召回剪贴板", action: #selector(recallClipboard), keyEquivalent: "v")' not in main_text, "host-menu-recall-clipboard-steals-command-v")
require("keyEquivalent: InputiaHostTextPolicy.recallClipboardMenuKeyEquivalent" in main_text, "host-menu-recall-clipboard-key-equivalent-not-policy-owned")
require("expectedTISIconPath" in main_text, "host-tis-missing-expected-icon-path")
require('urlProperty(source, key: kTISPropertyIconImageURL) == expectedTISIconPath' in main_text, "host-tis-source-selection-not-icon-filtered")
require("matchingSources.first" in main_text, "host-tis-source-selection-missing-fallback")
require('"--host-shortcut-self-check", "--shortcut-self-check"' in main_text, "host-shortcut-self-check-missing-short-alias")
normalize_index = main_text.find("private func normalizeHIToolbox()")
clear_index = main_text.find("private func clearInputSourcePreferences()")
normalize_text = main_text[normalize_index:clear_index]
require("AppleEnabledThirdPartyInputSources" in normalize_text, "host-normalize-missing-third-party-input-sources")
require("inputSourcesDomain" in normalize_text, "host-normalize-third-party-missing-inputsources-domain")
require("thirdPartyEnabledAfter=" in normalize_text, "host-normalize-missing-third-party-count-output")
require("hitoolboxNormalizeSkipped=true reason=manual-hitoolbox-write-disabled" in normalize_text, "host-normalize-missing-manual-write-disabled-output")
require("hitoolboxNormalizeRequiredAction=enable-via-system-settings-or-fix-user-preference-service" in normalize_text, "host-normalize-missing-required-action-output")
require("setPreferenceArray(" not in normalize_text, "host-normalize-still-writes-preferences")
require("writeHIToolboxPreferencePlist(" not in normalize_text, "host-normalize-still-writes-hitoolbox-plist")
clear_text = main_text[clear_index:main_text.find("private func dumpInputSource", clear_index)]
require("inputSourcePreferencesClearSkipped=true reason=manual-hitoolbox-write-disabled" in clear_text, "host-clear-preferences-missing-manual-write-disabled-output")
require("CFPreferencesSet" not in main_text, "host-still-has-cfpreferences-write")
require("writeHIToolboxPreferencePlist(" not in main_text, "host-still-has-hitoolbox-plist-writer")
install_user_text = (root / "install-user.sh").read_text()
install_system_text = (root / "install-system.sh").read_text()
postinstall_text = (root / "Packaging/scripts/postinstall").read_text()
for forbidden in ["--enable-input-source", "--normalize-hitoolbox", "--select-input-source"]:
    require(forbidden not in install_user_text, f"install-user-still-calls-{forbidden}")
    require(forbidden not in install_system_text, f"install-system-still-calls-{forbidden}")
    require(forbidden not in postinstall_text, f"postinstall-still-calls-{forbidden}")
require("--clear-input-source-preferences" not in install_user_text, "install-user-still-clears-input-source-preferences")
require("--clear-input-source-preferences" not in install_system_text, "install-system-still-clears-input-source-preferences")
require("--clear-input-source-preferences" not in postinstall_text, "postinstall-still-clears-input-source-preferences")
require("userInstallRequiredAction=add-input-source-in-system-settings" in install_user_text, "install-user-missing-manual-add-required-action")
require("systemInstallRequiredAction=add-input-source-in-system-settings" in install_system_text, "install-system-missing-manual-add-required-action")
require("--register-input-source" in install_user_text, "install-user-missing-tis-register")
require("--register-input-source" in install_system_text, "install-system-missing-tis-register")
require("--register-input-source" in postinstall_text, "postinstall-missing-tis-register")
require("inputiaPostinstallRequiredAction=add-input-source-in-system-settings" in postinstall_text, "postinstall-missing-manual-add-required-action")
status_text = (root / "status.sh").read_text()
tis_readiness_text = (root / "tis-readiness.sh").read_text()
require("statusLegacyHIToolboxInputiaEnabled=" in status_text, "status-missing-legacy-hitoolbox-enabled-diagnostic")
require("statusStaleHIToolboxEnabledStateSuspected=" in status_text, "status-missing-stale-hitoolbox-diagnostic")
require("tis.legacyHIToolboxInputiaEnabled=" in tis_readiness_text, "tis-readiness-missing-legacy-hitoolbox-enabled-diagnostic")
require("tis.staleHIToolboxEnabledStateSuspected=" in tis_readiness_text, "tis-readiness-missing-stale-hitoolbox-diagnostic")
handle_key_down_index = main_text.find("private func handleKeyDown(_ event: NSEvent, client: IMKTextInput) -> Bool")
pass_through_index = main_text.find("InputiaShortcutClassifier.shouldPassThroughKeyDown", handle_key_down_index)
script_toggle_index = main_text.find("isScriptToggleShortcut(event, modifiers: modifiers)", handle_key_down_index)
clipboard_recall_index = main_text.find("isClipboardRecallShortcut(event, modifiers: modifiers)", handle_key_down_index)
punctuation_toggle_index = main_text.find("isPunctuationToggleShortcut(event, modifiers: modifiers)", handle_key_down_index)
require(handle_key_down_index >= 0, "host-keydown-handler-missing")
require(pass_through_index > handle_key_down_index, "host-keydown-command-pass-through-missing")
require(script_toggle_index > pass_through_index, "host-keydown-command-pass-through-after-script-toggle")
require(clipboard_recall_index > pass_through_index, "host-keydown-command-pass-through-after-clipboard-recall")
require(punctuation_toggle_index > pass_through_index, "host-keydown-command-pass-through-after-punctuation-toggle")
require("return false" in main_text[pass_through_index:script_toggle_index], "host-keydown-command-pass-through-does-not-return-false")
for label, text in (
    ("shortcut-tool", shortcut_self_check_text),
    ("host-shortcut", main_text),
):
    require("shouldPassThroughKeyDown" in text, f"{label}-missing-keydown-pass-through-check")
    require("officialAppleCommandKeyDownSetPassesThrough" in text, f"{label}-missing-official-apple-command-keydown-set")
    require("appleCommandCKeyDownPassThrough" in text, f"{label}-missing-apple-command-c-keydown")
    require("appleCommandVKeyDownPassThrough" in text, f"{label}-missing-apple-command-v-keydown")
    require("appleCommandXKeyDownPassThrough" in text, f"{label}-missing-apple-command-x-keydown")
    require("appleCommandZKeyDownPassThrough" in text, f"{label}-missing-apple-command-z-keydown")
    require("appleCommandAKeyDownPassThrough" in text, f"{label}-missing-apple-command-a-keydown")
    require("appleCommandFKeyDownPassThrough" in text, f"{label}-missing-apple-command-f-keydown")
    require("appleCommandSKeyDownPassThrough" in text, f"{label}-missing-apple-command-s-keydown")
    require("appleCommandPKeyDownPassThrough" in text, f"{label}-missing-apple-command-p-keydown")
    require("appleCommandQKeyDownPassThrough" in text, f"{label}-missing-apple-command-q-keydown")
    require("appleCommandTabKeyDownPassThrough" in text, f"{label}-missing-apple-command-tab-keydown")
    require("appleCommandSpaceKeyDownPassThrough" in text, f"{label}-missing-apple-command-space-keydown")
    require("appleCommandOptionEscapeKeyDownPassThrough" in text, f"{label}-missing-apple-command-option-escape-keydown")
    require("commandModifierVariants" in text, f"{label}-missing-command-modifier-variants")
    require("[.command, .control, .option]" in text, f"{label}-missing-command-control-option-variant")
    require("[.command, .control, .option, .shift]" in text, f"{label}-missing-command-all-modifier-variant")
    require("allCommandModifierVariantsPassThrough" in text, f"{label}-missing-all-command-modifier-marker")
    require("ctrlShiftSScriptToggle" in text, f"{label}-missing-script-toggle-marker")
    require("ctrlShiftCommandSScriptToggleRejected" in text, f"{label}-missing-command-script-toggle-rejection-marker")
    require("scriptToggleRejectedWhenDisabled" in text, f"{label}-missing-disabled-script-toggle-marker")
for label, process, reason in (
    ("post-install-non-gui", "TextEdit", "post-install-non-gui-launched-textedit"),
    ("post-install-non-gui", "Safari", "post-install-non-gui-launched-safari"),
    ("post-install-ui-tis", "TextEdit", "post-install-ui-tis-gate-launched-textedit"),
    ("post-install-ui-tis", "Safari", "post-install-ui-tis-gate-launched-safari"),
    ("await-ui-not-ready", "TextEdit", "await-ui-not-ready-launched-textedit"),
    ("await-ui-not-ready", "Safari", "await-ui-not-ready-launched-safari"),
):
    assert_marker = f'assert_process_not_running {process} "{reason}"'
    reason_index = verify_nongui_text.find(assert_marker)
    if process == "TextEdit":
        guard_index = verify_nongui_text.rfind('if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]', 0, reason_index)
    else:
        guard_index = verify_nongui_text.rfind('if [[ "$SAFARI_PREEXISTING" == "false" ]]', 0, reason_index)
    require(reason_index >= 0, f"verify-nongui-missing-{label}-{process.lower()}-assert")
    require(guard_index >= 0, f"verify-nongui-unguarded-{label}-{process.lower()}-assert")
collect_residue_index = verify_nongui_text.rfind("\ncollect_residue()")
collector_self_check_section_index = verify_nongui_text.find('section "residue collector self-check"', collect_residue_index)
residue_section_index = verify_nongui_text.find('\nsection "residue"\n', collector_self_check_section_index)
tmp_residue_index = verify_nongui_text.find("tmp_residue=", residue_section_index)
lock_exclude_index = verify_nongui_text.find('! -path "$VERIFY_LOCK_DIR"', tmp_residue_index)
real_lock_exclude_index = verify_nongui_text.find('! -path "$VERIFY_LOCK_REAL_DIR"', tmp_residue_index)
collect_residue_block = verify_nongui_text[collect_residue_index:residue_section_index]
require(residue_section_index >= 0, "verify-nongui-missing-residue-section")
require(collect_residue_index >= 0, "verify-nongui-missing-collect-residue-function")
require(collector_self_check_section_index > collect_residue_index, "verify-nongui-missing-residue-collector-self-check-section")
require("release_verify_lock" not in verify_nongui_text[residue_section_index:tmp_residue_index], "verify-nongui-releases-lock-before-residue-complete")
require(lock_exclude_index > tmp_residue_index, "verify-nongui-tmp-scan-does-not-exclude-own-lock")
require(real_lock_exclude_index > lock_exclude_index, "verify-nongui-tmp-scan-does-not-exclude-own-real-lock")
require("$2 ~ /(^|\\/)(bash|zsh|sh|env)$/" in collect_residue_block, "verify-nongui-residue-does-not-require-shell-wrapper")
require("/bin\\/(bash|zsh) -c" not in collect_residue_block, "verify-nongui-residue-skips-all-shell-c-wrappers")
require("residueCollectorSelfCheck=true" in verify_nongui_text, "verify-nongui-missing-residue-collector-self-check")
for tmp_pattern in (
    "-select.*.log",
    "-restore.*.log",
    "-test.*.url",
    "-osascript.*.applescript",
    "inputia-hitoolbox-preference.*.txt",
    "inputia-verify-nongui.lock",
    "inputia-launchservices-*.log",
    "inputia-install-user.*",
    "inputia-debug-event-*",
):
    require(tmp_pattern in verify_nongui_text, f"verify-nongui-missing-tmp-residue-pattern-{tmp_pattern}")

print("cleanupPermissionContract=true")
PY
}

section "syntax"
echo "textEditPreExisting=$TEXTEDIT_PREEXISTING"
echo "safariPreExisting=$SAFARI_PREEXISTING"
bash -n \
  "$ROOT_DIR/smoke-common.sh" \
	  "$ROOT_DIR/smoke-preflight.sh" \
	  "$ROOT_DIR/smoke-textedit.sh" \
	  "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" \
	  "$ROOT_DIR/smoke-clipboard-recall.sh" \
  "$ROOT_DIR/smoke-safari-typing.sh" \
  "$ROOT_DIR/smoke-safari-enter.sh" \
  "$ROOT_DIR/diagnose-safari-input-source.sh" \
  "$ROOT_DIR/tis-readiness.sh"
zsh -n \
  "$ROOT_DIR/verify-pkg.sh" \
  "$ROOT_DIR/status.sh" \
  "$ROOT_DIR/await-system-install.sh" \
  "$ROOT_DIR/post-install-regression.sh" \
  "$ROOT_DIR/build-pkg.sh" \
  "$ROOT_DIR/open-settings.sh" \
  "$ROOT_DIR/open-installer.sh" \
  "$ROOT_DIR/gui-smoke-readiness.sh" \
  "$ROOT_DIR/gui-smoke-suite.sh" \
	  "$ROOT_DIR/install-system.sh" \
	  "$ROOT_DIR/install-user.sh" \
	  "$ROOT_DIR/import-signing-identity.sh" \
	  "$ROOT_DIR/notarization-readiness.sh" \
	  "$ROOT_DIR/notarize-app.sh" \
	  "$ROOT_DIR/notarize-pkg.sh" \
	  "$ROOT_DIR/uninstall-system.sh" \
	  "$ROOT_DIR/uninstall-user.sh"
echo "syntaxOK=true"

section "cleanup permission contract"
verify_cleanup_permission_contract

section "signing identity import self-check"
signing_import_self_check_root="$(/usr/bin/mktemp -d "/tmp/inputia-signing-import-self-check.XXXXXX")"
VERIFY_TEMP_DIRS+=("$signing_import_self_check_root")
signing_import_fake_p12="$signing_import_self_check_root/fake.p12"
signing_import_fake_keychain="$signing_import_self_check_root/login.keychain-db"
/usr/bin/touch "$signing_import_fake_p12" "$signing_import_fake_keychain"
/bin/chmod 600 "$signing_import_fake_p12"
run_expect_rc 12 "signingImportMissingPasswordSelfCheck" \
  /usr/bin/env \
    -u INPUTIA_P12_PASSWORD \
    -u INPUTIA_SIGNING_P12_PASSWORD \
    -u INPUTIA_KEYCHAIN_PASSWORD \
    INPUTIA_CODESIGN_IDENTITY="Inputia Missing Password Self Check Identity" \
    INPUTIA_SIGNING_P12="$signing_import_fake_p12" \
    INPUTIA_SIGNING_KEYCHAIN="$signing_import_fake_keychain" \
    "$ROOT_DIR/import-signing-identity.sh"
require_output "$RUN_EXPECT_RC_OUTPUT" "signingIdentityImportReady=false reason=missing-p12-password" "signing-import-self-check-missing-password-gate"
require_output "$RUN_EXPECT_RC_OUTPUT" "signingIdentityRequiredAction=set-INPUTIA_P12_PASSWORD-or-INPUTIA_SIGNING_P12_PASSWORD" "signing-import-self-check-missing-required-action"
echo "signingIdentityImportMissingPasswordSelfCheck=true"

section "notarization app preflight"
notarize_archive_before="$(/bin/ls "$ROOT_DIR"/dist/*-notary.zip 2>/dev/null || true)"
run_allow_rc "0,12,13,14,15,16" "notarizeAppPreflightOnly" \
  /usr/bin/env \
    INPUTIA_NOTARIZE_APP_PREFLIGHT_ONLY=1 \
    "$ROOT_DIR/notarize-app.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "inputiaNotarizeAppTool=true" "notarize-app-preflight-missing-tool-marker"
if [[ "$RUN_EXPECT_RC_ACTUAL" == "0" ]]; then
  require_output "$RUN_EXPECT_RC_OUTPUT" "notarizeAppReady=true mode=preflight-only" "notarize-app-preflight-missing-preflight-only-success"
  require_output "$RUN_EXPECT_RC_OUTPUT" "notarizeAppPassed=skipped reason=preflight-only" "notarize-app-preflight-missing-skip-marker"
else
  require_output "$RUN_EXPECT_RC_OUTPUT" "notarizeAppPassed=false" "notarize-app-preflight-missing-failure-marker"
fi
notarize_archive_after="$(/bin/ls "$ROOT_DIR"/dist/*-notary.zip 2>/dev/null || true)"
if [[ "$notarize_archive_before" != "$notarize_archive_after" ]]; then
  echo "nonGuiVerificationPassed=false reason=notarize-app-preflight-created-archive"
  exit 1
fi
echo "notarizeAppPreflightNoSubmit=true"

section "notarization package preflight"
pkg_notary_submission_before="$(/bin/ls "$ROOT_DIR"/build/notary/pkg-notary-submit.plist 2>/dev/null || true)"
run_allow_rc "0,10,12,15" "notarizePkgPreflightOnly" \
  /usr/bin/env \
    INPUTIA_NOTARIZE_PKG_PREFLIGHT_ONLY=1 \
    "$ROOT_DIR/notarize-pkg.sh" "$ROOT_DIR/dist/InputiaInputMethod-latest.pkg"
require_output "$RUN_EXPECT_RC_OUTPUT" "inputiaNotarizePkgTool=true" "notarize-pkg-preflight-missing-tool-marker"
if [[ "$RUN_EXPECT_RC_ACTUAL" == "0" ]]; then
  require_output "$RUN_EXPECT_RC_OUTPUT" "notarizePkgReady=true mode=preflight-only" "notarize-pkg-preflight-missing-preflight-only-success"
  require_output "$RUN_EXPECT_RC_OUTPUT" "notarizePkgPassed=skipped reason=preflight-only" "notarize-pkg-preflight-missing-skip-marker"
else
  require_output "$RUN_EXPECT_RC_OUTPUT" "notarizePkgPassed=false" "notarize-pkg-preflight-missing-failure-marker"
fi
pkg_notary_submission_after="$(/bin/ls "$ROOT_DIR"/build/notary/pkg-notary-submit.plist 2>/dev/null || true)"
if [[ "$pkg_notary_submission_before" != "$pkg_notary_submission_after" ]]; then
  echo "nonGuiVerificationPassed=false reason=notarize-pkg-preflight-created-submission"
  exit 1
fi
echo "notarizePkgPreflightNoSubmit=true"

section "shortcut pass-through self-checks"
require_executable "$ROOT_DIR/build/inputia-shortcut-self-check" "missing-shortcut-self-check"
require_executable "$ROOT_DIR/build/inputia-host-text-policy-self-check" "missing-host-text-policy-self-check"
require_executable "$BUILD_APP/Contents/MacOS/InputiaInputMethod" "missing-build-host-executable"
shortcut_checks_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
shortcut_checks_tis_before="$(current_input_source_id)"
shortcut_checks_debug_before="$(debug_events_env)"
shortcut_output="$("$ROOT_DIR/build/inputia-shortcut-self-check" 2>&1)"
printf '%s\n' "$shortcut_output" | /usr/bin/sed 's/^/shortcut: /'
require_output "$shortcut_output" "shortcutSelfCheck=true" "shortcut-self-check-failed"
require_output "$shortcut_output" "commonAppleCommandShortcutSetPassesThrough=true" "common-apple-command-shortcuts-not-covered"
require_output "$shortcut_output" "anyCommandModifiedKeyPassesThrough=true" "any-command-modified-key-not-covered"
require_output "$shortcut_output" "allCommandModifierVariantsPassThrough=true" "all-command-modifier-variants-not-covered"
require_output "$shortcut_output" "commandCPassThrough=true" "command-c-not-covered"
require_output "$shortcut_output" "commandVPassThrough=true" "command-v-not-covered"
require_output "$shortcut_output" "commandShiftVPassThrough=true" "command-shift-v-not-covered"
require_output "$shortcut_output" "commandOptionShiftVPassThrough=true" "command-option-shift-v-not-covered"
require_output "$shortcut_output" "commandControlVPassThrough=true" "command-control-v-not-covered"
require_output "$shortcut_output" "ctrlShiftVClipboardRecall=true" "inputia-clipboard-recall-not-covered"
require_output "$shortcut_output" "ctrlShiftCommandVRejected=true" "command-clipboard-recall-conflict"
require_output "$shortcut_output" "ctrlShiftSScriptToggle=true" "script-toggle-not-covered"
require_output "$shortcut_output" "ctrlShiftCommandSScriptToggleRejected=true" "command-script-toggle-conflict"
require_output "$shortcut_output" "scriptToggleRejectedWhenDisabled=true" "disabled-script-toggle-conflict"

host_shortcut_output="$("$BUILD_APP/Contents/MacOS/InputiaInputMethod" --host-shortcut-self-check 2>&1)"
printf '%s\n' "$host_shortcut_output" | /usr/bin/sed 's/^/hostShortcut: /'
require_output "$host_shortcut_output" "hostShortcutSelfCheck=true" "host-shortcut-self-check-failed"
require_output "$host_shortcut_output" "commonAppleCommandShortcutSetPassesThrough=true" "host-common-apple-command-shortcuts-not-covered"
require_output "$host_shortcut_output" "anyCommandModifiedKeyPassesThrough=true" "host-any-command-modified-key-not-covered"
require_output "$host_shortcut_output" "allCommandModifierVariantsPassThrough=true" "host-all-command-modifier-variants-not-covered"
require_output "$host_shortcut_output" "commandCPassThrough=true" "host-command-c-not-covered"
require_output "$host_shortcut_output" "commandVPassThrough=true" "host-command-v-not-covered"
require_output "$host_shortcut_output" "ctrlShiftVClipboardRecall=true" "host-inputia-clipboard-recall-not-covered"
require_output "$host_shortcut_output" "ctrlShiftCommandVRejected=true" "host-command-clipboard-recall-conflict"
require_output "$host_shortcut_output" "ctrlShiftSScriptToggle=true" "host-script-toggle-not-covered"
require_output "$host_shortcut_output" "ctrlShiftCommandSScriptToggleRejected=true" "host-command-script-toggle-conflict"
require_output "$host_shortcut_output" "scriptToggleRejectedWhenDisabled=true" "host-disabled-script-toggle-conflict"

host_policy_output="$("$ROOT_DIR/build/inputia-host-text-policy-self-check" 2>&1)"
printf '%s\n' "$host_policy_output" | /usr/bin/sed 's/^/hostTextPolicy: /'
require_output "$host_policy_output" "hostTextPolicySelfCheck=true" "host-text-policy-self-check-failed"
require_output "$host_policy_output" "appCommandcopyPassesThrough=true" "app-command-copy-not-covered"
require_output "$host_policy_output" "appCommandpastePassesThrough=true" "app-command-paste-not-covered"
require_output "$host_policy_output" "appCommandcutPassesThrough=true" "app-command-cut-not-covered"
require_output "$host_policy_output" "appCommandundoPassesThrough=true" "app-command-undo-not-covered"
require_output "$host_policy_output" "appCommandredoPassesThrough=true" "app-command-redo-not-covered"
require_output "$host_policy_output" "appCommandselectAllPassesThrough=true" "app-command-select-all-not-covered"
require_output "$host_policy_output" "recallClipboardMenuHasNoCommandKeyEquivalent=true" "recall-clipboard-menu-key-equivalent-steals-command-v"
shortcut_checks_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
shortcut_checks_tis_after="$(current_input_source_id)"
shortcut_checks_debug_after="$(debug_events_env)"
if [[ "$shortcut_checks_clipboard_after" != "$shortcut_checks_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=shortcut-checks-mutated-clipboard"
  exit 1
fi
echo "shortcutPassThroughSelfChecks.clipboardUnchanged=true"
assert_current_source_unchanged "shortcutPassThroughSelfChecks" "$shortcut_checks_tis_before" "$shortcut_checks_tis_after"
assert_debug_env_unchanged "shortcutPassThroughSelfChecks" "$shortcut_checks_debug_before" "$shortcut_checks_debug_after"
assert_no_user_host "shortcutPassThroughSelfChecks"
echo "shortcutPassThroughSelfChecks=true"

section "input text router and script self-checks"
require_executable "$ROOT_DIR/build/inputia-input-text-router-self-check" "missing-input-text-router-self-check"
input_text_router_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
input_text_router_tis_before="$(current_input_source_id)"
input_text_router_debug_before="$(debug_events_env)"
input_text_router_output="$("$ROOT_DIR/build/inputia-input-text-router-self-check" 2>&1)"
printf '%s\n' "$input_text_router_output" | /usr/bin/sed 's/^/inputTextRouter: /'
require_output "$input_text_router_output" "inputTextRouterSelfCheck=true" "input-text-router-self-check-failed"
require_output "$input_text_router_output" "carriageReturnCommitsRaw=true" "input-text-router-return-raw-not-covered"
require_output "$input_text_router_output" "composingSpaceCommitsCandidate=true" "input-text-router-space-candidate-not-covered"
require_output "$input_text_router_output" "simplifiedScriptCommitsSimplified=true" "input-text-router-simplified-script-not-covered"
require_output "$input_text_router_output" "traditionalScriptCommitsTraditional=true" "input-text-router-traditional-script-not-covered"
input_text_router_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
input_text_router_tis_after="$(current_input_source_id)"
input_text_router_debug_after="$(debug_events_env)"
if [[ "$input_text_router_clipboard_after" != "$input_text_router_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=input-text-router-mutated-clipboard"
  exit 1
fi
echo "inputTextRouterSelfChecks.clipboardUnchanged=true"
assert_current_source_unchanged "inputTextRouterSelfChecks" "$input_text_router_tis_before" "$input_text_router_tis_after"
assert_debug_env_unchanged "inputTextRouterSelfChecks" "$input_text_router_debug_before" "$input_text_router_debug_after"
assert_no_user_host "inputTextRouterSelfChecks"
echo "inputTextRouterSelfChecks=true"

section "debug env restore self-check"
debug_restore_sentinel="/tmp/inputia-debug-env-original.$$"
debug_restore_temp="/tmp/inputia-debug-env-temp.$$"
if run_allow_launchctl_env_probe "debugEnvRestoreSetOriginal" /bin/launchctl setenv INPUTIA_DEBUG_EVENTS "$debug_restore_sentinel"; then
  inputia_capture_debug_events_env
  if run_allow_launchctl_env_probe "debugEnvRestoreSetTemp" /bin/launchctl setenv INPUTIA_DEBUG_EVENTS "$debug_restore_temp"; then
    inputia_restore_debug_events_env
    debug_restore_after="$(debug_events_env)"
    echo "debugEnvRestoreExpected=$debug_restore_sentinel"
    echo "debugEnvRestoreActual=${debug_restore_after:-unset}"
    if [[ "$debug_restore_after" != "$debug_restore_sentinel" ]]; then
      echo "nonGuiVerificationPassed=false reason=debug-env-restore-self-check"
      exit 1
    fi
    restore_verify_debug_env
    echo "launchctlEnvReady=true"
    echo "debugEnvRestoreSelfCheck=true"
  else
    restore_verify_debug_env
    echo "launchctlEnvReady=false"
    echo "debugEnvRestoreSelfCheck=skipped reason=launchctl-env-unavailable"
  fi
else
  restore_verify_debug_env
  echo "launchctlEnvReady=false"
  echo "debugEnvRestoreSelfCheck=skipped reason=launchctl-env-unavailable"
fi

section "temp dir cleanup self-check"
temp_dir_cleanup_inputia="/tmp/inputia-temp-dir-cleanup.$$"
temp_dir_cleanup_non_inputia="/tmp/not-inputia-temp-dir-cleanup.$$"
/bin/rm -rf "$temp_dir_cleanup_inputia" "$temp_dir_cleanup_non_inputia"
/bin/mkdir "$temp_dir_cleanup_inputia" "$temp_dir_cleanup_non_inputia"
VERIFY_TEMP_DIRS+=("$temp_dir_cleanup_inputia" "$temp_dir_cleanup_non_inputia")
cleanup_verify_temp_dirs
if [[ -e "$temp_dir_cleanup_inputia" ]]; then
  echo "nonGuiVerificationPassed=false reason=temp-dir-cleanup-inputia-not-removed"
  /bin/rm -rf "$temp_dir_cleanup_inputia" "$temp_dir_cleanup_non_inputia"
  exit 1
fi
echo "tempDirCleanupInputiaRemoved=true"
if [[ ! -d "$temp_dir_cleanup_non_inputia" ]]; then
  echo "nonGuiVerificationPassed=false reason=temp-dir-cleanup-removed-non-inputia"
  exit 1
fi
echo "tempDirCleanupNonInputiaPreserved=true"
/bin/rm -rf "$temp_dir_cleanup_non_inputia"
echo "tempDirCleanupSelfCheck=true"

section "smoke file cleanup helper self-check"
smoke_file_cleanup_removed="/tmp/inputia-smoke-file-cleanup-removed.$$"
smoke_file_cleanup_kept="/tmp/inputia-smoke-file-cleanup-kept.$$"
smoke_file_cleanup_blank=""
VERIFY_TEMP_FILES+=("$smoke_file_cleanup_removed" "$smoke_file_cleanup_kept")
/usr/bin/printf 'remove-me' >"$smoke_file_cleanup_removed"
inputia_cleanup_smoke_files "$smoke_file_cleanup_removed" "$smoke_file_cleanup_blank"
if [[ -e "$smoke_file_cleanup_removed" ]]; then
  echo "nonGuiVerificationPassed=false reason=smoke-file-cleanup-did-not-remove"
  /bin/rm -f "$smoke_file_cleanup_removed" "$smoke_file_cleanup_kept"
  exit 1
fi
echo "smokeFileCleanupRemoved=true"
/usr/bin/printf 'keep-me' >"$smoke_file_cleanup_kept"
smoke_file_cleanup_keep_output="$(INPUTIA_KEEP_SMOKE_LOGS=1 inputia_cleanup_smoke_files "$smoke_file_cleanup_kept" 2>&1)"
printf '%s\n' "$smoke_file_cleanup_keep_output" | /usr/bin/sed 's/^/smokeFileCleanupKeepSelfCheck: /'
require_output "$smoke_file_cleanup_keep_output" "smokeTempCleanup=skipped" "smoke-file-cleanup-keep-missing-skip-marker"
if [[ ! -f "$smoke_file_cleanup_kept" ]]; then
  echo "nonGuiVerificationPassed=false reason=smoke-file-cleanup-keep-removed-file"
  exit 1
fi
echo "smokeFileCleanupKeepPreserved=true"
/bin/rm -f "$smoke_file_cleanup_kept"
echo "smokeFileCleanupHelperSelfCheck=true"

section "input source restore helper self-check"
input_source_restore_fake_tis="/tmp/inputia-restore-fake-tis.$$"
input_source_restore_log="/tmp/inputia-restore-fake-tis.$$.log"
VERIFY_TEMP_FILES+=("$input_source_restore_fake_tis" "$input_source_restore_log")
/bin/cat >"$input_source_restore_fake_tis" <<'FAKE_TIS'
#!/bin/bash
set -euo pipefail
case "${1:-}" in
  --dump-current-input-source)
    echo "id=com.inputia.current"
    ;;
  --select-source-id)
    echo "selectedSourceID=${2:-missing}"
    ;;
  *)
    echo "unexpectedFakeTISCommand=${1:-missing}" >&2
    exit 2
    ;;
esac
FAKE_TIS
/bin/chmod +x "$input_source_restore_fake_tis"
input_source_restore_current_output="$(
  INPUTIA_PREVIOUS_INPUT_SOURCE_ID="com.inputia.current" \
    inputia_restore_previous_input_source "$input_source_restore_fake_tis" "/nonexistent/InputiaInputMethod" "$input_source_restore_log" 2>&1
)"
printf '%s\n' "$input_source_restore_current_output" | /usr/bin/sed 's/^/inputSourceRestoreCurrentSelfCheck: /'
require_output "$input_source_restore_current_output" "inputSourceRestore=skipped reason=already-current" "input-source-restore-already-current-not-skipped"
set +e
input_source_restore_failure_output="$(
  INPUTIA_PREVIOUS_INPUT_SOURCE_ID="com.inputia.previous" \
    inputia_restore_previous_input_source "$input_source_restore_fake_tis" "/nonexistent/InputiaInputMethod" "$input_source_restore_log" 2>&1
)"
input_source_restore_failure_rc=$?
set -e
printf '%s\n' "$input_source_restore_failure_output" | /usr/bin/sed 's/^/inputSourceRestoreFailureSelfCheck: /'
echo "inputSourceRestoreFailureSelfCheck.rc=$input_source_restore_failure_rc"
if [[ "$input_source_restore_failure_rc" != "1" ]]; then
  echo "nonGuiVerificationPassed=false reason=input-source-restore-failure-not-fatal"
  exit 1
fi
require_output "$input_source_restore_failure_output" "inputSourceRestore=false expected=com.inputia.previous actual=com.inputia.current" "input-source-restore-failure-marker-missing"
if ! /usr/bin/grep -q "selectedSourceID=com.inputia.previous" "$input_source_restore_log"; then
  echo "nonGuiVerificationPassed=false reason=input-source-restore-select-not-attempted"
  exit 1
fi
/bin/rm -f "$input_source_restore_fake_tis" "$input_source_restore_log"
echo "inputSourceRestoreHelperSelfCheck=true"

section "process preflight helper self-check"
process_preflight_name="InputiaFakeProcessForTest"
process_preflight_clear_output="$(
  INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST=1 \
    /bin/bash -c 'source "$1"; inputia_require_process_not_running "$2" fakeReady 44 fake-running INPUTIA_FAKE_PROCESS_ALLOW' \
    _ "$ROOT_DIR/smoke-common.sh" "$process_preflight_name" 2>&1
)"
printf '%s\n' "$process_preflight_clear_output" | /usr/bin/sed 's/^/processPreflightClearSelfCheck: /'
require_output "$process_preflight_clear_output" "${process_preflight_name}Preflight=not-running" "process-preflight-clear-missing-not-running"
set +e
process_preflight_block_output="$(
  INPUTIA_PROCESS_RUNNING_FOR_TEST="$process_preflight_name" \
    INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST=1 \
    /bin/bash -c 'source "$1"; inputia_require_process_not_running "$2" fakeReady 44 fake-running INPUTIA_FAKE_PROCESS_ALLOW' \
      _ "$ROOT_DIR/smoke-common.sh" "$process_preflight_name" 2>&1
)"
process_preflight_block_rc=$?
set -e
printf '%s\n' "$process_preflight_block_output" | /usr/bin/sed 's/^/processPreflightBlockSelfCheck: /'
echo "processPreflightBlockSelfCheck.rc=$process_preflight_block_rc"
if [[ "$process_preflight_block_rc" != "44" ]]; then
  echo "nonGuiVerificationPassed=false reason=process-preflight-block-rc actual=$process_preflight_block_rc"
  exit 1
fi
require_output "$process_preflight_block_output" "${process_preflight_name}Preflight=running" "process-preflight-block-missing-running"
require_output "$process_preflight_block_output" "guiSmokeReady=false reason=fake-running" "process-preflight-block-missing-gui-ready"
require_output "$process_preflight_block_output" "fakeReady=false reason=fake-running" "process-preflight-block-missing-specific-ready"
process_preflight_allow_output="$(
  INPUTIA_PROCESS_RUNNING_FOR_TEST="$process_preflight_name" \
    INPUTIA_FAKE_PROCESS_ALLOW=1 \
    INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST=1 \
    /bin/bash -c 'source "$1"; inputia_require_process_not_running "$2" fakeReady 44 fake-running INPUTIA_FAKE_PROCESS_ALLOW' \
      _ "$ROOT_DIR/smoke-common.sh" "$process_preflight_name" 2>&1
)"
printf '%s\n' "$process_preflight_allow_output" | /usr/bin/sed 's/^/processPreflightAllowSelfCheck: /'
require_output "$process_preflight_allow_output" "${process_preflight_name}Preflight=running" "process-preflight-allow-missing-running"
require_output "$process_preflight_allow_output" "${process_preflight_name}PreflightAllowed=true" "process-preflight-allow-missing-allowed"
set +e
process_preflight_no_allow_output="$(
  INPUTIA_PROCESS_RUNNING_FOR_TEST="$process_preflight_name" \
    INPUTIA_PROCESS_IGNORE_REAL_FOR_TEST=1 \
    INPUTIA_FAKE_PROCESS_ALLOW=1 \
    /bin/bash -c 'source "$1"; inputia_require_process_not_running "$2" fakeReady 45 fake-running -' \
      _ "$ROOT_DIR/smoke-common.sh" "$process_preflight_name" 2>&1
)"
process_preflight_no_allow_rc=$?
set -e
printf '%s\n' "$process_preflight_no_allow_output" | /usr/bin/sed 's/^/processPreflightNoAllowSelfCheck: /'
echo "processPreflightNoAllowSelfCheck.rc=$process_preflight_no_allow_rc"
if [[ "$process_preflight_no_allow_rc" != "45" ]]; then
  echo "nonGuiVerificationPassed=false reason=process-preflight-no-allow-rc actual=$process_preflight_no_allow_rc"
  exit 1
fi
require_output "$process_preflight_no_allow_output" "${process_preflight_name}Preflight=running" "process-preflight-no-allow-missing-running"
require_output "$process_preflight_no_allow_output" "guiSmokeReady=false reason=fake-running" "process-preflight-no-allow-missing-gui-ready"
require_output "$process_preflight_no_allow_output" "fakeReady=false reason=fake-running" "process-preflight-no-allow-missing-specific-ready"
echo "processPreflightHelperSelfCheck=true"

section "await UI status self-check"
run_allow_rc "0,1,2,3,4,5" "awaitUiStatusSelfCheckRun" \
  env INPUTIA_AWAIT_UI_STATUS_SELF_CHECK=1 "$ROOT_DIR/await-system-install.sh"
await_ui_status_output="$RUN_EXPECT_RC_OUTPUT"
printf '%s\n' "$await_ui_status_output"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck=true" "await-ui-status-self-check-failed"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=target-and-tis uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,missing-enabled-source" "await-ui-status-missing-target-and-tis-block-reasons"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=target-tis-userhost uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=target-cdhash-mismatch uiSmokeBlockReasons=target-cdhash-mismatch,missing-enabled-source,user-host-conflict" "await-ui-status-missing-target-tis-userhost-block-reasons"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=signature-rejected uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=signature-rejected uiSmokeBlockReasons=signature-rejected" "await-ui-status-missing-signature-rejected-block-line"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=app-missing uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=app-missing" "await-ui-status-missing-app-missing-block-line"
for await_gui_reason in no-console-user gui-bootstrap-unavailable login-not-complete screen-locked frontmost-unavailable loginwindow-frontmost; do
  require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=$await_gui_reason uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=$await_gui_reason uiSmokeBlockReasons=$await_gui_reason" "await-ui-status-missing-block-line-$await_gui_reason"
done
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=textedit-already-running uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiInputiaHostPreflight=not-running uiSmokeWouldStart=false uiSmokeBlockReason=textedit-already-running uiSmokeBlockReasons=textedit-already-running" "await-ui-status-missing-textedit-running-block-line"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=safari-already-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiInputiaHostPreflight=not-running uiSmokeWouldStart=false uiSmokeBlockReason=safari-already-running uiSmokeBlockReasons=safari-already-running" "await-ui-status-missing-safari-running-block-line"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=inputia-host-running uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=not-running uiInputiaHostPreflight=running uiSmokeWouldStart=false uiSmokeBlockReason=inputia-host-running uiSmokeBlockReasons=inputia-host-running" "await-ui-status-missing-inputia-host-running-block-line"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=user-host-conflict uiSmokeRequested=true uiSmokeWouldStart=false uiSmokeBlockReason=user-host-conflict uiSmokeBlockReasons=user-host-conflict" "await-ui-status-missing-user-host-conflict-block-line"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=textedit-allow uiSmokeRequested=true uiTextEditPreflight=running uiSafariPreflight=not-running uiInputiaHostPreflight=not-running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none" "await-ui-status-missing-textedit-allow-line"
require_output "$await_ui_status_output" "awaitUiStatusSelfCheck reason=safari-allow uiSmokeRequested=true uiTextEditPreflight=not-running uiSafariPreflight=running uiInputiaHostPreflight=not-running uiSmokeWouldStart=true uiSmokeBlockReason=none uiSmokeBlockReasons=none" "await-ui-status-missing-safari-allow-line"

section "build preflight self-check"
build_preflight_output="$(INPUTIA_BUILD_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/build.sh" 2>&1)"
printf '%s\n' "$build_preflight_output"
require_output "$build_preflight_output" "buildPreflightSelfCheck clear=true" "build-preflight-clear-case-failed"
require_output "$build_preflight_output" "buildPreflightSelfCheck blocked=true" "build-preflight-blocked-case-failed"
require_output "$build_preflight_output" "buildPreflightSelfCheck=true" "build-preflight-self-check-failed"

section "build package preflight self-check"
build_pkg_preflight_output="$(INPUTIA_BUILD_PKG_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/build-pkg.sh" 2>&1)"
printf '%s\n' "$build_pkg_preflight_output"
require_output "$build_pkg_preflight_output" "buildPkgPreflightSelfCheck clear=true" "build-pkg-preflight-clear-case-failed"
require_output "$build_pkg_preflight_output" "buildPkgPreflightSelfCheck blocked=true" "build-pkg-preflight-blocked-case-failed"
require_output "$build_pkg_preflight_output" "buildPkgPreflightSelfCheck=true" "build-pkg-preflight-self-check-failed"

section "build package signing identity self-check"
pkg_before_sha="$(/usr/bin/shasum -a 256 "$ROOT_DIR/dist/InputiaInputMethod-latest.pkg" 2>/dev/null | /usr/bin/awk '{ print $1 }' || true)"
run_expect_rc 22 "buildPkgWrongSigningIdentitySelfCheck" \
  /usr/bin/env \
    INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST="123 /usr/bin/true" \
    INPUTIA_PKG_SIGN_IDENTITY="Codexbar Local Code Signing Leaf v4" \
    "$ROOT_DIR/build-pkg.sh"
require_output "$RUN_EXPECT_RC_OUTPUT" "buildPkgSignIdentityRequested=true" "build-pkg-wrong-sign-identity-missing-request-output"
require_output "$RUN_EXPECT_RC_OUTPUT" "buildPkgSignIdentityValid=false reason=not-developer-id-installer" "build-pkg-wrong-sign-identity-missing-invalid-output"
require_output "$RUN_EXPECT_RC_OUTPUT" "buildPkgReady=false reason=pkg-sign-identity-not-developer-id-installer" "build-pkg-wrong-sign-identity-missing-ready-output"
run_expect_rc 21 "buildPkgMissingSigningIdentitySelfCheck" \
  /usr/bin/env \
    INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST="123 /usr/bin/true" \
    INPUTIA_PKG_SIGN_IDENTITY="Developer ID Installer: Inputia Missing Installer (TEAMID)" \
    "$ROOT_DIR/build-pkg.sh"
require_output "$RUN_EXPECT_RC_OUTPUT" "buildPkgSignIdentityRequested=true" "build-pkg-missing-sign-identity-missing-request-output"
require_output "$RUN_EXPECT_RC_OUTPUT" "buildPkgSignIdentityValid=false reason=missing-developer-id-installer-identity" "build-pkg-missing-sign-identity-missing-invalid-output"
require_output "$RUN_EXPECT_RC_OUTPUT" "buildPkgReady=false reason=missing-pkg-sign-identity" "build-pkg-missing-sign-identity-missing-ready-output"
require_output "$RUN_EXPECT_RC_OUTPUT" "buildPkgRequiredAction=import-developer-id-installer-identity" "build-pkg-missing-sign-identity-missing-required-action"
pkg_after_sha="$(/usr/bin/shasum -a 256 "$ROOT_DIR/dist/InputiaInputMethod-latest.pkg" 2>/dev/null | /usr/bin/awk '{ print $1 }' || true)"
if [[ "$pkg_before_sha" != "$pkg_after_sha" ]]; then
  echo "nonGuiVerificationPassed=false reason=build-pkg-signing-self-check-mutated-package"
  exit 1
fi
echo "buildPkgSigningIdentitySelfCheck=true"

section "open settings preflight self-check"
open_settings_preflight_output="$(INPUTIA_OPEN_SETTINGS_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/open-settings.sh" 2>&1)"
printf '%s\n' "$open_settings_preflight_output"
require_output "$open_settings_preflight_output" "openSettingsPreflightSelfCheck clear=true" "open-settings-preflight-clear-case-failed"
require_output "$open_settings_preflight_output" "openSettingsPreflightSelfCheck blocked=true" "open-settings-preflight-blocked-case-failed"
require_output "$open_settings_preflight_output" "openSettingsPreflightSelfCheck=true" "open-settings-preflight-self-check-failed"

section "open installer preflight self-check"
open_installer_preflight_output="$(INPUTIA_OPEN_INSTALLER_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/open-installer.sh" 2>&1)"
printf '%s\n' "$open_installer_preflight_output"
require_output "$open_installer_preflight_output" "openInstallerPreflightSelfCheck clear=true" "open-installer-preflight-clear-case-failed"
require_output "$open_installer_preflight_output" "openInstallerPreflightSelfCheck blocked=true" "open-installer-preflight-blocked-case-failed"
require_output "$open_installer_preflight_output" "openInstallerPreflightSelfCheck=true" "open-installer-preflight-self-check-failed"

section "system install preflight self-check"
install_preflight_output="$(INPUTIA_INSTALL_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/install-system.sh" 2>&1)"
printf '%s\n' "$install_preflight_output"
require_output "$install_preflight_output" "systemInstallPreflightSelfCheck clear=true" "install-preflight-clear-case-failed"
require_output "$install_preflight_output" "systemInstallPreflightSelfCheck blocked=true" "install-preflight-blocked-case-failed"
require_output "$install_preflight_output" "systemInstallPreflightSelfCheck=true" "install-preflight-self-check-failed"

section "user install preflight self-check"
install_user_preflight_output="$(INPUTIA_INSTALL_USER_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/install-user.sh" 2>&1)"
printf '%s\n' "$install_user_preflight_output"
require_output "$install_user_preflight_output" "userInstallPreflightSelfCheck clear=true" "install-user-preflight-clear-case-failed"
require_output "$install_user_preflight_output" "userInstallPreflightSelfCheck blocked=true" "install-user-preflight-blocked-case-failed"
require_output "$install_user_preflight_output" "userInstallPreflightSelfCheck=true" "install-user-preflight-self-check-failed"

section "system uninstall preflight self-check"
uninstall_preflight_output="$(INPUTIA_UNINSTALL_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/uninstall-system.sh" 2>&1)"
printf '%s\n' "$uninstall_preflight_output"
require_output "$uninstall_preflight_output" "systemUninstallPreflightSelfCheck clear=true" "uninstall-preflight-clear-case-failed"
require_output "$uninstall_preflight_output" "systemUninstallPreflightSelfCheck blocked=true" "uninstall-preflight-blocked-case-failed"
require_output "$uninstall_preflight_output" "systemUninstallPreflightSelfCheck=true" "uninstall-preflight-self-check-failed"

section "user uninstall preflight self-check"
uninstall_user_preflight_output="$(INPUTIA_UNINSTALL_USER_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/uninstall-user.sh" 2>&1)"
printf '%s\n' "$uninstall_user_preflight_output"
require_output "$uninstall_user_preflight_output" "userUninstallPreflightSelfCheck clear=true" "uninstall-user-preflight-clear-case-failed"
require_output "$uninstall_user_preflight_output" "userUninstallPreflightSelfCheck blocked=true" "uninstall-user-preflight-blocked-case-failed"
require_output "$uninstall_user_preflight_output" "userUninstallPreflightSelfCheck=true" "uninstall-user-preflight-self-check-failed"

section "GUI smoke readiness self-check"
gui_smoke_readiness_self_check_output="$(INPUTIA_GUI_SMOKE_READINESS_SELF_CHECK=1 "$ROOT_DIR/gui-smoke-readiness.sh" 2>&1)"
printf '%s\n' "$gui_smoke_readiness_self_check_output"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=pkg expected=pkg-not-ready actual=pkg-not-ready" "gui-readiness-self-check-missing-pkg"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=admin expected=admin-required actual=admin-required" "gui-readiness-self-check-missing-admin"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=target expected=target-cdhash-mismatch actual=target-cdhash-mismatch" "gui-readiness-self-check-missing-target"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=userhost expected=user-host-conflict actual=user-host-conflict" "gui-readiness-self-check-missing-user-host-conflict"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=inputia expected=inputia-host-running actual=inputia-host-running" "gui-readiness-self-check-missing-inputia-host"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=gui-bootstrap expected=gui-bootstrap-unavailable actual=gui-bootstrap-unavailable" "gui-readiness-self-check-missing-gui-bootstrap"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=ready expected=none actual=none" "gui-readiness-self-check-missing-ready"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=allow-textedit expected=none actual=none" "gui-readiness-self-check-missing-allow-textedit"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=allow-safari expected=none actual=none" "gui-readiness-self-check-missing-allow-safari"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=signature expected=signature-rejected actual=signature-rejected" "gui-readiness-self-check-missing-signature"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck case=appmissing expected=app-missing actual=app-missing" "gui-readiness-self-check-missing-app-missing"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck allBlockReasons=" "gui-readiness-self-check-missing-all-block-reasons"
for readiness_required_reason in target-cdhash-mismatch admin-required settings-version-mismatch tis-not-ready user-host-conflict inputia-host-running screen-locked textedit-already-running safari-already-running; do
  require_output "$gui_smoke_readiness_self_check_output" "$readiness_required_reason" "gui-readiness-self-check-missing-all-block-reason-$readiness_required_reason"
done
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck signatureBlockReasons=signature-rejected actual=signature-rejected" "gui-readiness-self-check-missing-signature-block-reasons"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck appMissingBlockReasons=app-missing,tis-not-ready actual=" "gui-readiness-self-check-missing-app-missing-block-reasons"
require_output "$gui_smoke_readiness_self_check_output" "guiSmokeReadinessSelfCheck=true" "gui-readiness-self-check-failed"

if ! process_running InputiaInputMethod; then
  start_fake_existing_process InputiaInputMethod
  fake_readiness_inputia_pid="$INPUTIA_FAKE_EXISTING_PID"
  run_allow_rc "0,1,2,3,4,5" "guiSmokeReadinessInputiaHostGate" \
    "$ROOT_DIR/gui-smoke-readiness.sh" "$BUILD_APP"
  gui_smoke_readiness_inputia_output="$RUN_EXPECT_RC_OUTPUT"
  require_output "$gui_smoke_readiness_inputia_output" "inputiaHostPreflight=running" "gui-readiness-inputia-host-gate-missing-preflight"
  require_output "$gui_smoke_readiness_inputia_output" "inputia-host-running" "gui-readiness-inputia-host-gate-missing-blocker"
  stop_fake_existing_process InputiaInputMethod "$fake_readiness_inputia_pid"
  echo "guiSmokeReadinessInputiaHostGateSelfCheck=true"
else
  echo "guiSmokeReadinessInputiaHostGateSelfCheck=skipped reason=inputia-host-preexisting"
fi

gui_readiness_user_host_root="$(/usr/bin/mktemp -d "/tmp/inputia-gui-readiness-user-host.XXXXXX")"
VERIFY_TEMP_DIRS+=("$gui_readiness_user_host_root")
gui_readiness_fake_user_app="$gui_readiness_user_host_root/InputiaInputMethod.app"
gui_readiness_fake_system_app="$gui_readiness_user_host_root/SystemInputiaInputMethod.app"
/bin/mkdir -p "$gui_readiness_fake_user_app"
/bin/mkdir -p "$gui_readiness_fake_system_app"
gui_smoke_readiness_user_host_output="$(
  INPUTIA_USER_APP_FOR_TEST="$gui_readiness_fake_user_app" \
    INPUTIA_SYSTEM_APP_FOR_TEST="$gui_readiness_fake_system_app" \
    "$ROOT_DIR/gui-smoke-readiness.sh" "$BUILD_APP" 2>&1
)"
printf '%s\n' "$gui_smoke_readiness_user_host_output" | /usr/bin/sed 's/^/guiSmokeReadinessUserHostGate: /'
require_output "$gui_smoke_readiness_user_host_output" "userHostConflict=true" "gui-readiness-user-host-gate-missing-conflict-state"
require_output "$gui_smoke_readiness_user_host_output" "user-host-conflict" "gui-readiness-user-host-gate-missing-blocker"
echo "guiSmokeReadinessUserHostGateSelfCheck=true"

section "GUI smoke readiness current gate"
gui_smoke_readiness_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_readiness_tis_before="$(current_input_source_id)"
gui_smoke_readiness_debug_before="$(debug_events_env)"
gui_smoke_readiness_current_output="$("$ROOT_DIR/gui-smoke-readiness.sh" "$BUILD_APP" 2>&1)"
printf '%s\n' "$gui_smoke_readiness_current_output" | /usr/bin/sed 's/^/guiSmokeReadinessCurrent: /'
require_output "$gui_smoke_readiness_current_output" "guiSmokeReadinessReady=" "gui-readiness-current-missing-ready-line"
gui_smoke_readiness_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_readiness_tis_after="$(current_input_source_id)"
gui_smoke_readiness_debug_after="$(debug_events_env)"
if [[ "$gui_smoke_readiness_clipboard_after" != "$gui_smoke_readiness_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=gui-smoke-readiness-mutated-clipboard"
  exit 1
fi
echo "guiSmokeReadinessCurrent.clipboardUnchanged=true"
assert_current_source_unchanged "guiSmokeReadinessCurrent" "$gui_smoke_readiness_tis_before" "$gui_smoke_readiness_tis_after"
assert_debug_env_unchanged "guiSmokeReadinessCurrent" "$gui_smoke_readiness_debug_before" "$gui_smoke_readiness_debug_after"
assert_no_user_host "guiSmokeReadinessCurrent"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "gui-smoke-readiness-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "gui-smoke-readiness-launched-safari"
fi
assert_process_not_running osascript "gui-smoke-readiness-left-osascript"
assert_process_not_running InputiaInputMethod "gui-smoke-readiness-left-inputia-host"
echo "guiSmokeReadinessCurrentNoMutationPassed=true"

section "GUI smoke suite self-check"
gui_smoke_suite_self_check_output="$(INPUTIA_GUI_SMOKE_SUITE_SELF_CHECK=1 "$ROOT_DIR/gui-smoke-suite.sh" 2>&1)"
printf '%s\n' "$gui_smoke_suite_self_check_output"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteReady=false reason=readiness-output-missing" "gui-suite-self-check-missing-output-ready-line"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteBlockReasons=readiness-output-missing" "gui-suite-self-check-missing-output-block-reasons"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=missing guiSmokeSuiteWouldRun=false" "gui-suite-self-check-missing-output-no-run"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=missing rc=12" "gui-suite-self-check-missing-output-rc"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteReady=false reason=admin-required" "gui-suite-self-check-missing-blocked-ready-line"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready" "gui-suite-self-check-missing-blocked-block-reasons"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=blocked guiSmokeSuiteWouldRun=false" "gui-suite-self-check-missing-blocked-no-run"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=blocked rc=12" "gui-suite-self-check-missing-blocked-rc"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteReady=true reason=none" "gui-suite-self-check-missing-ready-line"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteBlockReasons=none" "gui-suite-self-check-missing-ready-block-reasons"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteWouldRun=true" "gui-suite-self-check-missing-ready-would-run"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=ready guiSmokeSuiteRunSkipped=true reason=self-check" "gui-suite-self-check-missing-ready-skip"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteReady=false reason=readiness-inconsistent" "gui-suite-self-check-missing-inconsistent-ready-line"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteBlockReasons=tis-not-ready" "gui-suite-self-check-missing-inconsistent-block-reasons"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=inconsistent guiSmokeSuiteWouldRun=false" "gui-suite-self-check-missing-inconsistent-no-run"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=inconsistent rc=12" "gui-suite-self-check-missing-inconsistent-rc"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuitePostInstallForTest=true rc=23" "gui-suite-self-check-missing-post-install-failure-test-hook"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=post-install-failure guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23" "gui-suite-self-check-missing-post-install-failure-marker"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck case=post-install-failure rc=23" "gui-suite-self-check-missing-post-install-failure-rc"
require_output "$gui_smoke_suite_self_check_output" "guiSmokeSuiteSelfCheck=true" "gui-suite-self-check-failed"

section "GUI smoke suite missing-readiness gate"
gui_smoke_suite_missing_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_missing_tis_before="$(current_input_source_id)"
gui_smoke_suite_missing_debug_before="$(debug_events_env)"
run_expect_rc 12 "guiSmokeSuiteMissingReadinessGate" \
  env INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST="" \
    "$ROOT_DIR/gui-smoke-suite.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteReady=false reason=readiness-output-missing" "gui-suite-missing-readiness-gate-missing-ready-line"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteBlockReasons=readiness-output-missing" "gui-suite-missing-readiness-gate-missing-block-reasons"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteWouldRun=false" "gui-suite-missing-readiness-gate-missing-would-not-run"
gui_smoke_suite_missing_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_missing_tis_after="$(current_input_source_id)"
gui_smoke_suite_missing_debug_after="$(debug_events_env)"
if [[ "$gui_smoke_suite_missing_clipboard_after" != "$gui_smoke_suite_missing_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=gui-smoke-suite-missing-readiness-mutated-clipboard"
  exit 1
fi
echo "guiSmokeSuiteMissingReadinessGate.clipboardUnchanged=true"
assert_current_source_unchanged "guiSmokeSuiteMissingReadinessGate" "$gui_smoke_suite_missing_tis_before" "$gui_smoke_suite_missing_tis_after"
assert_debug_env_unchanged "guiSmokeSuiteMissingReadinessGate" "$gui_smoke_suite_missing_debug_before" "$gui_smoke_suite_missing_debug_after"
assert_no_user_host "guiSmokeSuiteMissingReadinessGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "gui-smoke-suite-missing-readiness-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "gui-smoke-suite-missing-readiness-launched-safari"
fi
assert_process_not_running osascript "gui-smoke-suite-missing-readiness-left-osascript"
assert_process_not_running InputiaInputMethod "gui-smoke-suite-missing-readiness-left-inputia-host"
echo "guiSmokeSuiteMissingReadinessGateNoMutationPassed=true"

section "GUI smoke suite blocked gate"
gui_smoke_suite_blocked_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_blocked_tis_before="$(current_input_source_id)"
gui_smoke_suite_blocked_debug_before="$(debug_events_env)"
run_expect_rc 12 "guiSmokeSuiteBlockedGate" \
  env INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST=$'guiSmokeReadinessBlockReasons=settings-version-mismatch,admin-required,tis-not-ready\nguiSmokeReadinessReady=false reason=admin-required' \
    "$ROOT_DIR/gui-smoke-suite.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteReady=false reason=admin-required" "gui-suite-blocked-gate-missing-ready-line"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteBlockReasons=settings-version-mismatch,admin-required,tis-not-ready" "gui-suite-blocked-gate-missing-block-reasons"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteWouldRun=false" "gui-suite-blocked-gate-missing-would-not-run"
gui_smoke_suite_blocked_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_blocked_tis_after="$(current_input_source_id)"
gui_smoke_suite_blocked_debug_after="$(debug_events_env)"
if [[ "$gui_smoke_suite_blocked_clipboard_after" != "$gui_smoke_suite_blocked_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=gui-smoke-suite-blocked-mutated-clipboard"
  exit 1
fi
echo "guiSmokeSuiteBlockedGate.clipboardUnchanged=true"
assert_current_source_unchanged "guiSmokeSuiteBlockedGate" "$gui_smoke_suite_blocked_tis_before" "$gui_smoke_suite_blocked_tis_after"
assert_debug_env_unchanged "guiSmokeSuiteBlockedGate" "$gui_smoke_suite_blocked_debug_before" "$gui_smoke_suite_blocked_debug_after"
assert_no_user_host "guiSmokeSuiteBlockedGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "gui-smoke-suite-blocked-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "gui-smoke-suite-blocked-launched-safari"
fi
assert_process_not_running osascript "gui-smoke-suite-blocked-left-osascript"
assert_process_not_running InputiaInputMethod "gui-smoke-suite-blocked-left-inputia-host"
echo "guiSmokeSuiteBlockedGateNoMutationPassed=true"

section "GUI smoke suite current blocked gate"
gui_smoke_suite_current_blocked_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_current_blocked_tis_before="$(current_input_source_id)"
gui_smoke_suite_current_blocked_debug_before="$(debug_events_env)"
run_expect_rc 12 "guiSmokeSuiteCurrentBlockedGate" \
  "$ROOT_DIR/gui-smoke-suite.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteReady=false reason=" "gui-suite-current-blocked-gate-missing-ready-line"
require_output_regex \
  "$RUN_EXPECT_RC_OUTPUT" \
  'guiSmokeSuiteBlockReasons=.*(signature-rejected|tis-not-ready|pkg-not-ready|admin-required|gui-bootstrap-unavailable|frontmost-unavailable|target-cdhash-mismatch)' \
  "gui-suite-current-blocked-gate-missing-safe-blocker"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteWouldRun=false" "gui-suite-current-blocked-gate-missing-would-not-run"
gui_smoke_suite_current_blocked_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_current_blocked_tis_after="$(current_input_source_id)"
gui_smoke_suite_current_blocked_debug_after="$(debug_events_env)"
if [[ "$gui_smoke_suite_current_blocked_clipboard_after" != "$gui_smoke_suite_current_blocked_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=gui-smoke-suite-current-blocked-mutated-clipboard"
  exit 1
fi
echo "guiSmokeSuiteCurrentBlockedGate.clipboardUnchanged=true"
assert_current_source_unchanged "guiSmokeSuiteCurrentBlockedGate" "$gui_smoke_suite_current_blocked_tis_before" "$gui_smoke_suite_current_blocked_tis_after"
assert_debug_env_unchanged "guiSmokeSuiteCurrentBlockedGate" "$gui_smoke_suite_current_blocked_debug_before" "$gui_smoke_suite_current_blocked_debug_after"
assert_no_user_host "guiSmokeSuiteCurrentBlockedGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "gui-smoke-suite-current-blocked-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "gui-smoke-suite-current-blocked-launched-safari"
fi
assert_process_not_running osascript "gui-smoke-suite-current-blocked-left-osascript"
assert_process_not_running InputiaInputMethod "gui-smoke-suite-current-blocked-left-inputia-host"
echo "guiSmokeSuiteCurrentBlockedGateNoMutationPassed=true"

section "GUI smoke suite inconsistent readiness gate"
gui_smoke_suite_inconsistent_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_inconsistent_tis_before="$(current_input_source_id)"
gui_smoke_suite_inconsistent_debug_before="$(debug_events_env)"
run_expect_rc 12 "guiSmokeSuiteInconsistentReadinessGate" \
  env INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST=$'guiSmokeReadinessBlockReasons=tis-not-ready\nguiSmokeReadinessReady=true reason=none' \
    "$ROOT_DIR/gui-smoke-suite.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteReady=false reason=readiness-inconsistent" "gui-suite-inconsistent-readiness-gate-missing-ready-line"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteBlockReasons=tis-not-ready" "gui-suite-inconsistent-readiness-gate-missing-block-reasons"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteWouldRun=false" "gui-suite-inconsistent-readiness-gate-missing-would-not-run"
gui_smoke_suite_inconsistent_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_inconsistent_tis_after="$(current_input_source_id)"
gui_smoke_suite_inconsistent_debug_after="$(debug_events_env)"
if [[ "$gui_smoke_suite_inconsistent_clipboard_after" != "$gui_smoke_suite_inconsistent_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=gui-smoke-suite-inconsistent-readiness-mutated-clipboard"
  exit 1
fi
echo "guiSmokeSuiteInconsistentReadinessGate.clipboardUnchanged=true"
assert_current_source_unchanged "guiSmokeSuiteInconsistentReadinessGate" "$gui_smoke_suite_inconsistent_tis_before" "$gui_smoke_suite_inconsistent_tis_after"
assert_debug_env_unchanged "guiSmokeSuiteInconsistentReadinessGate" "$gui_smoke_suite_inconsistent_debug_before" "$gui_smoke_suite_inconsistent_debug_after"
assert_no_user_host "guiSmokeSuiteInconsistentReadinessGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "gui-smoke-suite-inconsistent-readiness-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "gui-smoke-suite-inconsistent-readiness-launched-safari"
fi
assert_process_not_running osascript "gui-smoke-suite-inconsistent-readiness-left-osascript"
assert_process_not_running InputiaInputMethod "gui-smoke-suite-inconsistent-readiness-left-inputia-host"
echo "guiSmokeSuiteInconsistentReadinessGateNoMutationPassed=true"

section "GUI smoke suite ready-skip gate"
gui_smoke_suite_ready_skip_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_ready_skip_tis_before="$(current_input_source_id)"
gui_smoke_suite_ready_skip_debug_before="$(debug_events_env)"
run_expect_rc 0 "guiSmokeSuiteReadySkipGate" \
  env INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST=$'guiSmokeReadinessBlockReasons=none\nguiSmokeReadinessReady=true reason=none' \
    INPUTIA_GUI_SMOKE_SUITE_SKIP_RUN_FOR_TEST=1 \
    "$ROOT_DIR/gui-smoke-suite.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteReady=true reason=none" "gui-suite-ready-skip-gate-missing-ready-line"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteBlockReasons=none" "gui-suite-ready-skip-gate-missing-block-reasons"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteWouldRun=true" "gui-suite-ready-skip-gate-missing-would-run"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteRunSkipped=true reason=self-check" "gui-suite-ready-skip-gate-missing-skip-line"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuitePassed=true" "gui-suite-ready-skip-gate-missing-passed-line"
gui_smoke_suite_ready_skip_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_ready_skip_tis_after="$(current_input_source_id)"
gui_smoke_suite_ready_skip_debug_after="$(debug_events_env)"
if [[ "$gui_smoke_suite_ready_skip_clipboard_after" != "$gui_smoke_suite_ready_skip_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=gui-smoke-suite-ready-skip-mutated-clipboard"
  exit 1
fi
echo "guiSmokeSuiteReadySkipGate.clipboardUnchanged=true"
assert_current_source_unchanged "guiSmokeSuiteReadySkipGate" "$gui_smoke_suite_ready_skip_tis_before" "$gui_smoke_suite_ready_skip_tis_after"
assert_debug_env_unchanged "guiSmokeSuiteReadySkipGate" "$gui_smoke_suite_ready_skip_debug_before" "$gui_smoke_suite_ready_skip_debug_after"
assert_no_user_host "guiSmokeSuiteReadySkipGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "gui-smoke-suite-ready-skip-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "gui-smoke-suite-ready-skip-launched-safari"
fi
assert_process_not_running osascript "gui-smoke-suite-ready-skip-left-osascript"
assert_process_not_running InputiaInputMethod "gui-smoke-suite-ready-skip-left-inputia-host"
echo "guiSmokeSuiteReadySkipGateNoMutationPassed=true"

section "GUI smoke suite post-install failure gate"
gui_smoke_suite_failure_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_failure_tis_before="$(current_input_source_id)"
gui_smoke_suite_failure_debug_before="$(debug_events_env)"
run_expect_rc 23 "guiSmokeSuitePostInstallFailureGate" \
  env INPUTIA_GUI_SMOKE_SUITE_READINESS_OUTPUT_FOR_TEST=$'guiSmokeReadinessBlockReasons=none\nguiSmokeReadinessReady=true reason=none' \
    INPUTIA_GUI_SMOKE_SUITE_POST_INSTALL_RC_FOR_TEST=23 \
    "$ROOT_DIR/gui-smoke-suite.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteReady=true reason=none" "gui-suite-post-install-failure-gate-missing-ready-line"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteBlockReasons=none" "gui-suite-post-install-failure-gate-missing-block-reasons"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuiteWouldRun=true" "gui-suite-post-install-failure-gate-missing-would-run"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuitePostInstallForTest=true rc=23" "gui-suite-post-install-failure-gate-missing-test-hook-line"
require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeSuitePassed=false reason=post-install-regression-failed rc=23" "gui-suite-post-install-failure-gate-missing-failure-line"
gui_smoke_suite_failure_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
gui_smoke_suite_failure_tis_after="$(current_input_source_id)"
gui_smoke_suite_failure_debug_after="$(debug_events_env)"
if [[ "$gui_smoke_suite_failure_clipboard_after" != "$gui_smoke_suite_failure_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=gui-smoke-suite-post-install-failure-mutated-clipboard"
  exit 1
fi
echo "guiSmokeSuitePostInstallFailureGate.clipboardUnchanged=true"
assert_current_source_unchanged "guiSmokeSuitePostInstallFailureGate" "$gui_smoke_suite_failure_tis_before" "$gui_smoke_suite_failure_tis_after"
assert_debug_env_unchanged "guiSmokeSuitePostInstallFailureGate" "$gui_smoke_suite_failure_debug_before" "$gui_smoke_suite_failure_debug_after"
assert_no_user_host "guiSmokeSuitePostInstallFailureGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "gui-smoke-suite-post-install-failure-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "gui-smoke-suite-post-install-failure-launched-safari"
fi
assert_process_not_running osascript "gui-smoke-suite-post-install-failure-left-osascript"
assert_process_not_running InputiaInputMethod "gui-smoke-suite-post-install-failure-left-inputia-host"
echo "guiSmokeSuitePostInstallFailureGateNoMutationPassed=true"

section "post-install user settings conflict gate"
/bin/mkdir -p "$VERIFY_POST_INSTALL_USER_SETTINGS_APP"
post_install_user_settings_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_user_settings_tis_before="$(current_input_source_id)"
post_install_user_settings_debug_before="$(debug_events_env)"
run_expect_rc 3 "postInstallUserSettingsGate" \
  env INPUTIA_RUN_UI_SMOKE=1 \
    INPUTIA_USER_APP="$VERIFY_POST_INSTALL_USER_APP" \
    INPUTIA_USER_LEGACY_APP="$VERIFY_POST_INSTALL_USER_LEGACY_APP" \
    INPUTIA_USER_SETTINGS_APP="$VERIFY_POST_INSTALL_USER_SETTINGS_APP" \
    "$ROOT_DIR/post-install-regression.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "userHostConflict=true" "post-install-user-settings-gate-missing-conflict"
require_output "$RUN_EXPECT_RC_OUTPUT" "settingsPath=$VERIFY_POST_INSTALL_USER_SETTINGS_APP" "post-install-user-settings-gate-missing-settings-path"
post_install_user_settings_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_user_settings_tis_after="$(current_input_source_id)"
post_install_user_settings_debug_after="$(debug_events_env)"
if [[ "$post_install_user_settings_clipboard_after" != "$post_install_user_settings_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=post-install-user-settings-gate-mutated-clipboard"
  exit 1
fi
echo "postInstallUserSettingsGate.clipboardUnchanged=true"
assert_current_source_unchanged "postInstallUserSettingsGate" "$post_install_user_settings_tis_before" "$post_install_user_settings_tis_after"
assert_debug_env_unchanged "postInstallUserSettingsGate" "$post_install_user_settings_debug_before" "$post_install_user_settings_debug_after"
assert_no_user_host "postInstallUserSettingsGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "post-install-user-settings-gate-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "post-install-user-settings-gate-launched-safari"
fi
assert_process_not_running osascript "post-install-user-settings-gate-left-osascript"
assert_process_not_running InputiaInputMethod "post-install-user-settings-gate-left-inputia-host"
/bin/rmdir "$VERIFY_POST_INSTALL_USER_SETTINGS_APP" >/dev/null 2>&1 || true
echo "postInstallUserSettingsGateNoMutationPassed=true"

section "post-install active lock gate"
post_install_active_lock_dir="/tmp/inputia-post-install-active-lock.$$"
VERIFY_TEMP_DIRS+=("$post_install_active_lock_dir")
/bin/rm -rf "$post_install_active_lock_dir"
/bin/mkdir "$post_install_active_lock_dir"
echo "$$" >"$post_install_active_lock_dir/pid"
post_install_active_lock_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_active_lock_tis_before="$(current_input_source_id)"
post_install_active_lock_debug_before="$(debug_events_env)"
run_expect_rc 5 "postInstallActiveLockGate" \
  env INPUTIA_POST_INSTALL_LOCK_DIR="$post_install_active_lock_dir" \
	    INPUTIA_RUN_UI_SMOKE=1 \
	    INPUTIA_USER_APP="$VERIFY_POST_INSTALL_USER_APP" \
	    INPUTIA_USER_LEGACY_APP="$VERIFY_POST_INSTALL_USER_LEGACY_APP" \
	    INPUTIA_USER_SETTINGS_APP="$VERIFY_POST_INSTALL_USER_SETTINGS_APP" \
	    "$ROOT_DIR/post-install-regression.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "postInstallRegressionReady=false reason=already-running pid=$$" "post-install-active-lock-missing-already-running-marker"
post_install_active_lock_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_active_lock_tis_after="$(current_input_source_id)"
post_install_active_lock_debug_after="$(debug_events_env)"
if [[ "$post_install_active_lock_clipboard_after" != "$post_install_active_lock_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=post-install-active-lock-mutated-clipboard"
  /bin/rm -rf "$post_install_active_lock_dir"
  exit 1
fi
echo "postInstallActiveLockGate.clipboardUnchanged=true"
assert_current_source_unchanged "postInstallActiveLockGate" "$post_install_active_lock_tis_before" "$post_install_active_lock_tis_after"
assert_debug_env_unchanged "postInstallActiveLockGate" "$post_install_active_lock_debug_before" "$post_install_active_lock_debug_after"
assert_no_user_host "postInstallActiveLockGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "post-install-active-lock-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "post-install-active-lock-launched-safari"
fi
assert_process_not_running osascript "post-install-active-lock-left-osascript"
assert_process_not_running InputiaInputMethod "post-install-active-lock-left-inputia-host"
/bin/rm -rf "$post_install_active_lock_dir"
echo "postInstallActiveLockGateNoMutationPassed=true"

section "post-install UI preflight self-check"
post_install_ui_preflight_output="$(INPUTIA_POST_INSTALL_UI_PREFLIGHT_SELF_CHECK=1 "$ROOT_DIR/post-install-regression.sh" "$BUILD_APP" 2>&1)"
printf '%s\n' "$post_install_ui_preflight_output"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=textedit-block TextEditPreflight=running" "post-install-ui-preflight-missing-textedit-running"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=textedit-block guiSmokeReady=false reason=textedit-already-running" "post-install-ui-preflight-missing-textedit-block"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=textedit-block postInstallUiSmokeReady=false reason=textedit-already-running" "post-install-ui-preflight-missing-textedit-postinstall-block"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=textedit-block rc=4" "post-install-ui-preflight-missing-textedit-rc"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=safari-block SafariPreflight=running" "post-install-ui-preflight-missing-safari-running"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=safari-block guiSmokeReady=false reason=safari-already-running" "post-install-ui-preflight-missing-safari-block"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=safari-block postInstallUiSmokeReady=false reason=safari-already-running" "post-install-ui-preflight-missing-safari-postinstall-block"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=safari-block rc=4" "post-install-ui-preflight-missing-safari-rc"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=inputia-block InputiaInputMethodPreflight=running" "post-install-ui-preflight-missing-inputia-running"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=inputia-block guiSmokeReady=false reason=inputia-host-running" "post-install-ui-preflight-missing-inputia-block"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=inputia-block postInstallUiSmokeReady=false reason=inputia-host-running" "post-install-ui-preflight-missing-inputia-postinstall-block"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=inputia-block rc=4" "post-install-ui-preflight-missing-inputia-rc"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=textedit-allow TextEditPreflightAllowed=true" "post-install-ui-preflight-missing-textedit-allow"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck case=safari-allow SafariPreflightAllowed=true" "post-install-ui-preflight-missing-safari-allow"
require_output "$post_install_ui_preflight_output" "postInstallUiPreflightSelfCheck=true" "post-install-ui-preflight-self-check-failed"

section "debug event log lifecycle self-check"
provided_event_log="/tmp/inputia-debug-event-provided.$$"
generated_event_log="/tmp/inputia-debug-event-generated.$$"
VERIFY_TEMP_FILES+=("$provided_event_log" "$generated_event_log")
/usr/bin/printf 'previous-content' >"$provided_event_log"
provided_inode_before="$(/usr/bin/stat -f '%i' "$provided_event_log")"
inputia_prepare_debug_event_log "$provided_event_log" "provided"
inputia_assert_debug_event_log_clean "$provided_event_log" "debugEventLogLifecycleReady" 31
provided_inode_after="$(/usr/bin/stat -f '%i' "$provided_event_log")"
provided_size_after="$(/usr/bin/stat -f '%z' "$provided_event_log")"
echo "debugEventLogProvidedInodeBefore=$provided_inode_before"
echo "debugEventLogProvidedInodeAfter=$provided_inode_after"
echo "debugEventLogProvidedSizeAfter=$provided_size_after"
if [[ "$provided_inode_after" != "$provided_inode_before" || "$provided_size_after" != "0" ]]; then
  echo "nonGuiVerificationPassed=false reason=debug-event-log-provided-lifecycle"
  /bin/rm -f "$provided_event_log" "$generated_event_log"
  exit 1
fi
/usr/bin/printf 'old-temp-content' >"$generated_event_log"
inputia_prepare_debug_event_log "$generated_event_log" ""
if [[ -e "$generated_event_log" ]]; then
  echo "nonGuiVerificationPassed=false reason=debug-event-log-generated-lifecycle"
  /bin/rm -f "$provided_event_log" "$generated_event_log"
  exit 1
fi
/usr/bin/printf 'stale-event' >"$generated_event_log"
set +e
debug_event_dirty_output="$(
  /bin/bash -c 'source "$1"; inputia_assert_debug_event_log_clean "$2" "debugEventLogLifecycleReady" 32' \
    _ "$ROOT_DIR/smoke-common.sh" "$generated_event_log" 2>&1
)"
debug_event_dirty_rc=$?
set -e
printf '%s\n' "$debug_event_dirty_output" | /usr/bin/sed 's/^/debugEventLogDirty: /'
if [[ "$debug_event_dirty_rc" != "32" ]]; then
  echo "nonGuiVerificationPassed=false reason=debug-event-log-dirty-rc actual=$debug_event_dirty_rc"
  /bin/rm -f "$provided_event_log" "$generated_event_log"
  exit 1
fi
require_output "$debug_event_dirty_output" "debugEventLogClean=false" "debug-event-log-dirty-marker-missing"
require_output "$debug_event_dirty_output" "debugEventLogLifecycleReady=false reason=debug-event-log-not-clean" "debug-event-log-dirty-ready-marker-missing"
/bin/rm -f "$provided_event_log" "$generated_event_log"
echo "debugEventLogLifecycleSelfCheck=true"

section "textedit cleanup self-check"
textedit_cleanup_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
textedit_cleanup_tis_before="$(current_input_source_id)"
textedit_cleanup_debug_before="$(debug_events_env)"
run_expect_rc 27 "textEditCleanupSelfCheck" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 \
    INPUTIA_TEXTEDIT_CLEANUP_SELF_CHECK=1 \
    "$ROOT_DIR/smoke-textedit.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "textEditCleanupSelfCheck=true phase=after-temp-write" "textedit-cleanup-self-check-marker-missing"
textedit_cleanup_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
textedit_cleanup_tis_after="$(current_input_source_id)"
textedit_cleanup_debug_after="$(debug_events_env)"
if [[ "$textedit_cleanup_after" != "$textedit_cleanup_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=textedit-cleanup-self-check-mutated-clipboard"
  exit 1
fi
echo "textEditCleanupSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "textEditCleanupSelfCheck" "$textedit_cleanup_tis_before" "$textedit_cleanup_tis_after"
assert_debug_env_unchanged "textEditCleanupSelfCheck" "$textedit_cleanup_debug_before" "$textedit_cleanup_debug_after"
assert_no_user_host "textEditCleanupSelfCheck"
textedit_cleanup_residue="$(find "$TMP_RESIDUE_ROOT" -maxdepth 1 -name 'inputia-textedit-*' ! -name 'inputia-textedit-command-*' -print 2>/dev/null | sort || true)"
if [[ -n "$textedit_cleanup_residue" ]]; then
  echo "nonGuiVerificationPassed=false reason=textedit-cleanup-self-check-left-temp"
  printf '%s\n' "$textedit_cleanup_residue"
  exit 1
fi
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "textedit-cleanup-self-check-launched-textedit"
fi
assert_process_not_running osascript "textedit-cleanup-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "textedit-cleanup-self-check-left-inputia-host"
echo "textEditCleanupSelfCheckNoMutationPassed=true"

section "safari typing cleanup self-check"
safari_typing_cleanup_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_typing_cleanup_tis_before="$(current_input_source_id)"
safari_typing_cleanup_debug_before="$(debug_events_env)"
run_expect_rc 28 "safariTypingCleanupSelfCheck" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
    INPUTIA_SAFARI_TYPING_CLEANUP_SELF_CHECK=1 \
    "$ROOT_DIR/smoke-safari-typing.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "safariTypingCleanupSelfCheck=true phase=after-temp-write" "safari-typing-cleanup-self-check-marker-missing"
safari_typing_cleanup_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_typing_cleanup_tis_after="$(current_input_source_id)"
safari_typing_cleanup_debug_after="$(debug_events_env)"
if [[ "$safari_typing_cleanup_clipboard_after" != "$safari_typing_cleanup_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-typing-cleanup-self-check-mutated-clipboard"
  exit 1
fi
echo "safariTypingCleanupSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "safariTypingCleanupSelfCheck" "$safari_typing_cleanup_tis_before" "$safari_typing_cleanup_tis_after"
assert_debug_env_unchanged "safariTypingCleanupSelfCheck" "$safari_typing_cleanup_debug_before" "$safari_typing_cleanup_debug_after"
assert_no_user_host "safariTypingCleanupSelfCheck"
safari_typing_cleanup_residue="$(find "$TMP_RESIDUE_ROOT" -maxdepth 1 -name 'inputia-safari-typing-*' -print 2>/dev/null | sort || true)"
if [[ -n "$safari_typing_cleanup_residue" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-typing-cleanup-self-check-left-temp"
  printf '%s\n' "$safari_typing_cleanup_residue"
  exit 1
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "safari-typing-cleanup-self-check-launched-safari"
fi
assert_process_not_running osascript "safari-typing-cleanup-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "safari-typing-cleanup-self-check-left-inputia-host"
echo "safariTypingCleanupSelfCheckNoMutationPassed=true"

section "safari enter cleanup self-check"
safari_enter_cleanup_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_enter_cleanup_tis_before="$(current_input_source_id)"
safari_enter_cleanup_debug_before="$(debug_events_env)"
run_expect_rc 26 "safariEnterCleanupSelfCheck" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
    INPUTIA_SAFARI_ENTER_CLEANUP_SELF_CHECK=1 \
    "$ROOT_DIR/smoke-safari-enter.sh" "$BUILD_APP"
require_output_regex "$RUN_EXPECT_RC_OUTPUT" "safariEnterCleanupSelfCheck=true phase=after-temp-write\\+(debug-env-write|launchctl-env-unavailable)" "safari-enter-cleanup-self-check-marker-missing"
safari_enter_cleanup_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_enter_cleanup_tis_after="$(current_input_source_id)"
safari_enter_cleanup_debug_after="$(debug_events_env)"
if [[ "$safari_enter_cleanup_clipboard_after" != "$safari_enter_cleanup_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-enter-cleanup-self-check-mutated-clipboard"
  exit 1
fi
echo "safariEnterCleanupSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "safariEnterCleanupSelfCheck" "$safari_enter_cleanup_tis_before" "$safari_enter_cleanup_tis_after"
assert_debug_env_unchanged "safariEnterCleanupSelfCheck" "$safari_enter_cleanup_debug_before" "$safari_enter_cleanup_debug_after"
assert_no_user_host "safariEnterCleanupSelfCheck"
safari_enter_cleanup_residue="$(find "$TMP_RESIDUE_ROOT" -maxdepth 1 -name 'inputia-safari-enter*' -print 2>/dev/null | sort || true)"
if [[ -n "$safari_enter_cleanup_residue" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-enter-cleanup-self-check-left-temp"
  printf '%s\n' "$safari_enter_cleanup_residue"
  exit 1
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "safari-enter-cleanup-self-check-launched-safari"
fi
assert_process_not_running osascript "safari-enter-cleanup-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "safari-enter-cleanup-self-check-left-inputia-host"
echo "safariEnterCleanupSelfCheckNoMutationPassed=true"

section "safari enter debug log prepare failure gate"
safari_enter_debug_log_dir="$(/usr/bin/mktemp -d /tmp/inputia-safari-enter-dirty-log.XXXXXX)"
VERIFY_TEMP_DIRS+=("$safari_enter_debug_log_dir")
safari_enter_debug_log_gate_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_enter_debug_log_gate_tis_before="$(current_input_source_id)"
safari_enter_debug_log_gate_debug_before="$(debug_events_env)"
run_expect_rc 1 "safariEnterDebugLogPrepareGate" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
    INPUTIA_DEBUG_EVENTS="$safari_enter_debug_log_dir" \
    "$ROOT_DIR/smoke-safari-enter.sh" "$BUILD_APP" 2>&1
require_output "$RUN_EXPECT_RC_OUTPUT" "debugEventLogPrepare=false" "safari-enter-debug-log-prepare-marker-missing"
require_output "$RUN_EXPECT_RC_OUTPUT" "reason=not-regular-file" "safari-enter-debug-log-prepare-reason-missing"
if printf '%s\n' "$RUN_EXPECT_RC_OUTPUT" | /usr/bin/grep -q 'previousInputSourceID='; then
  echo "nonGuiVerificationPassed=false reason=safari-enter-debug-log-prepare-gate-selected-input-source"
  exit 1
fi
safari_enter_debug_log_gate_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_enter_debug_log_gate_tis_after="$(current_input_source_id)"
safari_enter_debug_log_gate_debug_after="$(debug_events_env)"
if [[ "$safari_enter_debug_log_gate_clipboard_after" != "$safari_enter_debug_log_gate_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-enter-debug-log-prepare-gate-mutated-clipboard"
  exit 1
fi
echo "safariEnterDebugLogPrepareGate.clipboardUnchanged=true"
assert_current_source_unchanged "safariEnterDebugLogPrepareGate" "$safari_enter_debug_log_gate_tis_before" "$safari_enter_debug_log_gate_tis_after"
assert_debug_env_unchanged "safariEnterDebugLogPrepareGate" "$safari_enter_debug_log_gate_debug_before" "$safari_enter_debug_log_gate_debug_after"
assert_no_user_host "safariEnterDebugLogPrepareGate"
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "safari-enter-debug-log-prepare-gate-launched-safari"
fi
assert_process_not_running osascript "safari-enter-debug-log-prepare-gate-left-osascript"
assert_process_not_running InputiaInputMethod "safari-enter-debug-log-prepare-gate-left-inputia-host"
echo "safariEnterDebugLogPrepareGateNoMutationPassed=true"

section "textedit command cleanup self-check"
textedit_command_cleanup_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
textedit_command_cleanup_tis_before="$(current_input_source_id)"
textedit_command_cleanup_debug_before="$(debug_events_env)"
run_expect_rc 25 "textEditCommandCleanupSelfCheck" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 \
    INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
    INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="Unicode text, 42" \
    INPUTIA_TEXTEDIT_COMMAND_CLEANUP_SELF_CHECK=1 \
    "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$BUILD_APP"
require_output_regex "$RUN_EXPECT_RC_OUTPUT" "textEditCommandCleanupSelfCheck=true phase=(after-clipboard-write|pasteboard-unavailable)" "textedit-command-cleanup-self-check-marker-missing"
textedit_command_cleanup_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
textedit_command_cleanup_tis_after="$(current_input_source_id)"
textedit_command_cleanup_debug_after="$(debug_events_env)"
if [[ "$textedit_command_cleanup_after" != "$textedit_command_cleanup_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=textedit-command-cleanup-self-check-mutated-clipboard"
  exit 1
fi
echo "textEditCommandCleanupSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "textEditCommandCleanupSelfCheck" "$textedit_command_cleanup_tis_before" "$textedit_command_cleanup_tis_after"
assert_debug_env_unchanged "textEditCommandCleanupSelfCheck" "$textedit_command_cleanup_debug_before" "$textedit_command_cleanup_debug_after"
assert_no_user_host "textEditCommandCleanupSelfCheck"
textedit_command_cleanup_residue="$(find "$TMP_RESIDUE_ROOT" -maxdepth 1 -name 'inputia-textedit-command-*' -print 2>/dev/null | sort || true)"
if [[ -n "$textedit_command_cleanup_residue" ]]; then
  echo "nonGuiVerificationPassed=false reason=textedit-command-cleanup-self-check-left-temp"
  printf '%s\n' "$textedit_command_cleanup_residue"
  exit 1
fi
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "textedit-command-cleanup-self-check-launched-textedit"
fi
assert_process_not_running osascript "textedit-command-cleanup-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "textedit-command-cleanup-self-check-left-inputia-host"
echo "textEditCommandCleanupSelfCheckNoMutationPassed=true"

section "clipboard recall cleanup self-check"
clipboard_cleanup_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
clipboard_cleanup_tis_before="$(current_input_source_id)"
clipboard_cleanup_debug_before="$(debug_events_env)"
run_expect_rc 23 "clipboardRecallCleanupSelfCheck" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
    INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="Unicode text, 42" \
    INPUTIA_CLIPBOARD_RECALL_CLEANUP_SELF_CHECK=1 \
    "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP"
require_output_regex "$RUN_EXPECT_RC_OUTPUT" "clipboardRecallCleanupSelfCheck=true phase=(after-clipboard-write|pasteboard-unavailable)\\+(debug-env-write|launchctl-env-unavailable)" "clipboard-cleanup-self-check-marker-missing"
clipboard_cleanup_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
clipboard_cleanup_tis_after="$(current_input_source_id)"
clipboard_cleanup_debug_after="$(debug_events_env)"
if [[ "$clipboard_cleanup_after" != "$clipboard_cleanup_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=clipboard-cleanup-self-check-mutated-clipboard"
  exit 1
fi
echo "clipboardRecallCleanupSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "clipboardRecallCleanupSelfCheck" "$clipboard_cleanup_tis_before" "$clipboard_cleanup_tis_after"
assert_debug_env_unchanged "clipboardRecallCleanupSelfCheck" "$clipboard_cleanup_debug_before" "$clipboard_cleanup_debug_after"
assert_no_user_host "clipboardRecallCleanupSelfCheck"
clipboard_cleanup_residue="$(find "$TMP_RESIDUE_ROOT" -maxdepth 1 -name 'inputia-clipboard-recall-*' -print 2>/dev/null | sort || true)"
if [[ -n "$clipboard_cleanup_residue" ]]; then
  echo "nonGuiVerificationPassed=false reason=clipboard-cleanup-self-check-left-temp"
  printf '%s\n' "$clipboard_cleanup_residue"
  exit 1
fi
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "clipboard-cleanup-self-check-launched-textedit"
fi
assert_process_not_running osascript "clipboard-cleanup-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "clipboard-cleanup-self-check-left-inputia-host"
echo "clipboardRecallCleanupSelfCheckNoMutationPassed=true"

section "clipboard recall debug log prepare failure gate"
clipboard_debug_log_dir="$(/usr/bin/mktemp -d /tmp/inputia-clipboard-dirty-log.XXXXXX)"
VERIFY_TEMP_DIRS+=("$clipboard_debug_log_dir")
clipboard_debug_log_gate_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
clipboard_debug_log_gate_tis_before="$(current_input_source_id)"
clipboard_debug_log_gate_debug_before="$(debug_events_env)"
run_expect_rc 1 "clipboardDebugLogPrepareGate" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
    INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="Unicode text, 42" \
    INPUTIA_DEBUG_EVENTS="$clipboard_debug_log_dir" \
    "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP" 2>&1
require_output "$RUN_EXPECT_RC_OUTPUT" "debugEventLogPrepare=false" "clipboard-debug-log-prepare-marker-missing"
require_output "$RUN_EXPECT_RC_OUTPUT" "reason=not-regular-file" "clipboard-debug-log-prepare-reason-missing"
if printf '%s\n' "$RUN_EXPECT_RC_OUTPUT" | /usr/bin/grep -q 'previousInputSourceID='; then
  echo "nonGuiVerificationPassed=false reason=clipboard-debug-log-prepare-gate-selected-input-source"
  exit 1
fi
clipboard_debug_log_gate_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
clipboard_debug_log_gate_tis_after="$(current_input_source_id)"
clipboard_debug_log_gate_debug_after="$(debug_events_env)"
if [[ "$clipboard_debug_log_gate_clipboard_after" != "$clipboard_debug_log_gate_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=clipboard-debug-log-prepare-gate-mutated-clipboard"
  exit 1
fi
echo "clipboardDebugLogPrepareGate.clipboardUnchanged=true"
assert_current_source_unchanged "clipboardDebugLogPrepareGate" "$clipboard_debug_log_gate_tis_before" "$clipboard_debug_log_gate_tis_after"
assert_debug_env_unchanged "clipboardDebugLogPrepareGate" "$clipboard_debug_log_gate_debug_before" "$clipboard_debug_log_gate_debug_after"
assert_no_user_host "clipboardDebugLogPrepareGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "clipboard-debug-log-prepare-gate-launched-textedit"
fi
assert_process_not_running osascript "clipboard-debug-log-prepare-gate-left-osascript"
assert_process_not_running InputiaInputMethod "clipboard-debug-log-prepare-gate-left-inputia-host"
echo "clipboardDebugLogPrepareGateNoMutationPassed=true"

section "safari command cleanup self-check"
safari_command_cleanup_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_command_cleanup_tis_before="$(current_input_source_id)"
safari_command_cleanup_debug_before="$(debug_events_env)"
run_expect_rc 24 "safariCommandCleanupSelfCheck" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
    INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
    INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
    INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="Unicode text, 42" \
    INPUTIA_SAFARI_COMMAND_CLEANUP_SELF_CHECK=1 \
    "$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$BUILD_APP"
require_output_regex "$RUN_EXPECT_RC_OUTPUT" "safariCommandCleanupSelfCheck=true phase=(after-clipboard-write|pasteboard-unavailable)" "safari-command-cleanup-self-check-marker-missing"
safari_command_cleanup_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_command_cleanup_tis_after="$(current_input_source_id)"
safari_command_cleanup_debug_after="$(debug_events_env)"
if [[ "$safari_command_cleanup_after" != "$safari_command_cleanup_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-command-cleanup-self-check-mutated-clipboard"
  exit 1
fi
echo "safariCommandCleanupSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "safariCommandCleanupSelfCheck" "$safari_command_cleanup_tis_before" "$safari_command_cleanup_tis_after"
assert_debug_env_unchanged "safariCommandCleanupSelfCheck" "$safari_command_cleanup_debug_before" "$safari_command_cleanup_debug_after"
assert_no_user_host "safariCommandCleanupSelfCheck"
safari_command_cleanup_residue="$(find "$TMP_RESIDUE_ROOT" -maxdepth 1 -name 'inputia-safari-command-*' -print 2>/dev/null | sort || true)"
if [[ -n "$safari_command_cleanup_residue" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-command-cleanup-self-check-left-temp"
  printf '%s\n' "$safari_command_cleanup_residue"
  exit 1
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "safari-command-cleanup-self-check-launched-safari"
fi
assert_process_not_running osascript "safari-command-cleanup-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "safari-command-cleanup-self-check-left-inputia-host"
echo "safariCommandCleanupSelfCheckNoMutationPassed=true"

section "safari diagnose cleanup self-check"
safari_diagnose_cleanup_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_diagnose_cleanup_tis_before="$(current_input_source_id)"
safari_diagnose_cleanup_debug_before="$(debug_events_env)"
run_expect_rc 29 "safariDiagnoseCleanupSelfCheck" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
    INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
    INPUTIA_SAFARI_DIAGNOSE_CLEANUP_SELF_CHECK=1 \
    "$ROOT_DIR/diagnose-safari-input-source.sh" "$BUILD_APP"
require_output "$RUN_EXPECT_RC_OUTPUT" "safariDiagnoseCleanupSelfCheck=true phase=after-temp-write" "safari-diagnose-cleanup-self-check-marker-missing"
safari_diagnose_cleanup_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_diagnose_cleanup_tis_after="$(current_input_source_id)"
safari_diagnose_cleanup_debug_after="$(debug_events_env)"
if [[ "$safari_diagnose_cleanup_after" != "$safari_diagnose_cleanup_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-diagnose-cleanup-self-check-mutated-clipboard"
  exit 1
fi
echo "safariDiagnoseCleanupSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "safariDiagnoseCleanupSelfCheck" "$safari_diagnose_cleanup_tis_before" "$safari_diagnose_cleanup_tis_after"
assert_debug_env_unchanged "safariDiagnoseCleanupSelfCheck" "$safari_diagnose_cleanup_debug_before" "$safari_diagnose_cleanup_debug_after"
assert_no_user_host "safariDiagnoseCleanupSelfCheck"
safari_diagnose_cleanup_residue="$(
  find "$TMP_RESIDUE_ROOT" -maxdepth 1 \
    \( -name 'inputia-safari-input-source-test.*' \
    -o -name 'inputia-hitoolbox-preference.*' \
    -o -name 'inputia-safari-source-select.*' \
    -o -name 'inputia-safari-focused-select.*' \
    -o -name 'inputia-safari-diagnose-*' \) \
    -print 2>/dev/null | sort || true
)"
if [[ -n "$safari_diagnose_cleanup_residue" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-diagnose-cleanup-self-check-left-temp"
  printf '%s\n' "$safari_diagnose_cleanup_residue"
  exit 1
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "safari-diagnose-cleanup-self-check-launched-safari"
fi
assert_process_not_running osascript "safari-diagnose-cleanup-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "safari-diagnose-cleanup-self-check-left-inputia-host"
echo "safariDiagnoseCleanupSelfCheckNoMutationPassed=true"

section "clipboard info classifier self-check"
clipboard_classifier_case() {
  local label="$1"
  local info="$2"
  local expected_reason="$3"
  local expected_rc="$4"
  local actual_reason actual_rc

  set +e
  actual_reason="$(inputia_clipboard_info_restorable_reason "$info")"
  actual_rc=$?
  set -e

  echo "clipboardClassifierCase=$label reason=$actual_reason rc=$actual_rc"
  if [[ "$actual_reason" != "$expected_reason" || "$actual_rc" != "$expected_rc" ]]; then
    echo "nonGuiVerificationPassed=false reason=clipboard-classifier-$label expectedReason=$expected_reason actualReason=$actual_reason expectedRc=$expected_rc actualRc=$actual_rc"
    exit 1
  fi
}

clipboard_classifier_case "utf8-text" "«class utf8», 12, «class ut16», 22, string, 11, Unicode text, 22" "text-restorable" 0
clipboard_classifier_case "plain-string" "string, 11" "text-restorable" 0
clipboard_classifier_case "rtf-plus-text" "«class RTF », 512, string, 11, Unicode text, 22" "non-text-clipboard" 1
clipboard_classifier_case "tiff-plus-text" "TIFF picture, 2048, string, 11, Unicode text, 22" "non-text-clipboard" 1
clipboard_classifier_case "pdf-plus-text" "PDF , 2048, string, 11, Unicode text, 22" "non-text-clipboard" 1
clipboard_classifier_case "file-url-plus-text" "file URL, 80, string, 11, Unicode text, 22" "non-text-clipboard" 1
clipboard_classifier_case "empty-info" "" "missing-text-clipboard" 2
clipboard_classifier_case "image-only" "JPEG picture, 2048" "missing-text-clipboard" 2
echo "clipboardInfoClassifierSelfCheck=true"

section "clipboard info override gate self-check"
clipboard_real_info="$(inputia_current_clipboard_info)"
clipboard_override_disabled_info="$(
  INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="override-without-gate" inputia_current_clipboard_info
)"
clipboard_override_enabled_info="$(
  INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
    INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="override-with-gate" \
    inputia_current_clipboard_info
)"
echo "clipboardInfoOverrideDisabledIgnored=$([[ "$clipboard_override_disabled_info" != "override-without-gate" ]] && echo true || echo false)"
echo "clipboardInfoOverrideEnabledValue=$clipboard_override_enabled_info"
if [[ "$clipboard_override_disabled_info" == "override-without-gate" ]]; then
  echo "nonGuiVerificationPassed=false reason=clipboard-info-override-without-gate"
  exit 1
fi
if [[ "$clipboard_override_enabled_info" != "override-with-gate" ]]; then
  echo "nonGuiVerificationPassed=false reason=clipboard-info-override-gate-ignored"
  exit 1
fi
if [[ "$clipboard_override_disabled_info" != "$clipboard_real_info" ]]; then
  echo "nonGuiVerificationPassed=false reason=clipboard-info-override-disabled-mutated-provider"
  exit 1
fi
echo "clipboardInfoOverrideGateSelfCheck=true"

section "cleanup trap status self-check"
set +e
cleanup_success_failure_output="$(/bin/bash -c 'set -e; cleanup(){ echo cleanup-status-return-1; return 1; }; trap cleanup EXIT; echo body-ok; true' 2>&1)"
cleanup_success_failure_rc=$?
cleanup_body_failure_output="$(/bin/bash -c 'set -e; cleanup(){ echo cleanup-status-return-0; return 0; }; trap cleanup EXIT; echo body-fail; false' 2>&1)"
cleanup_body_failure_rc=$?
set -e
printf '%s\n' "$cleanup_success_failure_output" | /usr/bin/sed 's/^/cleanupTrapSuccessFailure: /'
echo "cleanupTrapSuccessFailure.rc=$cleanup_success_failure_rc"
printf '%s\n' "$cleanup_body_failure_output" | /usr/bin/sed 's/^/cleanupTrapBodyFailure: /'
echo "cleanupTrapBodyFailure.rc=$cleanup_body_failure_rc"
if [[ "$cleanup_success_failure_rc" == "0" ]]; then
  echo "nonGuiVerificationPassed=false reason=cleanup-trap-failure-not-fatal"
  exit 1
fi
if [[ "$cleanup_body_failure_rc" == "0" ]]; then
  echo "nonGuiVerificationPassed=false reason=cleanup-trap-body-failure-masked"
  exit 1
fi
echo "cleanupTrapStatusSelfCheck=true"

section "timeout helper self-check"
set +e
timeout_fast_output="$(inputia_run_with_timeout timeout-fast-check 5 /bin/echo timeout-fast-ok 2>&1)"
timeout_fast_rc=$?
timeout_fail_output="$(inputia_run_with_timeout timeout-fail-check 5 /bin/sh -c 'echo timeout-fail-stderr >&2; exit 7' 2>&1)"
timeout_fail_rc=$?
timeout_output="$(inputia_run_with_timeout timeout-self-check 1 /bin/sleep 5 2>&1)"
timeout_rc=$?
timeout_child_output="$(inputia_run_with_timeout timeout-child-process-group-check 1 /bin/sh -c 'sleep 20 & echo timeoutChildPid=$!; wait' 2>&1)"
timeout_child_rc=$?
set -e
printf '%s\n' "$timeout_fast_output" | /usr/bin/sed 's/^/timeoutFastCheck: /'
echo "timeoutFastCheck.rc=$timeout_fast_rc"
printf '%s\n' "$timeout_fail_output" | /usr/bin/sed 's/^/timeoutFailCheck: /'
echo "timeoutFailCheck.rc=$timeout_fail_rc"
printf '%s\n' "$timeout_output" | /usr/bin/sed 's/^/timeoutSelfCheck: /'
echo "timeoutSelfCheck.rc=$timeout_rc"
printf '%s\n' "$timeout_child_output" | /usr/bin/sed 's/^/timeoutChildProcessGroupCheck: /'
echo "timeoutChildProcessGroupCheck.rc=$timeout_child_rc"
if [[ "$timeout_fast_rc" != "0" || "$timeout_fast_output" != "timeout-fast-ok" ]]; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-fast-path"
  exit 1
fi
if [[ "$timeout_fail_rc" != "7" ]]; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-fail-rc"
  exit 1
fi
if ! /usr/bin/grep -q 'timeout-fail-stderr' <<<"$timeout_fail_output"; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-fail-output"
  exit 1
fi
if [[ "$timeout_rc" == "0" ]]; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-zero-rc"
  exit 1
fi
if ! /usr/bin/grep -q 'inputiaSmokeTimeout=timeout-self-check seconds=1' <<<"$timeout_output"; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-missing-marker"
  exit 1
fi
if [[ "$timeout_child_rc" == "0" ]]; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-child-zero-rc"
  exit 1
fi
if ! /usr/bin/grep -q 'inputiaSmokeTimeout=timeout-child-process-group-check seconds=1' <<<"$timeout_child_output"; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-child-missing-marker"
  exit 1
fi
timeout_child_pid="$(/usr/bin/sed -n 's/^timeoutChildPid=//p' <<<"$timeout_child_output" | /usr/bin/tail -1)"
if [[ -z "$timeout_child_pid" ]]; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-child-missing-pid"
  exit 1
fi
timeout_child_waited=0
while /bin/ps -p "$timeout_child_pid" -o comm= 2>/dev/null | /usr/bin/grep -q 'sleep' &&
  ((timeout_child_waited < 50)); do
  /bin/sleep 0.1
  timeout_child_waited=$((timeout_child_waited + 1))
done
if /bin/ps -p "$timeout_child_pid" -o comm= 2>/dev/null | /usr/bin/grep -q 'sleep'; then
  echo "nonGuiVerificationPassed=false reason=timeout-helper-left-child pid=$timeout_child_pid waitedTicks=$timeout_child_waited"
  /bin/kill -9 "$timeout_child_pid" >/dev/null 2>&1 || true
  exit 1
fi
echo "timeoutChildProcessGroupCleaned=true"
echo "timeoutHelperSelfCheck=true"

section "applescript compile"
if [[ "${INPUTIA_VERIFY_APPLESCRIPT_COMPILE:-0}" == "1" &&
  "${INPUTIA_ALLOW_APPLESCRIPT_COMPILE_APP_LAUNCH:-0}" == "1" ]]; then
	compile_quoted_applescript_blocks "smoke-common" "$ROOT_DIR/smoke-common.sh"
	compile_quoted_applescript_blocks "smoke-textedit" "$ROOT_DIR/smoke-textedit.sh"
	compile_quoted_applescript_blocks "smoke-textedit-command-shortcuts" "$ROOT_DIR/smoke-textedit-command-shortcuts.sh"
	compile_quoted_applescript_blocks "smoke-clipboard-recall" "$ROOT_DIR/smoke-clipboard-recall.sh"
	compile_safari_applescript_block "smoke-safari-typing" "$ROOT_DIR/smoke-safari-typing.sh"
	compile_safari_applescript_block "smoke-safari-command-shortcuts" "$ROOT_DIR/smoke-safari-command-shortcuts.sh"
	compile_safari_applescript_block "smoke-safari-enter" "$ROOT_DIR/smoke-safari-enter.sh"
elif [[ "${INPUTIA_VERIFY_APPLESCRIPT_COMPILE:-0}" == "1" ]]; then
  echo "appleScriptCompileSkipped=true reason=osacompile-may-launch-target-apps"
else
  echo "appleScriptCompileSkipped=true reason=would-launch-target-apps"
fi

section "package"
run_and_prefix "verifyPkg: " "$ROOT_DIR/verify-pkg.sh"

section "status"
status_output="$("$ROOT_DIR/status.sh" 2>&1)"
status_waits=0
status_max_wait="${INPUTIA_PROCESS_WAIT_TICKS:-100}"
while ! /usr/bin/grep -q 'running=false' <<<"$status_output" && ((status_waits < status_max_wait)); do
  /bin/sleep 0.1
  status_waits=$((status_waits + 1))
  status_output="$("$ROOT_DIR/status.sh" 2>&1)"
done
if ! /usr/bin/grep -q 'running=false' <<<"$status_output"; then
  echo "nonGuiVerificationPassed=false reason=inputia-host-running waitedTicks=$status_waits"
  process_details InputiaInputMethod
  exit 1
fi
printf '%s\n' "$status_output" | /usr/bin/awk '
  /^buildVersion=|^buildCDHash=|^systemMatchesBuild=|^userMatchesBuild=|^userHostConflict=|^includeAllInstalled=|^matches=|^running=|^sha256=|^statusAdminInstallReady=|^statusTISEnabledMatches=|^statusTISInstalledMatches=|^statusTISCurrentMatchesTarget=|^statusLegacyHIToolboxInputiaEnabled=|^statusLegacyHIToolboxInputiaSelected=|^statusStaleHIToolboxEnabledStateSuspected=|^statusSignatureAccepted=|^statusMenuReadiness=|^statusMenuBlockReason=|^statusUserHostConflict=|^statusGuiSessionBlockReason=|^statusTextEditPreflight=|^statusSafariPreflight=|^statusInputiaHostPreflight=|^statusGuiSmokeBlockReasons=|^statusGuiSmokeReady=/ { print "status: " $0 }
'
expected_status_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$ROOT_DIR/Info.plist")"
require_output "$status_output" "buildVersion=$expected_status_version" "status-build-version-missing"
require_output "$status_output" 'statusUserHostConflict=false' "status-user-host-conflict-summary-missing-current-clean"
status_gui_session_reason="$(/usr/bin/awk -F= '$1 == "statusGuiSessionBlockReason" { print $2; found = 1; exit } END { if (!found) print "" }' <<<"$status_output")"
if [[ -z "$status_gui_session_reason" ]]; then
  echo "nonGuiVerificationPassed=false reason=status-gui-session-summary-missing"
  exit 1
fi
if [[ "$status_gui_session_reason" != "none" ]]; then
  require_status_block_reason "$status_output" "$status_gui_session_reason" "status-gui-session-block-reason-missing-current-blockers"
fi
require_output "$status_output" 'statusTextEditPreflight=not-running' "status-textedit-preflight-summary-missing-current-idle"
require_output "$status_output" 'statusSafariPreflight=not-running' "status-safari-preflight-summary-missing-current-idle"
require_output "$status_output" 'statusInputiaHostPreflight=not-running' "status-inputia-host-preflight-summary-missing-current-idle"
require_signature_block_if_rejected "$status_output" "status-current-missing-signature-rejected"
require_output "$status_output" 'statusGuiSmokeReady=false reason=' "status-gui-smoke-ready-summary-missing-current-blockers"

section "status GUI blocker self-check"
status_blocker_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
status_blocker_tis_before="$(current_input_source_id)"
status_blocker_debug_before="$(debug_events_env)"
status_session_output="$(INPUTIA_GUI_SESSION_BLOCK_REASON_FOR_TEST=screen-locked "$ROOT_DIR/status.sh" 2>&1)"
require_output "$status_session_output" 'statusGuiSessionBlockReason=screen-locked' "status-gui-session-blocker-self-check-missing-session-reason"
require_signature_block_if_rejected "$status_session_output" "status-gui-session-blocker-self-check-missing-signature-rejected"
require_status_block_reason "$status_session_output" "screen-locked" "status-gui-session-blocker-self-check-missing-screen-locked"
echo "statusGuiSessionBlockerSelfCheck=true"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  start_fake_existing_process TextEdit
  fake_status_textedit_pid="$INPUTIA_FAKE_EXISTING_PID"
  status_textedit_output="$("$ROOT_DIR/status.sh" 2>&1)"
  require_output "$status_textedit_output" 'statusTextEditPreflight=running' "status-textedit-blocker-self-check-missing-running"
  require_signature_block_if_rejected "$status_textedit_output" "status-textedit-blocker-self-check-missing-signature-rejected"
  require_status_block_reason "$status_textedit_output" "textedit-already-running" "status-textedit-blocker-self-check-missing-textedit-already-running"
  status_textedit_allow_output="$(INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 "$ROOT_DIR/status.sh" 2>&1)"
  require_output "$status_textedit_allow_output" 'statusTextEditPreflight=running' "status-textedit-allow-self-check-missing-running"
  require_signature_block_if_rejected "$status_textedit_allow_output" "status-textedit-allow-self-check-missing-signature-rejected"
  if /usr/bin/grep -q 'textedit-already-running' <<<"$status_textedit_allow_output"; then
    echo "nonGuiVerificationPassed=false reason=status-textedit-allow-still-blocked"
    exit 1
  fi
  stop_fake_existing_process TextEdit "$fake_status_textedit_pid"
  echo "statusTextEditBlockerSelfCheck=true"
  echo "statusTextEditAllowSelfCheck=true"
else
  echo "statusTextEditBlockerSelfCheck=skipped reason=textedit-preexisting"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  start_fake_existing_process Safari
  fake_status_safari_pid="$INPUTIA_FAKE_EXISTING_PID"
  status_safari_output="$("$ROOT_DIR/status.sh" 2>&1)"
  require_output "$status_safari_output" 'statusSafariPreflight=running' "status-safari-blocker-self-check-missing-running"
  require_signature_block_if_rejected "$status_safari_output" "status-safari-blocker-self-check-missing-signature-rejected"
  require_status_block_reason "$status_safari_output" "safari-already-running" "status-safari-blocker-self-check-missing-safari-already-running"
  status_safari_allow_output="$(INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 "$ROOT_DIR/status.sh" 2>&1)"
  require_output "$status_safari_allow_output" 'statusSafariPreflight=running' "status-safari-allow-self-check-missing-running"
  require_signature_block_if_rejected "$status_safari_allow_output" "status-safari-allow-self-check-missing-signature-rejected"
  if /usr/bin/grep -q 'safari-already-running' <<<"$status_safari_allow_output"; then
    echo "nonGuiVerificationPassed=false reason=status-safari-allow-still-blocked"
    exit 1
  fi
  stop_fake_existing_process Safari "$fake_status_safari_pid"
  echo "statusSafariBlockerSelfCheck=true"
  echo "statusSafariAllowSelfCheck=true"
else
  echo "statusSafariBlockerSelfCheck=skipped reason=safari-preexisting"
fi
start_fake_existing_process InputiaInputMethod
fake_status_inputia_pid="$INPUTIA_FAKE_EXISTING_PID"
status_inputia_output="$("$ROOT_DIR/status.sh" 2>&1)"
require_output "$status_inputia_output" 'statusInputiaHostPreflight=running' "status-inputia-host-blocker-self-check-missing-running"
require_signature_block_if_rejected "$status_inputia_output" "status-inputia-host-blocker-self-check-missing-signature-rejected"
require_status_block_reason "$status_inputia_output" "inputia-host-running" "status-inputia-host-blocker-self-check-missing-inputia-host-running"
require_output "$status_inputia_output" 'statusGuiSmokeReady=false reason=' "status-inputia-host-blocker-self-check-missing-ready-reason"
stop_fake_existing_process InputiaInputMethod "$fake_status_inputia_pid"
echo "statusInputiaHostBlockerSelfCheck=true"
status_user_host_root="$(/usr/bin/mktemp -d "/tmp/inputia-status-user-host.XXXXXX")"
VERIFY_TEMP_DIRS+=("$status_user_host_root")
status_fake_user_app="$status_user_host_root/InputiaInputMethod.app"
status_fake_system_app="$status_user_host_root/SystemInputiaInputMethod.app"
/bin/mkdir -p "$status_fake_user_app"
/bin/mkdir -p "$status_fake_system_app"
status_user_host_output="$(
  INPUTIA_USER_APP_FOR_TEST="$status_fake_user_app" \
    INPUTIA_SYSTEM_APP_FOR_TEST="$status_fake_system_app" \
    "$ROOT_DIR/status.sh" 2>&1
)"
require_output "$status_user_host_output" 'statusUserHostConflict=true' "status-user-host-conflict-self-check-missing-summary"
require_status_block_reason "$status_user_host_output" "user-host-conflict" "status-user-host-conflict-self-check-missing-block-reason"
echo "statusUserHostConflictSelfCheck=true"
status_blocker_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
status_blocker_tis_after="$(current_input_source_id)"
status_blocker_debug_after="$(debug_events_env)"
if [[ "$status_blocker_clipboard_after" != "$status_blocker_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=status-blocker-self-check-mutated-clipboard"
  exit 1
fi
echo "statusBlockerSelfCheck.clipboardUnchanged=true"
assert_current_source_unchanged "statusBlockerSelfCheck" "$status_blocker_tis_before" "$status_blocker_tis_after"
assert_debug_env_unchanged "statusBlockerSelfCheck" "$status_blocker_debug_before" "$status_blocker_debug_after"
assert_process_not_running osascript "status-blocker-self-check-left-osascript"
assert_process_not_running InputiaInputMethod "status-blocker-self-check-left-inputia-host"

section "smoke preflight safe gates"
tis_readiness_build_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
tis_readiness_build_tis_before="$(current_input_source_id)"
tis_readiness_build_debug_before="$(debug_events_env)"
run_and_prefix "tisReadinessBuild: " "$ROOT_DIR/tis-readiness.sh" "$BUILD_APP"
tis_readiness_build_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
tis_readiness_build_tis_after="$(current_input_source_id)"
tis_readiness_build_debug_after="$(debug_events_env)"
if [[ "$tis_readiness_build_clipboard_after" != "$tis_readiness_build_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=tis-readiness-build-mutated-clipboard"
  exit 1
fi
echo "tisReadinessBuild.clipboardUnchanged=true"
assert_current_source_unchanged "tisReadinessBuild" "$tis_readiness_build_tis_before" "$tis_readiness_build_tis_after"
assert_debug_env_unchanged "tisReadinessBuild" "$tis_readiness_build_debug_before" "$tis_readiness_build_debug_after"
assert_no_user_host "tisReadinessBuild"
system_preflight_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
system_preflight_tis_before="$(current_input_source_id)"
system_preflight_debug_before="$(debug_events_env)"
system_preflight_app_missing=false
run_allow_rc "1,2,4" "systemPreflight" "$ROOT_DIR/smoke-preflight.sh" "$SYSTEM_APP"
if [[ "$RUN_EXPECT_RC_OUTPUT" == *"smokePreflightReady=false reason=missing-executable"* ]]; then
  system_preflight_app_missing=true
  echo "systemPreflightAcceptedBlockReason=missing-executable"
elif [[ "$RUN_EXPECT_RC_OUTPUT" == *"smokePreflightReady=false reason=cdhash-mismatch"* ]]; then
  echo "systemPreflightAcceptedBlockReason=cdhash-mismatch"
else
  require_output "$RUN_EXPECT_RC_OUTPUT" "smokePreflightReady=false reason=ui-smoke-disabled" "system-preflight-missing-ui-disabled-gate"
  echo "systemPreflightAcceptedBlockReason=ui-smoke-disabled"
fi
system_preflight_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
system_preflight_tis_after="$(current_input_source_id)"
system_preflight_debug_after="$(debug_events_env)"
if [[ "$system_preflight_clipboard_after" != "$system_preflight_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=system-preflight-mutated-clipboard"
  exit 1
fi
echo "systemPreflight.clipboardUnchanged=true"
assert_current_source_unchanged "systemPreflight" "$system_preflight_tis_before" "$system_preflight_tis_after"
assert_debug_env_unchanged "systemPreflight" "$system_preflight_debug_before" "$system_preflight_debug_after"
assert_no_user_host "systemPreflight"
if [[ "$TEXTEDIT_PREEXISTING" == "false" && "$system_preflight_app_missing" == "false" ]]; then
  system_preflight_textedit_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  system_preflight_textedit_tis_before="$(current_input_source_id)"
  system_preflight_textedit_debug_before="$(debug_events_env)"
  run_expect_rc 6 "systemPreflightTextEditBeforeCdhashGate" \
    env INPUTIA_PROCESS_RUNNING_FOR_TEST=TextEdit \
      INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 \
      "$ROOT_DIR/smoke-preflight.sh" "$SYSTEM_APP"
  require_output "$RUN_EXPECT_RC_OUTPUT" "textEditPreflight=running" "system-preflight-textedit-before-cdhash-missing-preflight"
  require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeReady=false reason=textedit-already-running" "system-preflight-textedit-before-cdhash-missing-gui-block"
  require_output "$RUN_EXPECT_RC_OUTPUT" "smokePreflightReady=false reason=textedit-already-running" "system-preflight-textedit-before-cdhash-missing-ready-block"
  if /usr/bin/grep -q "cdhash-mismatch" <<<"$RUN_EXPECT_RC_OUTPUT"; then
    echo "nonGuiVerificationPassed=false reason=system-preflight-textedit-checked-cdhash-first"
    exit 1
  fi
  system_preflight_textedit_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  system_preflight_textedit_tis_after="$(current_input_source_id)"
  system_preflight_textedit_debug_after="$(debug_events_env)"
  if [[ "$system_preflight_textedit_clipboard_after" != "$system_preflight_textedit_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=system-preflight-textedit-before-cdhash-mutated-clipboard"
    exit 1
  fi
  echo "systemPreflightTextEditBeforeCdhashGate.clipboardUnchanged=true"
  assert_current_source_unchanged "systemPreflightTextEditBeforeCdhashGate" "$system_preflight_textedit_tis_before" "$system_preflight_textedit_tis_after"
  assert_debug_env_unchanged "systemPreflightTextEditBeforeCdhashGate" "$system_preflight_textedit_debug_before" "$system_preflight_textedit_debug_after"
  assert_no_user_host "systemPreflightTextEditBeforeCdhashGate"
  assert_process_not_running osascript "system-preflight-textedit-before-cdhash-left-osascript"
  assert_process_not_running InputiaInputMethod "system-preflight-textedit-before-cdhash-left-inputia-host"
  echo "systemPreflightTextEditBeforeCdhashGateNoLaunchPassed=true"
else
  echo "systemPreflightTextEditBeforeCdhashGateSkipped=true reason=textedit-preexisting"
fi
build_preflight_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
build_preflight_tis_before="$(current_input_source_id)"
build_preflight_debug_before="$(debug_events_env)"
run_expect_rc 4 "buildPreflightUiDisabled" \
  env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/smoke-preflight.sh" "$BUILD_APP"
build_preflight_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
build_preflight_tis_after="$(current_input_source_id)"
build_preflight_debug_after="$(debug_events_env)"
if [[ "$build_preflight_clipboard_after" != "$build_preflight_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=build-preflight-ui-disabled-mutated-clipboard"
  exit 1
fi
echo "buildPreflightUiDisabled.clipboardUnchanged=true"
assert_current_source_unchanged "buildPreflightUiDisabled" "$build_preflight_tis_before" "$build_preflight_tis_after"
assert_debug_env_unchanged "buildPreflightUiDisabled" "$build_preflight_debug_before" "$build_preflight_debug_after"
assert_no_user_host "buildPreflightUiDisabled"
if [[ "$TEXTEDIT_PREEXISTING" == "false" && "$SAFARI_PREEXISTING" == "false" ]]; then
  build_preflight_tis_gate_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_tis_gate_tis_before="$(current_input_source_id)"
  build_preflight_tis_gate_debug_before="$(debug_events_env)"
  run_expect_rc 8 "buildPreflightUiTisGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-preflight.sh" "$BUILD_APP"
  require_output "$RUN_EXPECT_RC_OUTPUT" "guiSessionCheck=skipped" "build-preflight-ui-tis-gate-missing-gui-session-skip"
  require_output "$RUN_EXPECT_RC_OUTPUT" "textEditPreflight=not-running" "build-preflight-ui-tis-gate-missing-textedit-preflight"
  require_output "$RUN_EXPECT_RC_OUTPUT" "safariPreflight=not-running" "build-preflight-ui-tis-gate-missing-safari-preflight"
  require_output_regex \
    "$RUN_EXPECT_RC_OUTPUT" \
    'smokePreflightReady=false reason=(signature-rejected|tis-not-ready)' \
    "build-preflight-ui-tis-gate-missing-readiness-blocker"
  build_preflight_tis_gate_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_tis_gate_tis_after="$(current_input_source_id)"
  build_preflight_tis_gate_debug_after="$(debug_events_env)"
  if [[ "$build_preflight_tis_gate_clipboard_after" != "$build_preflight_tis_gate_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=build-preflight-ui-tis-gate-mutated-clipboard"
    exit 1
  fi
  echo "buildPreflightUiTisGate.clipboardUnchanged=true"
  assert_current_source_unchanged "buildPreflightUiTisGate" "$build_preflight_tis_gate_tis_before" "$build_preflight_tis_gate_tis_after"
  assert_debug_env_unchanged "buildPreflightUiTisGate" "$build_preflight_tis_gate_debug_before" "$build_preflight_tis_gate_debug_after"
  assert_no_user_host "buildPreflightUiTisGate"
  assert_process_not_running TextEdit "build-preflight-ui-tis-gate-launched-textedit"
  assert_process_not_running Safari "build-preflight-ui-tis-gate-launched-safari"
  assert_process_not_running osascript "build-preflight-ui-tis-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "build-preflight-ui-tis-gate-left-inputia-host"
  echo "buildPreflightUiTisGateNoLaunchPassed=true"
else
  echo "buildPreflightUiTisGateSkipped=true reason=existing-gui-app"
fi

if ! process_running InputiaInputMethod; then
  start_fake_existing_process InputiaInputMethod
  fake_preflight_inputia_pid="$INPUTIA_FAKE_EXISTING_PID"
  build_preflight_inputia_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_inputia_tis_before="$(current_input_source_id)"
  build_preflight_inputia_debug_before="$(debug_events_env)"
  run_expect_rc 9 "buildPreflightInputiaHostGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-preflight.sh" "$BUILD_APP"
  require_output "$RUN_EXPECT_RC_OUTPUT" "InputiaInputMethodPreflight=running" "build-preflight-inputia-host-gate-missing-preflight"
  require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeReady=false reason=inputia-host-running" "build-preflight-inputia-host-gate-missing-gui-block"
  require_output "$RUN_EXPECT_RC_OUTPUT" "smokePreflightReady=false reason=inputia-host-running" "build-preflight-inputia-host-gate-missing-ready-block"
  build_preflight_inputia_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_inputia_tis_after="$(current_input_source_id)"
  build_preflight_inputia_debug_after="$(debug_events_env)"
  if [[ "$build_preflight_inputia_clipboard_after" != "$build_preflight_inputia_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=build-preflight-inputia-host-gate-mutated-clipboard"
    exit 1
  fi
  echo "buildPreflightInputiaHostGate.clipboardUnchanged=true"
  assert_current_source_unchanged "buildPreflightInputiaHostGate" "$build_preflight_inputia_tis_before" "$build_preflight_inputia_tis_after"
  assert_debug_env_unchanged "buildPreflightInputiaHostGate" "$build_preflight_inputia_debug_before" "$build_preflight_inputia_debug_after"
  assert_no_user_host "buildPreflightInputiaHostGate"
  if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
    assert_process_not_running TextEdit "build-preflight-inputia-host-gate-launched-textedit"
  fi
  if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
    assert_process_not_running Safari "build-preflight-inputia-host-gate-launched-safari"
  fi
  assert_process_not_running osascript "build-preflight-inputia-host-gate-left-osascript"
  stop_fake_existing_process InputiaInputMethod "$fake_preflight_inputia_pid"
  echo "buildPreflightInputiaHostGateNoLaunchPassed=true"
else
  echo "buildPreflightInputiaHostGateSkipped=true reason=inputia-host-preexisting"
fi

if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  start_fake_existing_process TextEdit
  fake_preflight_textedit_pid="$INPUTIA_FAKE_EXISTING_PID"
  build_preflight_textedit_allow_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_textedit_allow_tis_before="$(current_input_source_id)"
  build_preflight_textedit_allow_debug_before="$(debug_events_env)"
  run_expect_rc 6 "buildPreflightTextEditNoAllowGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_TEXTEDIT_SMOKE_ALLOW_EXISTING=1 \
      "$ROOT_DIR/smoke-preflight.sh" "$BUILD_APP"
  require_output "$RUN_EXPECT_RC_OUTPUT" "textEditPreflight=running" "build-preflight-textedit-no-allow-missing-preflight"
  require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeReady=false reason=textedit-already-running" "build-preflight-textedit-no-allow-missing-gui-block"
  require_output "$RUN_EXPECT_RC_OUTPUT" "smokePreflightReady=false reason=textedit-already-running" "build-preflight-textedit-no-allow-missing-ready-block"
  build_preflight_textedit_allow_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_textedit_allow_tis_after="$(current_input_source_id)"
  build_preflight_textedit_allow_debug_after="$(debug_events_env)"
  if [[ "$build_preflight_textedit_allow_clipboard_after" != "$build_preflight_textedit_allow_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=build-preflight-textedit-no-allow-mutated-clipboard"
    exit 1
  fi
  echo "buildPreflightTextEditNoAllowGate.clipboardUnchanged=true"
  assert_current_source_unchanged "buildPreflightTextEditNoAllowGate" "$build_preflight_textedit_allow_tis_before" "$build_preflight_textedit_allow_tis_after"
  assert_debug_env_unchanged "buildPreflightTextEditNoAllowGate" "$build_preflight_textedit_allow_debug_before" "$build_preflight_textedit_allow_debug_after"
  assert_no_user_host "buildPreflightTextEditNoAllowGate"
  stop_fake_existing_process TextEdit "$fake_preflight_textedit_pid"
  assert_process_not_running osascript "build-preflight-textedit-no-allow-left-osascript"
  assert_process_not_running InputiaInputMethod "build-preflight-textedit-no-allow-left-inputia-host"
  echo "buildPreflightTextEditNoAllowGateNoLaunchPassed=true"
else
  echo "buildPreflightTextEditNoAllowGateSkipped=true reason=textedit-preexisting"
fi

if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  start_fake_existing_process Safari
  fake_preflight_safari_pid="$INPUTIA_FAKE_EXISTING_PID"
  build_preflight_safari_allow_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_safari_allow_tis_before="$(current_input_source_id)"
  build_preflight_safari_allow_debug_before="$(debug_events_env)"
  run_expect_rc 7 "buildPreflightSafariNoAllowGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_SAFARI_SMOKE_ALLOW_EXISTING=1 \
      "$ROOT_DIR/smoke-preflight.sh" "$BUILD_APP"
  require_output "$RUN_EXPECT_RC_OUTPUT" "safariPreflight=running" "build-preflight-safari-no-allow-missing-preflight"
  require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeReady=false reason=safari-already-running" "build-preflight-safari-no-allow-missing-gui-block"
  require_output "$RUN_EXPECT_RC_OUTPUT" "smokePreflightReady=false reason=safari-already-running" "build-preflight-safari-no-allow-missing-ready-block"
  build_preflight_safari_allow_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  build_preflight_safari_allow_tis_after="$(current_input_source_id)"
  build_preflight_safari_allow_debug_after="$(debug_events_env)"
  if [[ "$build_preflight_safari_allow_clipboard_after" != "$build_preflight_safari_allow_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=build-preflight-safari-no-allow-mutated-clipboard"
    exit 1
  fi
  echo "buildPreflightSafariNoAllowGate.clipboardUnchanged=true"
  assert_current_source_unchanged "buildPreflightSafariNoAllowGate" "$build_preflight_safari_allow_tis_before" "$build_preflight_safari_allow_tis_after"
  assert_debug_env_unchanged "buildPreflightSafariNoAllowGate" "$build_preflight_safari_allow_debug_before" "$build_preflight_safari_allow_debug_after"
  assert_no_user_host "buildPreflightSafariNoAllowGate"
  stop_fake_existing_process Safari "$fake_preflight_safari_pid"
  assert_process_not_running osascript "build-preflight-safari-no-allow-left-osascript"
  assert_process_not_running InputiaInputMethod "build-preflight-safari-no-allow-left-inputia-host"
  echo "buildPreflightSafariNoAllowGateNoLaunchPassed=true"
else
  echo "buildPreflightSafariNoAllowGateSkipped=true reason=safari-preexisting"
fi

section "ui-disabled smoke no-launch gates"
if process_running TextEdit; then
  echo "uiDisabledNoLaunchSkipped=true reason=textedit-already-running"
else
	  textedit_ui_disabled_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_ui_disabled_tis_before="$(current_input_source_id)"
	  textedit_ui_disabled_debug_before="$(debug_events_env)"
	  run_expect_rc 14 "textEditUiDisabled" \
	    env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/smoke-textedit.sh" "$BUILD_APP"
	  textedit_ui_disabled_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_ui_disabled_tis_after="$(current_input_source_id)"
	  textedit_ui_disabled_debug_after="$(debug_events_env)"
	  if [[ "$textedit_ui_disabled_clipboard_after" != "$textedit_ui_disabled_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=textedit-ui-disabled-mutated-clipboard"
	    exit 1
	  fi
	  echo "textEditUiDisabled.clipboardUnchanged=true"
	  assert_current_source_unchanged "textEditUiDisabled" "$textedit_ui_disabled_tis_before" "$textedit_ui_disabled_tis_after"
	  assert_debug_env_unchanged "textEditUiDisabled" "$textedit_ui_disabled_debug_before" "$textedit_ui_disabled_debug_after"
	  assert_no_user_host "textEditUiDisabled"
	  textedit_command_ui_disabled_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_ui_disabled_tis_before="$(current_input_source_id)"
	  textedit_command_ui_disabled_debug_before="$(debug_events_env)"
	  run_expect_rc 16 "textEditCommandUiDisabled" \
	    env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$BUILD_APP"
	  textedit_command_ui_disabled_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_ui_disabled_tis_after="$(current_input_source_id)"
	  textedit_command_ui_disabled_debug_after="$(debug_events_env)"
	  if [[ "$textedit_command_ui_disabled_clipboard_after" != "$textedit_command_ui_disabled_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=textedit-command-ui-disabled-mutated-clipboard"
	    exit 1
	  fi
	  echo "textEditCommandUiDisabled.clipboardUnchanged=true"
	  assert_current_source_unchanged "textEditCommandUiDisabled" "$textedit_command_ui_disabled_tis_before" "$textedit_command_ui_disabled_tis_after"
	  assert_debug_env_unchanged "textEditCommandUiDisabled" "$textedit_command_ui_disabled_debug_before" "$textedit_command_ui_disabled_debug_after"
	  assert_no_user_host "textEditCommandUiDisabled"
	  clipboard_ui_disabled_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_ui_disabled_tis_before="$(current_input_source_id)"
  clipboard_ui_disabled_debug_before="$(debug_events_env)"
  run_expect_rc 7 "clipboardUiDisabled" \
    env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP"
  clipboard_ui_disabled_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_ui_disabled_tis_after="$(current_input_source_id)"
  clipboard_ui_disabled_debug_after="$(debug_events_env)"
  if [[ "$clipboard_ui_disabled_clipboard_after" != "$clipboard_ui_disabled_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=clipboard-ui-disabled-mutated-clipboard"
    exit 1
  fi
  echo "clipboardUiDisabled.clipboardUnchanged=true"
  assert_current_source_unchanged "clipboardUiDisabled" "$clipboard_ui_disabled_tis_before" "$clipboard_ui_disabled_tis_after"
  assert_debug_env_unchanged "clipboardUiDisabled" "$clipboard_ui_disabled_debug_before" "$clipboard_ui_disabled_debug_after"
  assert_no_user_host "clipboardUiDisabled"
  assert_process_not_running TextEdit "ui-disabled-launched-textedit"
  assert_process_not_running osascript "ui-disabled-left-osascript"
  assert_process_not_running InputiaInputMethod "ui-disabled-left-inputia-host"
  echo "uiDisabledNoLaunchPassed=true"

  clipboard_inputia_host_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_inputia_host_tis_before="$(current_input_source_id)"
  clipboard_inputia_host_debug_before="$(debug_events_env)"
  run_expect_rc 10 "clipboardInputiaHostGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod \
      "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP"
  require_output "$RUN_EXPECT_RC_OUTPUT" "InputiaInputMethodPreflight=running" "clipboard-inputia-host-gate-missing-preflight"
  require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeReady=false reason=inputia-host-running" "clipboard-inputia-host-gate-missing-gui-block"
  require_output "$RUN_EXPECT_RC_OUTPUT" "clipboardRecallSmokeReady=false reason=inputia-host-running" "clipboard-inputia-host-gate-missing-ready-block"
  clipboard_inputia_host_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_inputia_host_tis_after="$(current_input_source_id)"
  clipboard_inputia_host_debug_after="$(debug_events_env)"
  if [[ "$clipboard_inputia_host_clipboard_after" != "$clipboard_inputia_host_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=clipboard-inputia-host-gate-mutated-clipboard"
    exit 1
  fi
  echo "clipboardInputiaHostGate.clipboardUnchanged=true"
  assert_current_source_unchanged "clipboardInputiaHostGate" "$clipboard_inputia_host_tis_before" "$clipboard_inputia_host_tis_after"
  assert_debug_env_unchanged "clipboardInputiaHostGate" "$clipboard_inputia_host_debug_before" "$clipboard_inputia_host_debug_after"
  assert_no_user_host "clipboardInputiaHostGate"
  assert_process_not_running TextEdit "clipboard-inputia-host-gate-launched-textedit"
  assert_process_not_running osascript "clipboard-inputia-host-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "clipboard-inputia-host-gate-left-inputia-host"
  echo "clipboardInputiaHostGateNoLaunchPassed=true"

  textedit_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  textedit_tis_before="$(current_input_source_id)"
  textedit_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 15 14 "textEditUiTisGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-textedit.sh" "$BUILD_APP"
  textedit_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  textedit_tis_after="$(current_input_source_id)"
  textedit_debug_after="$(debug_events_env)"
  if [[ "$textedit_clipboard_after" != "$textedit_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=textedit-ui-tis-gate-mutated-clipboard"
    exit 1
  fi
  echo "textEditUiTisGate.clipboardUnchanged=true"
  assert_current_source_unchanged "textEditUiTisGate" "$textedit_tis_before" "$textedit_tis_after"
  assert_debug_env_unchanged "textEditUiTisGate" "$textedit_debug_before" "$textedit_debug_after"
  assert_no_user_host "textEditUiTisGate"
	  assert_process_not_running TextEdit "textedit-ui-tis-gate-launched-textedit"
	  assert_process_not_running osascript "textedit-ui-tis-gate-left-osascript"
	  assert_process_not_running InputiaInputMethod "textedit-ui-tis-gate-left-inputia-host"
	  echo "textEditUiTisGateNoLaunchPassed=true"

	  textedit_command_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_tis_before="$(current_input_source_id)"
	  textedit_command_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 17 16 "textEditCommandUiTisGate" \
	    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
	      "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$BUILD_APP"
	  textedit_command_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_tis_after="$(current_input_source_id)"
	  textedit_command_debug_after="$(debug_events_env)"
	  if [[ "$textedit_command_clipboard_after" != "$textedit_command_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=textedit-command-ui-tis-gate-mutated-clipboard"
	    exit 1
	  fi
	  assert_current_source_unchanged "textEditCommandUiTisGate" "$textedit_command_tis_before" "$textedit_command_tis_after"
	  assert_debug_env_unchanged "textEditCommandUiTisGate" "$textedit_command_debug_before" "$textedit_command_debug_after"
	  assert_no_user_host "textEditCommandUiTisGate"
	  assert_process_not_running TextEdit "textedit-command-ui-tis-gate-launched-textedit"
	  assert_process_not_running osascript "textedit-command-ui-tis-gate-left-osascript"
	  assert_process_not_running InputiaInputMethod "textedit-command-ui-tis-gate-left-inputia-host"
	  echo "textEditCommandUiTisGateNoLaunchPassed=true"

	  textedit_command_non_text_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_non_text_tis_before="$(current_input_source_id)"
	  textedit_command_non_text_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 18 16 "textEditCommandNonTextClipboardGate" \
	    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
	      INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
	      INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="«class RTF », 512, string, 11, Unicode text, 22" \
	      "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$BUILD_APP"
	  textedit_command_non_text_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_non_text_tis_after="$(current_input_source_id)"
	  textedit_command_non_text_debug_after="$(debug_events_env)"
	  if [[ "$textedit_command_non_text_clipboard_after" != "$textedit_command_non_text_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=textedit-command-non-text-gate-mutated-clipboard"
	    exit 1
	  fi
	  echo "textEditCommandNonTextClipboardGate.clipboardUnchanged=true"
	  assert_current_source_unchanged "textEditCommandNonTextClipboardGate" "$textedit_command_non_text_tis_before" "$textedit_command_non_text_tis_after"
	  assert_debug_env_unchanged "textEditCommandNonTextClipboardGate" "$textedit_command_non_text_debug_before" "$textedit_command_non_text_debug_after"
	  assert_no_user_host "textEditCommandNonTextClipboardGate"
	  assert_process_not_running TextEdit "textedit-command-non-text-gate-launched-textedit"
	  assert_process_not_running osascript "textedit-command-non-text-gate-left-osascript"
	  assert_process_not_running InputiaInputMethod "textedit-command-non-text-gate-left-inputia-host"
	  echo "textEditCommandNonTextClipboardGatePassed=true"

	  textedit_command_missing_text_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_missing_text_tis_before="$(current_input_source_id)"
	  textedit_command_missing_text_debug_before="$(debug_events_env)"
		  run_expect_rc_or_gui_block 18 16 "textEditCommandMissingTextClipboardGate" \
	    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
	      INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
	      INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="JPEG picture, 2048" \
	      "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$BUILD_APP"
	  textedit_command_missing_text_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_missing_text_tis_after="$(current_input_source_id)"
	  textedit_command_missing_text_debug_after="$(debug_events_env)"
	  if [[ "$textedit_command_missing_text_clipboard_after" != "$textedit_command_missing_text_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=textedit-command-missing-text-gate-mutated-clipboard"
	    exit 1
	  fi
	  echo "textEditCommandMissingTextClipboardGate.clipboardUnchanged=true"
	  assert_current_source_unchanged "textEditCommandMissingTextClipboardGate" "$textedit_command_missing_text_tis_before" "$textedit_command_missing_text_tis_after"
	  assert_debug_env_unchanged "textEditCommandMissingTextClipboardGate" "$textedit_command_missing_text_debug_before" "$textedit_command_missing_text_debug_after"
	  assert_no_user_host "textEditCommandMissingTextClipboardGate"
	  assert_process_not_running TextEdit "textedit-command-missing-text-gate-launched-textedit"
	  assert_process_not_running osascript "textedit-command-missing-text-gate-left-osascript"
	  assert_process_not_running InputiaInputMethod "textedit-command-missing-text-gate-left-inputia-host"
	  echo "textEditCommandMissingTextClipboardGatePassed=true"

	  start_fake_existing_process TextEdit
	  fake_textedit_pid="$INPUTIA_FAKE_EXISTING_PID"
	  textedit_existing_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_existing_tis_before="$(current_input_source_id)"
	  textedit_existing_debug_before="$(debug_events_env)"
		  run_expect_rc_or_gui_block 13 14 "textEditExistingGate" \
	    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
	      "$ROOT_DIR/smoke-textedit.sh" "$BUILD_APP"
	  textedit_existing_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_existing_tis_after="$(current_input_source_id)"
	  textedit_existing_debug_after="$(debug_events_env)"
	  if [[ "$textedit_existing_clipboard_after" != "$textedit_existing_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=textedit-existing-gate-mutated-clipboard"
	    exit 1
	  fi
	  echo "textEditExistingGate.clipboardUnchanged=true"
	  assert_current_source_unchanged "textEditExistingGate" "$textedit_existing_tis_before" "$textedit_existing_tis_after"
	  assert_debug_env_unchanged "textEditExistingGate" "$textedit_existing_debug_before" "$textedit_existing_debug_after"
	  assert_no_user_host "textEditExistingGate"

	  textedit_command_existing_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_existing_tis_before="$(current_input_source_id)"
	  textedit_command_existing_debug_before="$(debug_events_env)"
		  run_expect_rc_or_gui_block 13 16 "textEditCommandExistingGate" \
	    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
	      "$ROOT_DIR/smoke-textedit-command-shortcuts.sh" "$BUILD_APP"
	  textedit_command_existing_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  textedit_command_existing_tis_after="$(current_input_source_id)"
	  textedit_command_existing_debug_after="$(debug_events_env)"
	  if [[ "$textedit_command_existing_clipboard_after" != "$textedit_command_existing_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=textedit-command-existing-gate-mutated-clipboard"
	    exit 1
	  fi
	  echo "textEditCommandExistingGate.clipboardUnchanged=true"
	  assert_current_source_unchanged "textEditCommandExistingGate" "$textedit_command_existing_tis_before" "$textedit_command_existing_tis_after"
	  assert_debug_env_unchanged "textEditCommandExistingGate" "$textedit_command_existing_debug_before" "$textedit_command_existing_debug_after"
	  assert_no_user_host "textEditCommandExistingGate"

	  clipboard_existing_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  clipboard_existing_tis_before="$(current_input_source_id)"
	  clipboard_existing_debug_before="$(debug_events_env)"
		  run_expect_rc_or_gui_block 6 7 "clipboardExistingTextEditGate" \
	    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
	      "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP"
	  clipboard_existing_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
	  clipboard_existing_tis_after="$(current_input_source_id)"
	  clipboard_existing_debug_after="$(debug_events_env)"
	  if [[ "$clipboard_existing_clipboard_after" != "$clipboard_existing_clipboard_before" ]]; then
	    echo "nonGuiVerificationPassed=false reason=clipboard-existing-textedit-gate-mutated-clipboard"
	    exit 1
	  fi
	  echo "clipboardExistingTextEditGate.clipboardUnchanged=true"
	  assert_current_source_unchanged "clipboardExistingTextEditGate" "$clipboard_existing_tis_before" "$clipboard_existing_tis_after"
	  assert_debug_env_unchanged "clipboardExistingTextEditGate" "$clipboard_existing_debug_before" "$clipboard_existing_debug_after"
	  assert_no_user_host "clipboardExistingTextEditGate"
	  assert_process_not_running osascript "textedit-existing-gate-left-osascript"
	  assert_process_not_running InputiaInputMethod "textedit-existing-gate-left-inputia-host"
	  stop_fake_existing_process TextEdit "$fake_textedit_pid"
	  echo "textEditExistingGateNoMutationPassed=true"

	  clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_tis_before="$(current_input_source_id)"
  clipboard_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 8 7 "clipboardUiTisGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP"
  clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_tis_after="$(current_input_source_id)"
  clipboard_debug_after="$(debug_events_env)"
  if [[ "$clipboard_after" != "$clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=clipboard-ui-tis-gate-mutated-clipboard"
    exit 1
  fi
  assert_current_source_unchanged "clipboardUiTisGate" "$clipboard_tis_before" "$clipboard_tis_after"
  assert_debug_env_unchanged "clipboardUiTisGate" "$clipboard_debug_before" "$clipboard_debug_after"
  assert_no_user_host "clipboardUiTisGate"
  assert_process_not_running TextEdit "clipboard-ui-tis-gate-launched-textedit"
  assert_process_not_running osascript "clipboard-ui-tis-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "clipboard-ui-tis-gate-left-inputia-host"
  echo "clipboardUiTisGateNoLaunchPassed=true"

  clipboard_non_text_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_non_text_tis_before="$(current_input_source_id)"
  clipboard_non_text_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 9 7 "clipboardNonTextGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
      INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="TIFF picture, 2048, string, 11, Unicode text, 22" \
      "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP"
  clipboard_non_text_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_non_text_tis_after="$(current_input_source_id)"
  clipboard_non_text_debug_after="$(debug_events_env)"
  if [[ "$clipboard_non_text_clipboard_after" != "$clipboard_non_text_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=clipboard-non-text-gate-mutated-clipboard"
    exit 1
  fi
  echo "clipboardNonTextGate.clipboardUnchanged=true"
  assert_current_source_unchanged "clipboardNonTextGate" "$clipboard_non_text_tis_before" "$clipboard_non_text_tis_after"
  assert_debug_env_unchanged "clipboardNonTextGate" "$clipboard_non_text_debug_before" "$clipboard_non_text_debug_after"
  assert_no_user_host "clipboardNonTextGate"
  assert_process_not_running TextEdit "clipboard-non-text-gate-launched-textedit"
  assert_process_not_running osascript "clipboard-non-text-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "clipboard-non-text-gate-left-inputia-host"
  echo "clipboardNonTextGatePassed=true"

  clipboard_missing_text_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_missing_text_tis_before="$(current_input_source_id)"
  clipboard_missing_text_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 9 7 "clipboardMissingTextGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
      INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="" \
      "$ROOT_DIR/smoke-clipboard-recall.sh" "$BUILD_APP"
  clipboard_missing_text_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  clipboard_missing_text_tis_after="$(current_input_source_id)"
  clipboard_missing_text_debug_after="$(debug_events_env)"
  if [[ "$clipboard_missing_text_clipboard_after" != "$clipboard_missing_text_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=clipboard-missing-text-gate-mutated-clipboard"
    exit 1
  fi
  echo "clipboardMissingTextGate.clipboardUnchanged=true"
  assert_current_source_unchanged "clipboardMissingTextGate" "$clipboard_missing_text_tis_before" "$clipboard_missing_text_tis_after"
  assert_debug_env_unchanged "clipboardMissingTextGate" "$clipboard_missing_text_debug_before" "$clipboard_missing_text_debug_after"
  assert_no_user_host "clipboardMissingTextGate"
  assert_process_not_running TextEdit "clipboard-missing-text-gate-launched-textedit"
  assert_process_not_running osascript "clipboard-missing-text-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "clipboard-missing-text-gate-left-inputia-host"
  echo "clipboardMissingTextGatePassed=true"
fi

section "safari ui-disabled no-launch gates"
if process_running Safari; then
  safari_preexisting=true
else
  safari_preexisting=false
fi
echo "safariPreExisting=$safari_preexisting"
safari_typing_ui_disabled_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_typing_ui_disabled_tis_before="$(current_input_source_id)"
safari_typing_ui_disabled_debug_before="$(debug_events_env)"
run_expect_rc 7 "safariTypingUiDisabled" \
  env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/smoke-safari-typing.sh" "$BUILD_APP"
safari_typing_ui_disabled_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_typing_ui_disabled_tis_after="$(current_input_source_id)"
safari_typing_ui_disabled_debug_after="$(debug_events_env)"
if [[ "$safari_typing_ui_disabled_clipboard_after" != "$safari_typing_ui_disabled_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-typing-ui-disabled-mutated-clipboard"
  exit 1
fi
echo "safariTypingUiDisabled.clipboardUnchanged=true"
assert_current_source_unchanged "safariTypingUiDisabled" "$safari_typing_ui_disabled_tis_before" "$safari_typing_ui_disabled_tis_after"
assert_debug_env_unchanged "safariTypingUiDisabled" "$safari_typing_ui_disabled_debug_before" "$safari_typing_ui_disabled_debug_after"
assert_no_user_host "safariTypingUiDisabled"
safari_command_ui_disabled_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_command_ui_disabled_tis_before="$(current_input_source_id)"
safari_command_ui_disabled_debug_before="$(debug_events_env)"
run_expect_rc 12 "safariCommandUiDisabled" \
  env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$BUILD_APP"
safari_command_ui_disabled_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_command_ui_disabled_tis_after="$(current_input_source_id)"
safari_command_ui_disabled_debug_after="$(debug_events_env)"
if [[ "$safari_command_ui_disabled_clipboard_after" != "$safari_command_ui_disabled_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-command-ui-disabled-mutated-clipboard"
  exit 1
fi
echo "safariCommandUiDisabled.clipboardUnchanged=true"
assert_current_source_unchanged "safariCommandUiDisabled" "$safari_command_ui_disabled_tis_before" "$safari_command_ui_disabled_tis_after"
assert_debug_env_unchanged "safariCommandUiDisabled" "$safari_command_ui_disabled_debug_before" "$safari_command_ui_disabled_debug_after"
assert_no_user_host "safariCommandUiDisabled"
safari_enter_ui_disabled_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_enter_ui_disabled_tis_before="$(current_input_source_id)"
safari_enter_ui_disabled_debug_before="$(debug_events_env)"
run_expect_rc 5 "safariEnterUiDisabled" \
  env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/smoke-safari-enter.sh" "$BUILD_APP"
safari_enter_ui_disabled_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_enter_ui_disabled_tis_after="$(current_input_source_id)"
safari_enter_ui_disabled_debug_after="$(debug_events_env)"
if [[ "$safari_enter_ui_disabled_clipboard_after" != "$safari_enter_ui_disabled_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-enter-ui-disabled-mutated-clipboard"
  exit 1
fi
echo "safariEnterUiDisabled.clipboardUnchanged=true"
assert_current_source_unchanged "safariEnterUiDisabled" "$safari_enter_ui_disabled_tis_before" "$safari_enter_ui_disabled_tis_after"
assert_debug_env_unchanged "safariEnterUiDisabled" "$safari_enter_ui_disabled_debug_before" "$safari_enter_ui_disabled_debug_after"
assert_no_user_host "safariEnterUiDisabled"
safari_diagnose_ui_disabled_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_diagnose_ui_disabled_tis_before="$(current_input_source_id)"
safari_diagnose_ui_disabled_debug_before="$(debug_events_env)"
run_expect_rc 10 "safariDiagnoseUiDisabled" \
  env INPUTIA_SKIP_CDHASH_CHECK=1 "$ROOT_DIR/diagnose-safari-input-source.sh" "$BUILD_APP"
safari_diagnose_ui_disabled_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
safari_diagnose_ui_disabled_tis_after="$(current_input_source_id)"
safari_diagnose_ui_disabled_debug_after="$(debug_events_env)"
if [[ "$safari_diagnose_ui_disabled_clipboard_after" != "$safari_diagnose_ui_disabled_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=safari-diagnose-ui-disabled-mutated-clipboard"
  exit 1
fi
echo "safariDiagnoseUiDisabled.clipboardUnchanged=true"
assert_current_source_unchanged "safariDiagnoseUiDisabled" "$safari_diagnose_ui_disabled_tis_before" "$safari_diagnose_ui_disabled_tis_after"
assert_debug_env_unchanged "safariDiagnoseUiDisabled" "$safari_diagnose_ui_disabled_debug_before" "$safari_diagnose_ui_disabled_debug_after"
assert_no_user_host "safariDiagnoseUiDisabled"
assert_process_not_running osascript "safari-ui-disabled-left-osascript"
assert_process_not_running InputiaInputMethod "safari-ui-disabled-left-inputia-host"
if [[ "$safari_preexisting" == "false" ]]; then
  assert_process_not_running Safari "safari-ui-disabled-launched-safari"

  safari_enter_inputia_host_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_enter_inputia_host_tis_before="$(current_input_source_id)"
  safari_enter_inputia_host_debug_before="$(debug_events_env)"
  run_expect_rc 11 "safariEnterInputiaHostGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_GUI_SESSION_CHECK=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_PROCESS_RUNNING_FOR_TEST=InputiaInputMethod \
      "$ROOT_DIR/smoke-safari-enter.sh" "$BUILD_APP"
  require_output "$RUN_EXPECT_RC_OUTPUT" "InputiaInputMethodPreflight=running" "safari-enter-inputia-host-gate-missing-preflight"
  require_output "$RUN_EXPECT_RC_OUTPUT" "guiSmokeReady=false reason=inputia-host-running" "safari-enter-inputia-host-gate-missing-gui-block"
  require_output "$RUN_EXPECT_RC_OUTPUT" "safariEnterSmokeReady=false reason=inputia-host-running" "safari-enter-inputia-host-gate-missing-ready-block"
  safari_enter_inputia_host_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_enter_inputia_host_tis_after="$(current_input_source_id)"
  safari_enter_inputia_host_debug_after="$(debug_events_env)"
  if [[ "$safari_enter_inputia_host_clipboard_after" != "$safari_enter_inputia_host_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-enter-inputia-host-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariEnterInputiaHostGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariEnterInputiaHostGate" "$safari_enter_inputia_host_tis_before" "$safari_enter_inputia_host_tis_after"
  assert_debug_env_unchanged "safariEnterInputiaHostGate" "$safari_enter_inputia_host_debug_before" "$safari_enter_inputia_host_debug_after"
  assert_no_user_host "safariEnterInputiaHostGate"
  assert_process_not_running Safari "safari-enter-inputia-host-gate-launched-safari"
  assert_process_not_running osascript "safari-enter-inputia-host-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-enter-inputia-host-gate-left-inputia-host"
  echo "safariEnterInputiaHostGateNoLaunchPassed=true"

  safari_command_non_text_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_non_text_tis_before="$(current_input_source_id)"
  safari_command_non_text_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 15 12 "safariCommandNonTextClipboardGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
      INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="PDF , 2048, string, 11, Unicode text, 22" \
      "$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$BUILD_APP"
  safari_command_non_text_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_non_text_tis_after="$(current_input_source_id)"
  safari_command_non_text_debug_after="$(debug_events_env)"
  if [[ "$safari_command_non_text_clipboard_after" != "$safari_command_non_text_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-command-non-text-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariCommandNonTextClipboardGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariCommandNonTextClipboardGate" "$safari_command_non_text_tis_before" "$safari_command_non_text_tis_after"
  assert_debug_env_unchanged "safariCommandNonTextClipboardGate" "$safari_command_non_text_debug_before" "$safari_command_non_text_debug_after"
  assert_no_user_host "safariCommandNonTextClipboardGate"
  assert_process_not_running Safari "safari-command-non-text-gate-launched-safari"
  assert_process_not_running osascript "safari-command-non-text-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-command-non-text-gate-left-inputia-host"
  echo "safariCommandNonTextClipboardGatePassed=true"

  safari_command_missing_text_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_missing_text_tis_before="$(current_input_source_id)"
  safari_command_missing_text_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 15 12 "safariCommandMissingTextClipboardGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
      INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="JPEG picture, 2048" \
      "$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$BUILD_APP"
  safari_command_missing_text_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_missing_text_tis_after="$(current_input_source_id)"
  safari_command_missing_text_debug_after="$(debug_events_env)"
  if [[ "$safari_command_missing_text_clipboard_after" != "$safari_command_missing_text_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-command-missing-text-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariCommandMissingTextClipboardGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariCommandMissingTextClipboardGate" "$safari_command_missing_text_tis_before" "$safari_command_missing_text_tis_after"
  assert_debug_env_unchanged "safariCommandMissingTextClipboardGate" "$safari_command_missing_text_debug_before" "$safari_command_missing_text_debug_after"
  assert_no_user_host "safariCommandMissingTextClipboardGate"
  assert_process_not_running Safari "safari-command-missing-text-gate-launched-safari"
  assert_process_not_running osascript "safari-command-missing-text-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-command-missing-text-gate-left-inputia-host"
  echo "safariCommandMissingTextClipboardGatePassed=true"

  safari_typing_tis_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_typing_tis_before="$(current_input_source_id)"
  safari_typing_tis_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 8 7 "safariTypingUiTisGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-safari-typing.sh" "$BUILD_APP"
  safari_typing_tis_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_typing_tis_after="$(current_input_source_id)"
  safari_typing_tis_debug_after="$(debug_events_env)"
  if [[ "$safari_typing_tis_clipboard_after" != "$safari_typing_tis_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-typing-ui-tis-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariTypingUiTisGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariTypingUiTisGate" "$safari_typing_tis_before" "$safari_typing_tis_after"
  assert_debug_env_unchanged "safariTypingUiTisGate" "$safari_typing_tis_debug_before" "$safari_typing_tis_debug_after"
  assert_no_user_host "safariTypingUiTisGate"
  assert_process_not_running Safari "safari-typing-ui-tis-gate-launched-safari"
  assert_process_not_running osascript "safari-typing-ui-tis-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-typing-ui-tis-gate-left-inputia-host"
  echo "safariTypingUiTisGateNoLaunchPassed=true"

  safari_command_tis_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_tis_before="$(current_input_source_id)"
  safari_command_tis_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 14 12 "safariCommandUiTisGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      INPUTIA_ENABLE_CLIPBOARD_INFO_OVERRIDE_FOR_TEST=1 \
      INPUTIA_CLIPBOARD_INFO_OVERRIDE_FOR_TEST="Unicode text, 42" \
      "$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$BUILD_APP"
  safari_command_tis_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_tis_after="$(current_input_source_id)"
  safari_command_tis_debug_after="$(debug_events_env)"
  if [[ "$safari_command_tis_clipboard_after" != "$safari_command_tis_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-command-ui-tis-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariCommandUiTisGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariCommandUiTisGate" "$safari_command_tis_before" "$safari_command_tis_after"
  assert_debug_env_unchanged "safariCommandUiTisGate" "$safari_command_tis_debug_before" "$safari_command_tis_debug_after"
  assert_no_user_host "safariCommandUiTisGate"
  assert_process_not_running Safari "safari-command-ui-tis-gate-launched-safari"
  assert_process_not_running osascript "safari-command-ui-tis-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-command-ui-tis-gate-left-inputia-host"
  echo "safariCommandUiTisGateNoLaunchPassed=true"

  safari_enter_tis_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_enter_tis_before="$(current_input_source_id)"
  safari_enter_tis_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 6 5 "safariEnterUiTisGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-safari-enter.sh" "$BUILD_APP"
  safari_enter_tis_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_enter_tis_after="$(current_input_source_id)"
  safari_enter_tis_debug_after="$(debug_events_env)"
  if [[ "$safari_enter_tis_clipboard_after" != "$safari_enter_tis_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-enter-ui-tis-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariEnterUiTisGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariEnterUiTisGate" "$safari_enter_tis_before" "$safari_enter_tis_after"
  assert_debug_env_unchanged "safariEnterUiTisGate" "$safari_enter_tis_debug_before" "$safari_enter_tis_debug_after"
  assert_no_user_host "safariEnterUiTisGate"
  assert_process_not_running Safari "safari-enter-ui-tis-gate-launched-safari"
  assert_process_not_running osascript "safari-enter-ui-tis-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-enter-ui-tis-gate-left-inputia-host"
  echo "safariEnterUiTisGateNoLaunchPassed=true"

  safari_diagnose_tis_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_diagnose_tis_before="$(current_input_source_id)"
  safari_diagnose_tis_debug_before="$(debug_events_env)"
	  run_expect_rc_or_gui_block 12 10 "safariDiagnoseUiTisGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/diagnose-safari-input-source.sh" "$BUILD_APP"
  safari_diagnose_tis_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_diagnose_tis_after="$(current_input_source_id)"
  safari_diagnose_tis_debug_after="$(debug_events_env)"
  if [[ "$safari_diagnose_tis_clipboard_after" != "$safari_diagnose_tis_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-diagnose-ui-tis-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariDiagnoseUiTisGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariDiagnoseUiTisGate" "$safari_diagnose_tis_before" "$safari_diagnose_tis_after"
  assert_debug_env_unchanged "safariDiagnoseUiTisGate" "$safari_diagnose_tis_debug_before" "$safari_diagnose_tis_debug_after"
  assert_no_user_host "safariDiagnoseUiTisGate"
  assert_process_not_running Safari "safari-diagnose-ui-tis-gate-launched-safari"
  assert_process_not_running osascript "safari-diagnose-ui-tis-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-diagnose-ui-tis-gate-left-inputia-host"
  echo "safariDiagnoseUiTisGateNoLaunchPassed=true"
fi
echo "safariUiDisabledNoLaunchPassed=true"

fake_safari_pid=""
if [[ "$safari_preexisting" == "false" ]]; then
  start_fake_existing_process Safari
  fake_safari_pid="$INPUTIA_FAKE_EXISTING_PID"
fi

if process_running Safari; then
  safari_typing_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_typing_tis_before="$(current_input_source_id)"
  safari_typing_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 9 7 "safariTypingExistingGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-safari-typing.sh" "$BUILD_APP"
  safari_typing_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_typing_tis_after="$(current_input_source_id)"
  safari_typing_debug_after="$(debug_events_env)"
  if [[ "$safari_typing_clipboard_after" != "$safari_typing_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-typing-existing-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariTypingExistingGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariTypingExistingGate" "$safari_typing_tis_before" "$safari_typing_tis_after"
  assert_debug_env_unchanged "safariTypingExistingGate" "$safari_typing_debug_before" "$safari_typing_debug_after"
  assert_no_user_host "safariTypingExistingGate"

  safari_command_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_tis_before="$(current_input_source_id)"
  safari_command_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 13 12 "safariCommandExistingGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-safari-command-shortcuts.sh" "$BUILD_APP"
  safari_command_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_command_tis_after="$(current_input_source_id)"
  safari_command_debug_after="$(debug_events_env)"
  if [[ "$safari_command_clipboard_after" != "$safari_command_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-command-existing-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariCommandExistingGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariCommandExistingGate" "$safari_command_tis_before" "$safari_command_tis_after"
  assert_debug_env_unchanged "safariCommandExistingGate" "$safari_command_debug_before" "$safari_command_debug_after"
  assert_no_user_host "safariCommandExistingGate"

  safari_enter_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_enter_tis_before="$(current_input_source_id)"
  safari_enter_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 7 5 "safariEnterExistingGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/smoke-safari-enter.sh" "$BUILD_APP"
  safari_enter_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_enter_tis_after="$(current_input_source_id)"
  safari_enter_debug_after="$(debug_events_env)"
  if [[ "$safari_enter_clipboard_after" != "$safari_enter_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-enter-existing-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariEnterExistingGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariEnterExistingGate" "$safari_enter_tis_before" "$safari_enter_tis_after"
  assert_debug_env_unchanged "safariEnterExistingGate" "$safari_enter_debug_before" "$safari_enter_debug_after"
  assert_no_user_host "safariEnterExistingGate"

  safari_diagnose_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_diagnose_tis_before="$(current_input_source_id)"
  safari_diagnose_debug_before="$(debug_events_env)"
  run_expect_rc_or_gui_block 11 10 "safariDiagnoseExistingGate" \
    env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_SKIP_CDHASH_CHECK=1 \
      "$ROOT_DIR/diagnose-safari-input-source.sh" "$BUILD_APP"
  safari_diagnose_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  safari_diagnose_tis_after="$(current_input_source_id)"
  safari_diagnose_debug_after="$(debug_events_env)"
  if [[ "$safari_diagnose_clipboard_after" != "$safari_diagnose_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=safari-diagnose-existing-gate-mutated-clipboard"
    exit 1
  fi
  echo "safariDiagnoseExistingGate.clipboardUnchanged=true"
  assert_current_source_unchanged "safariDiagnoseExistingGate" "$safari_diagnose_tis_before" "$safari_diagnose_tis_after"
  assert_debug_env_unchanged "safariDiagnoseExistingGate" "$safari_diagnose_debug_before" "$safari_diagnose_debug_after"
  assert_no_user_host "safariDiagnoseExistingGate"

  assert_process_not_running osascript "safari-existing-gate-left-osascript"
  assert_process_not_running InputiaInputMethod "safari-existing-gate-left-inputia-host"
  if [[ -n "$fake_safari_pid" ]]; then
    stop_fake_existing_process Safari "$fake_safari_pid"
    fake_safari_pid=""
  fi
  echo "safariExistingGateNoMutationPassed=true"
else
  if [[ -n "$fake_safari_pid" ]]; then
    stop_fake_existing_process Safari "$fake_safari_pid"
    fake_safari_pid=""
  fi
  echo "safariExistingGateSkipped=true reason=safari-not-running"
fi

section "post-install regression non-gui"
post_install_non_gui_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_non_gui_tis_before="$(current_input_source_id)"
post_install_non_gui_debug_before="$(debug_events_env)"
run_allow_rc "0,1" "postInstall" \
  env INPUTIA_RUN_UI_SMOKE=0 \
  INPUTIA_USER_APP="$VERIFY_POST_INSTALL_USER_APP" \
  INPUTIA_USER_LEGACY_APP="$VERIFY_POST_INSTALL_USER_LEGACY_APP" \
  INPUTIA_USER_SETTINGS_APP="$VERIFY_POST_INSTALL_USER_SETTINGS_APP" \
  "$ROOT_DIR/post-install-regression.sh" "$BUILD_APP"
if [[ "$RUN_EXPECT_RC_ACTUAL" != "0" ]]; then
  require_output_regex \
    "$RUN_EXPECT_RC_OUTPUT" \
    '== codesign ==|CSSMERR_TP_NOT_TRUSTED|code object is not signed|invalid signature|rejected' \
    "post-install-non-gui-unexpected-failure"
  echo "postInstallNonGuiSignatureBlocked=true reason=codesign-or-trust-unavailable"
fi
post_install_non_gui_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_non_gui_tis_after="$(current_input_source_id)"
post_install_non_gui_debug_after="$(debug_events_env)"
if [[ "$post_install_non_gui_clipboard_after" != "$post_install_non_gui_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=post-install-non-gui-mutated-clipboard"
  exit 1
fi
echo "postInstallNonGui.clipboardUnchanged=true"
assert_current_source_unchanged "postInstallNonGui" "$post_install_non_gui_tis_before" "$post_install_non_gui_tis_after"
assert_debug_env_unchanged "postInstallNonGui" "$post_install_non_gui_debug_before" "$post_install_non_gui_debug_after"
assert_no_user_host "postInstallNonGui"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "post-install-non-gui-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "post-install-non-gui-launched-safari"
fi
assert_process_not_running osascript "post-install-non-gui-left-osascript"
assert_process_not_running InputiaInputMethod "post-install-non-gui-left-inputia-host"
echo "postInstallNonGuiNoMutationPassed=true"

section "post-install UI TIS gate"
post_install_ui_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_ui_tis_before="$(current_input_source_id)"
post_install_ui_debug_before="$(debug_events_env)"
run_expect_rc 6 "postInstallUiTisGate" \
  env INPUTIA_RUN_UI_SMOKE=1 \
    INPUTIA_USER_APP="$VERIFY_POST_INSTALL_USER_APP" \
    INPUTIA_USER_LEGACY_APP="$VERIFY_POST_INSTALL_USER_LEGACY_APP" \
    INPUTIA_USER_SETTINGS_APP="$VERIFY_POST_INSTALL_USER_SETTINGS_APP" \
    "$ROOT_DIR/post-install-regression.sh" "$BUILD_APP"
require_output_regex \
  "$RUN_EXPECT_RC_OUTPUT" \
  'guiSmokeReady=false reason=(signature-rejected|tis-not-ready)' \
  "post-install-ui-tis-gate-missing-gui-block-reason"
require_output_regex \
  "$RUN_EXPECT_RC_OUTPUT" \
  'postInstallUiSmokeReady=false reason=(signature-rejected|tis-not-ready)' \
  "post-install-ui-tis-gate-missing-postinstall-block-reason"
post_install_ui_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
post_install_ui_tis_after="$(current_input_source_id)"
post_install_ui_debug_after="$(debug_events_env)"
if [[ "$post_install_ui_clipboard_after" != "$post_install_ui_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=post-install-ui-tis-gate-mutated-clipboard"
  exit 1
fi
echo "postInstallUiTisGate.clipboardUnchanged=true"
assert_current_source_unchanged "postInstallUiTisGate" "$post_install_ui_tis_before" "$post_install_ui_tis_after"
assert_debug_env_unchanged "postInstallUiTisGate" "$post_install_ui_debug_before" "$post_install_ui_debug_after"
assert_no_user_host "postInstallUiTisGate"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "post-install-ui-tis-gate-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "post-install-ui-tis-gate-launched-safari"
fi
assert_process_not_running osascript "post-install-ui-tis-gate-left-osascript"
assert_process_not_running InputiaInputMethod "post-install-ui-tis-gate-left-inputia-host"
echo "postInstallUiTisGateNoLaunchPassed=true"

section "await short timeout"
await_short_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
await_short_tis_before="$(current_input_source_id)"
await_short_debug_before="$(debug_events_env)"
run_expect_rc 2 "awaitShort" \
  env INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 "$ROOT_DIR/await-system-install.sh"
require_output "$RUN_EXPECT_RC_OUTPUT" "uiSmokeRequested=false" "await-short-missing-ui-disabled-request-marker"
require_output "$RUN_EXPECT_RC_OUTPUT" "uiSmokeWouldStart=false" "await-short-missing-ui-disabled-start-marker"
require_output "$RUN_EXPECT_RC_OUTPUT" "uiSmokeBlockReason=ui-smoke-disabled" "await-short-missing-ui-disabled-block-reason"
require_output "$RUN_EXPECT_RC_OUTPUT" "uiSmokeBlockReasons=ui-smoke-disabled" "await-short-missing-ui-disabled-block-reasons"
await_short_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
await_short_tis_after="$(current_input_source_id)"
await_short_debug_after="$(debug_events_env)"
if [[ "$await_short_clipboard_after" != "$await_short_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=await-short-mutated-clipboard"
  exit 1
fi
echo "awaitShort.clipboardUnchanged=true"
assert_current_source_unchanged "awaitShort" "$await_short_tis_before" "$await_short_tis_after"
assert_debug_env_unchanged "awaitShort" "$await_short_debug_before" "$await_short_debug_after"
assert_no_user_host "awaitShort"

section "await UI requested not-ready gate"
await_ui_not_ready_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
await_ui_not_ready_tis_before="$(current_input_source_id)"
await_ui_not_ready_debug_before="$(debug_events_env)"
run_expect_rc 2 "awaitUiNotReady" \
  env INPUTIA_RUN_UI_SMOKE=1 INPUTIA_INSTALL_WAIT_SECONDS=0 INPUTIA_INSTALL_POLL_SECONDS=1 \
    "$ROOT_DIR/await-system-install.sh"
require_output "$RUN_EXPECT_RC_OUTPUT" "uiSmokeRequested=true" "await-ui-not-ready-missing-ui-request-marker"
require_output "$RUN_EXPECT_RC_OUTPUT" "uiSmokeWouldStart=false" "await-ui-not-ready-missing-start-block-marker"
require_output_regex "$RUN_EXPECT_RC_OUTPUT" 'uiSmokeBlockReason=[^[:space:]]+' "await-ui-not-ready-missing-block-reason"
require_output_regex "$RUN_EXPECT_RC_OUTPUT" 'uiSmokeBlockReasons=[^[:space:]]+' "await-ui-not-ready-missing-block-reasons"
await_ui_not_ready_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
await_ui_not_ready_tis_after="$(current_input_source_id)"
await_ui_not_ready_debug_after="$(debug_events_env)"
if [[ "$await_ui_not_ready_clipboard_after" != "$await_ui_not_ready_clipboard_before" ]]; then
  echo "nonGuiVerificationPassed=false reason=await-ui-not-ready-mutated-clipboard"
  exit 1
fi
echo "awaitUiNotReady.clipboardUnchanged=true"
assert_current_source_unchanged "awaitUiNotReady" "$await_ui_not_ready_tis_before" "$await_ui_not_ready_tis_after"
assert_debug_env_unchanged "awaitUiNotReady" "$await_ui_not_ready_debug_before" "$await_ui_not_ready_debug_after"
assert_no_user_host "awaitUiNotReady"
if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "await-ui-not-ready-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "await-ui-not-ready-launched-safari"
fi
assert_process_not_running osascript "await-ui-not-ready-left-osascript"
assert_process_not_running InputiaInputMethod "await-ui-not-ready-left-inputia-host"
echo "awaitUiNotReadyNoLaunchPassed=true"

section "admin install no-prompt gate"
if [[ -w "/Library/Input Methods" ]] || /usr/bin/sudo -n true >/dev/null 2>&1; then
  echo "installNoPromptGateSkipped=true reason=admin-available"
else
  install_no_prompt_clipboard_before="$(/usr/bin/pbpaste 2>/dev/null || true)"
  install_no_prompt_tis_before="$(current_input_source_id)"
  install_no_prompt_debug_before="$(debug_events_env)"
  run_allow_rc "12,13" "installNoPrompt" \
    env INPUTIA_INSTALL_NO_ADMIN_PROMPT=1 "$ROOT_DIR/install-system.sh"
  require_output_regex \
    "$RUN_EXPECT_RC_OUTPUT" \
    'systemInstallReady=false reason=(admin-required|user-directory-unavailable)' \
    "install-no-prompt-missing-ready-failure"
  if [[ "$RUN_EXPECT_RC_ACTUAL" == "13" ]]; then
    require_output "$RUN_EXPECT_RC_OUTPUT" "systemInstallRequiredAction=repair-current-user-directory-service" "install-no-prompt-missing-user-directory-action"
    require_output "$RUN_EXPECT_RC_OUTPUT" "systemInstallBlockReason=missing-passwd-record" "install-no-prompt-missing-passwd-blocker"
  fi
  install_no_prompt_clipboard_after="$(/usr/bin/pbpaste 2>/dev/null || true)"
  install_no_prompt_tis_after="$(current_input_source_id)"
  install_no_prompt_debug_after="$(debug_events_env)"
  if [[ "$install_no_prompt_clipboard_after" != "$install_no_prompt_clipboard_before" ]]; then
    echo "nonGuiVerificationPassed=false reason=install-no-prompt-mutated-clipboard"
    exit 1
  fi
  echo "installNoPrompt.clipboardUnchanged=true"
  assert_current_source_unchanged "installNoPrompt" "$install_no_prompt_tis_before" "$install_no_prompt_tis_after"
  assert_debug_env_unchanged "installNoPrompt" "$install_no_prompt_debug_before" "$install_no_prompt_debug_after"
  assert_no_user_host "installNoPrompt"
fi

collect_residue() {
  /bin/ps -axo pid=,comm=,command= |
    /usr/bin/awk '
      $2 ~ /(^|\/)osascript$/ { print; next }
      $2 ~ /(^|\/)InputiaInputMethod$/ { print; next }
      $0 ~ /\/(post-install-regression|verify-system|smoke-preflight|smoke-textedit|smoke-textedit-command-shortcuts|smoke-clipboard-recall|smoke-safari[^ ]*|diagnose-safari-input-source|gui-smoke-readiness|gui-smoke-suite|verify-pkg|status|await-system-install)\.sh( |$)/ &&
        $2 ~ /(^|\/)(bash|zsh|sh|env)$/ { print; next }
    '
}

section "residue collector self-check"
residue_fake_script="/tmp/smoke-textedit.sh"
/bin/rm -f "$residue_fake_script"
/usr/bin/printf '#!/bin/zsh\n/bin/sleep 60\n' >"$residue_fake_script"
/bin/chmod +x "$residue_fake_script"
VERIFY_TEMP_FILES+=("$residue_fake_script")
/bin/zsh "$residue_fake_script" &
residue_fake_shell_pid=$!
VERIFY_FAKE_PROCESS_PIDS+=("$residue_fake_shell_pid")
/bin/sleep 0.2
residue_fake_shell_output="$(collect_residue)"
if ! /usr/bin/grep -q "$residue_fake_shell_pid" <<<"$residue_fake_shell_output"; then
  echo "nonGuiVerificationPassed=false reason=residue-collector-missed-shell-wrapper pid=$residue_fake_shell_pid"
  printf '%s\n' "$residue_fake_shell_output"
  exit 1
fi
/bin/kill "$residue_fake_shell_pid" >/dev/null 2>&1 || true
wait "$residue_fake_shell_pid" >/dev/null 2>&1 || true
remaining_fake_pids=()
remaining_fake_count=0
for candidate_pid in ${VERIFY_FAKE_PROCESS_PIDS[@]+"${VERIFY_FAKE_PROCESS_PIDS[@]}"}; do
  if [[ "$candidate_pid" != "$residue_fake_shell_pid" ]]; then
    remaining_fake_pids+=("$candidate_pid")
    remaining_fake_count=$((remaining_fake_count + 1))
  fi
done
if ((remaining_fake_count == 0)); then
  VERIFY_FAKE_PROCESS_PIDS=()
else
  VERIFY_FAKE_PROCESS_PIDS=("${remaining_fake_pids[@]}")
fi
/usr/bin/python3 -c 'import time; time.sleep(60)' "$residue_fake_script" &
residue_fake_text_pid=$!
VERIFY_FAKE_PROCESS_PIDS+=("$residue_fake_text_pid")
/bin/sleep 0.2
residue_fake_text_output="$(collect_residue)"
if /usr/bin/grep -q "$residue_fake_text_pid" <<<"$residue_fake_text_output"; then
  echo "nonGuiVerificationPassed=false reason=residue-collector-false-positive-argument-text pid=$residue_fake_text_pid"
  printf '%s\n' "$residue_fake_text_output"
  exit 1
fi
/bin/kill "$residue_fake_text_pid" >/dev/null 2>&1 || true
wait "$residue_fake_text_pid" >/dev/null 2>&1 || true
remaining_fake_pids=()
remaining_fake_count=0
for candidate_pid in ${VERIFY_FAKE_PROCESS_PIDS[@]+"${VERIFY_FAKE_PROCESS_PIDS[@]}"}; do
  if [[ "$candidate_pid" != "$residue_fake_text_pid" ]]; then
    remaining_fake_pids+=("$candidate_pid")
    remaining_fake_count=$((remaining_fake_count + 1))
  fi
done
if ((remaining_fake_count == 0)); then
  VERIFY_FAKE_PROCESS_PIDS=()
else
  VERIFY_FAKE_PROCESS_PIDS=("${remaining_fake_pids[@]}")
fi
/bin/rm -f "$residue_fake_script"
echo "residueCollectorShellWrapperDetected=true"
echo "residueCollectorArgumentTextIgnored=true"
echo "residueCollectorSelfCheck=true"

section "residue"
residue="$(collect_residue)"
residue_waits=0
residue_max_wait="${INPUTIA_PROCESS_WAIT_TICKS:-100}"
while [[ -n "$residue" && "$residue_waits" -lt "$residue_max_wait" ]]; do
  /bin/sleep 0.1
  residue_waits=$((residue_waits + 1))
  residue="$(collect_residue)"
done
if [[ -n "$residue" ]]; then
  echo "$residue"
  echo "nonGuiVerificationPassed=false reason=residue"
  exit 1
fi
echo "residue=false"

if [[ "$TEXTEDIT_PREEXISTING" == "false" ]]; then
  assert_process_not_running TextEdit "non-gui-launched-textedit"
fi
if [[ "$SAFARI_PREEXISTING" == "false" ]]; then
  assert_process_not_running Safari "non-gui-launched-safari"
fi

tmp_residue="$(
  /usr/bin/find "$TMP_RESIDUE_ROOT" -maxdepth 1 \
    ! -path "$VERIFY_LOCK_DIR" \
    ! -path "$VERIFY_LOCK_REAL_DIR" \
    \( -name 'inputia-*-select.log' \
      -o -name 'inputia-*-select.*.log' \
      -o -name 'inputia-*-restore.log' \
      -o -name 'inputia-*-restore.*.log' \
      -o -name 'inputia-safari-*-test.url' \
      -o -name 'inputia-safari-*-test.*.url' \
      -o -name 'inputia-*-events.*.log' \
      -o -name 'inputia-*-applescript.*' \
      -o -name 'inputia-*-osascript.*.applescript' \
      -o -name 'inputia-hitoolbox-preference.*.txt' \
      -o -name 'inputia-pkg-verify.*' \
      -o -name 'inputia-launchservices-*.log' \
      -o -name 'inputia-install-user.*' \
      -o -name 'inputia-debug-event-*' \
      -o -name 'inputia-verify-nongui.lock' \
      -o -name 'inputia-post-install-regression.lock' \) \
    -print 2>/dev/null
)"
if [[ -n "$tmp_residue" ]]; then
  echo "$tmp_residue"
  echo "nonGuiVerificationPassed=false reason=tmp-residue"
  exit 1
fi
echo "tmpResidue=false"

section "result"
echo "nonGuiVerificationPassed=true"
