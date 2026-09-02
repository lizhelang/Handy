#!/bin/zsh

INPUTIA_BUILD_ARTIFACT_LOCK_DIR="${INPUTIA_BUILD_ARTIFACT_LOCK_DIR:-/tmp/inputia-build-artifacts.lock}"
INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED="${INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED:-0}"

inputia_build_artifact_release_lock() {
  if [[ "${INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED:-0}" == "1" ]]; then
    /bin/rm -rf "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR" >/dev/null 2>&1 || true
    INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED=0
    INPUTIA_BUILD_ARTIFACT_LOCK_HELD=0
    export INPUTIA_BUILD_ARTIFACT_LOCK_HELD
  fi
}

inputia_build_artifact_acquire_lock() {
  local label="${1:-buildArtifact}"
  local existing_pid existing_label

  if [[ "${INPUTIA_BUILD_ARTIFACT_LOCK_HELD:-0}" == "1" ]]; then
    echo "${label}BuildArtifactLock=reentrant path=$INPUTIA_BUILD_ARTIFACT_LOCK_DIR"
    return 0
  fi

  while ! /bin/mkdir "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR" 2>/dev/null; do
    existing_pid=""
    existing_label=""
    if [[ -f "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/pid" ]]; then
      existing_pid="$(/bin/cat "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/pid" 2>/dev/null || true)"
    fi
    if [[ -f "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/label" ]]; then
      existing_label="$(/bin/cat "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/label" 2>/dev/null || true)"
    fi

    if [[ -n "$existing_pid" ]] && /bin/kill -0 "$existing_pid" >/dev/null 2>&1; then
      echo "${label}Ready=false reason=build-artifact-lock-held"
      echo "buildArtifactLockPath=$INPUTIA_BUILD_ARTIFACT_LOCK_DIR"
      echo "buildArtifactLockOwnerPid=$existing_pid"
      echo "buildArtifactLockOwnerLabel=${existing_label:-unknown}"
      exit 20
    fi

    echo "buildArtifactLockStale=true path=$INPUTIA_BUILD_ARTIFACT_LOCK_DIR pid=${existing_pid:-unknown}"
    /bin/rm -rf "$INPUTIA_BUILD_ARTIFACT_LOCK_DIR" >/dev/null 2>&1 || true
  done

  INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED=1
  export INPUTIA_BUILD_ARTIFACT_LOCK_HELD=1
  export INPUTIA_BUILD_ARTIFACT_LOCK_DIR
  echo "$$" >"$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/pid"
  echo "$label" >"$INPUTIA_BUILD_ARTIFACT_LOCK_DIR/label"
  echo "${label}BuildArtifactLock=acquired path=$INPUTIA_BUILD_ARTIFACT_LOCK_DIR"
}
