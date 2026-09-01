#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOCK_SCRIPT="$ROOT_DIR/build-artifact-lock.sh"
TEST_ROOT="$(/usr/bin/mktemp -d /tmp/inputia-build-artifact-lock-test.XXXXXX)"

cleanup() {
  /bin/rm -rf "$TEST_ROOT" >/dev/null 2>&1 || true
}
trap cleanup EXIT

source "$LOCK_SCRIPT"

INPUTIA_BUILD_ARTIFACT_LOCK_DIR="$TEST_ROOT/normal.lock"
INPUTIA_BUILD_ARTIFACT_LOCK_HELD=0
INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED=0
inputia_build_artifact_acquire_lock lockSelfCheckNormal
normal_exists=false
[[ -d "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR" ]] && normal_exists=true
inputia_build_artifact_release_lock
normal_released=false
[[ ! -d "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR" ]] && normal_released=true

INPUTIA_BUILD_ARTIFACT_LOCK_DIR="$TEST_ROOT/stale.lock"
/bin/mkdir "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR"
echo 999999 >"$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/pid"
echo stale-owner >"$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/label"
INPUTIA_BUILD_ARTIFACT_LOCK_HELD=0
INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED=0
inputia_build_artifact_acquire_lock lockSelfCheckStale
stale_recovered=false
if [[ -f "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/label" ]] &&
  [[ "$(/bin/cat "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/label")" == "lockSelfCheckStale" ]]; then
  stale_recovered=true
fi
inputia_build_artifact_release_lock

ACTIVE_LOCK="$TEST_ROOT/active.lock"
/bin/mkdir "$ACTIVE_LOCK"
echo $$ >"$ACTIVE_LOCK/pid"
echo active-owner >"$ACTIVE_LOCK/label"
set +e
active_output="$(
  INPUTIA_BUILD_ARTIFACT_LOCK_DIR="$ACTIVE_LOCK" \
    /bin/zsh -c 'source "$1"; inputia_build_artifact_acquire_lock lockSelfCheckActive' _ "$LOCK_SCRIPT" 2>&1
)"
active_rc=$?
set -e
active_blocked=false
if [[ "$active_rc" == "20" ]] &&
  /usr/bin/grep -q 'reason=build-artifact-lock-held' <<<"$active_output"; then
  active_blocked=true
fi

echo "buildArtifactLockNormalAcquired=$normal_exists"
echo "buildArtifactLockNormalReleased=$normal_released"
echo "buildArtifactLockStaleRecovered=$stale_recovered"
echo "buildArtifactLockActiveBlocked=$active_blocked"
if [[ "$normal_exists" == "true" &&
  "$normal_released" == "true" &&
  "$stale_recovered" == "true" &&
  "$active_blocked" == "true" ]]; then
  echo "buildArtifactLockSelfCheck=true"
else
  echo "buildArtifactLockSelfCheck=false"
  exit 1
fi
