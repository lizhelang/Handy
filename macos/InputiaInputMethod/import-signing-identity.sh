#!/bin/zsh
set -eu
set -o pipefail
umask 077

IDENTITY="${INPUTIA_CODESIGN_IDENTITY:-Codexbar Local Code Signing Leaf v4}"
P12_PATH="${INPUTIA_SIGNING_P12:-$HOME/.inputia/signing/Codexbar-Local-Code-Signing-Leaf-v4.p12}"
KEYCHAIN="${INPUTIA_SIGNING_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
P12_PASSWORD="${INPUTIA_P12_PASSWORD:-${INPUTIA_SIGNING_P12_PASSWORD:-}}"
KEYCHAIN_PASSWORD="${INPUTIA_KEYCHAIN_PASSWORD:-}"
IMPORT_TIMEOUT_SECONDS="${INPUTIA_SIGNING_IMPORT_TIMEOUT_SECONDS:-30}"
IMPORT_OUTPUT="$(/usr/bin/mktemp "${TMPDIR:-/tmp}/inputia-signing-import.XXXXXX")"
PROBE_OUTPUT="$(/usr/bin/mktemp "${TMPDIR:-/tmp}/inputia-signing-probe.XXXXXX")"
PROBE_FILE="$(/usr/bin/mktemp "${TMPDIR:-/tmp}/inputia-signing-probe-bin.XXXXXX")"

cleanup() {
  /bin/rm -f "$IMPORT_OUTPUT" "$PROBE_OUTPUT" "$PROBE_FILE"
}
trap cleanup EXIT

identity_available() {
  /usr/bin/security find-identity -v -p codesigning 2>/dev/null |
    /usr/bin/grep -F "$IDENTITY" >/dev/null 2>&1
}

run_with_timeout() {
  local timeout_seconds="$1"
  local output_file="$2"
  shift 2

  "$@" >"$output_file" 2>&1 &
  local command_pid=$!

  (
    /bin/sleep "$timeout_seconds"
    if /bin/kill -0 "$command_pid" >/dev/null 2>&1; then
      echo "signingIdentityImportTimeoutSeconds=$timeout_seconds" >>"$output_file"
      /bin/kill "$command_pid" >/dev/null 2>&1 || true
      /bin/sleep 1
      /bin/kill -9 "$command_pid" >/dev/null 2>&1 || true
    fi
  ) &
  local timer_pid=$!

  local exit_status=0
  wait "$command_pid" || exit_status=$?
  /bin/kill "$timer_pid" >/dev/null 2>&1 || true
  wait "$timer_pid" >/dev/null 2>&1 || true
  return "$exit_status"
}

codesign_probe() {
  /usr/bin/printf '#!/bin/sh\nexit 0\n' >"$PROBE_FILE"
  /bin/chmod 755 "$PROBE_FILE"

  local probe_exit=0
  run_with_timeout "$IMPORT_TIMEOUT_SECONDS" "$PROBE_OUTPUT" \
    /usr/bin/codesign \
    --force \
    --sign "$IDENTITY" \
    --options runtime \
    --timestamp=none \
    "$PROBE_FILE" || probe_exit=$?

  if [[ -s "$PROBE_OUTPUT" ]]; then
    /usr/bin/sed 's/^/signingIdentityCodesignProbeOutput: /' "$PROBE_OUTPUT"
  fi

  if [[ "$probe_exit" -eq 0 ]]; then
    echo "signingIdentityCodesignProbe=true"
    return 0
  fi

  echo "signingIdentityCodesignProbe=false rc=$probe_exit"
  return "$probe_exit"
}

unlock_keychain_with_password() {
  if [[ -z "$KEYCHAIN_PASSWORD" ]]; then
    echo "signingIdentityKeychainUnlock=skipped reason=missing-keychain-password"
    return 1
  fi

  if /usr/bin/security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null 2>&1; then
    echo "signingIdentityKeychainUnlocked=true"
    return 0
  fi

  echo "signingIdentityKeychainUnlocked=false"
  return 2
}

set_key_partition_list_with_password() {
  if [[ -z "$KEYCHAIN_PASSWORD" ]]; then
    echo "signingIdentityKeyPartitionListSet=skipped reason=missing-keychain-password"
    return 1
  fi

  if /usr/bin/security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null 2>&1; then
    echo "signingIdentityKeyPartitionListSet=true"
    return 0
  fi

  echo "signingIdentityKeyPartitionListSet=false"
  return 2
}

echo "signingIdentityName=$IDENTITY"
echo "signingIdentityP12Path=$P12_PATH"
echo "signingIdentityKeychain=$KEYCHAIN"

if [[ ! -f "$P12_PATH" ]]; then
  echo "signingIdentityImportReady=false reason=missing-p12"
  exit 10
fi

p12_mode="$(/usr/bin/stat -f '%Lp' "$P12_PATH")"
p12_owner="$(/usr/bin/stat -f '%Su' "$P12_PATH")"
p12_group="$(/usr/bin/stat -f '%Sg' "$P12_PATH")"
p12_size="$(/usr/bin/stat -f '%z' "$P12_PATH")"
echo "signingIdentityP12Mode=$p12_mode"
echo "signingIdentityP12Owner=$p12_owner"
echo "signingIdentityP12Group=$p12_group"
echo "signingIdentityP12Size=$p12_size"

if [[ "$p12_mode" != "600" ]]; then
  echo "signingIdentityImportReady=false reason=insecure-p12-permissions"
  echo "signingIdentityRequiredAction=chmod-600-p12"
  exit 11
fi

if [[ ! -f "$KEYCHAIN" ]]; then
  echo "signingIdentityImportReady=false reason=missing-keychain"
  exit 13
fi

if identity_available; then
  echo "signingIdentityAlreadyAvailable=true"
  /usr/bin/security find-identity -v -p codesigning | /usr/bin/grep -F "$IDENTITY"
  if ! codesign_probe; then
    if [[ -n "$KEYCHAIN_PASSWORD" ]]; then
      unlock_keychain_with_password || exit 14
      set_key_partition_list_with_password || exit 15
      if codesign_probe; then
        echo "signingIdentityImportVerified=true"
        exit 0
      fi
    else
      echo "signingIdentityKeyPartitionRepair=skipped reason=missing-keychain-password"
    fi
    echo "signingIdentityImportVerified=false reason=codesign-probe-failed"
    echo "signingIdentityRequiredAction=fix-keychain-trust-or-private-key-acl"
    exit 17
  fi
  echo "signingIdentityImportVerified=true"
  exit 0
fi

echo "signingIdentityAlreadyAvailable=false"

if [[ -z "$P12_PASSWORD" ]]; then
  echo "signingIdentityImportReady=false reason=missing-p12-password"
  echo "signingIdentityRequiredAction=set-INPUTIA_P12_PASSWORD-or-INPUTIA_SIGNING_P12_PASSWORD"
  exit 12
fi

if [[ -n "$KEYCHAIN_PASSWORD" ]]; then
  unlock_keychain_with_password || exit 14
else
  unlock_keychain_with_password || true
fi

echo "signingIdentityImportReady=true"
import_exit=0
run_with_timeout "$IMPORT_TIMEOUT_SECONDS" "$IMPORT_OUTPUT" \
  /usr/bin/security import "$P12_PATH" \
  -k "$KEYCHAIN" \
  -P "$P12_PASSWORD" \
  -T /usr/bin/codesign \
  -T /usr/bin/security || import_exit=$?

if [[ -s "$IMPORT_OUTPUT" ]]; then
  /usr/bin/sed 's/^/signingIdentityImportOutput: /' "$IMPORT_OUTPUT"
fi

if [[ "$import_exit" -ne 0 ]]; then
  echo "signingIdentityImportSucceeded=false rc=$import_exit"
  exit "$import_exit"
fi
echo "signingIdentityImportSucceeded=true"

if [[ -n "$KEYCHAIN_PASSWORD" ]]; then
  set_key_partition_list_with_password || exit 15
else
  set_key_partition_list_with_password || true
fi

if identity_available; then
  /usr/bin/security find-identity -v -p codesigning | /usr/bin/grep -F "$IDENTITY"
  if ! codesign_probe; then
    echo "signingIdentityImportVerified=false reason=codesign-probe-failed"
    echo "signingIdentityRequiredAction=fix-keychain-trust-or-private-key-acl"
    exit 17
  fi
  echo "signingIdentityImportVerified=true"
  exit 0
fi

echo "signingIdentityImportVerified=false"
echo "signingIdentityRequiredAction=check-p12-private-key-and-keychain-trust"
exit 16
