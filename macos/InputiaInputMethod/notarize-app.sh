#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP="${1:-$ROOT_DIR/build/InputiaInputMethod.app}"
NOTARY_PROFILE="${INPUTIA_NOTARY_PROFILE:-Inputia}"
DIST_DIR="$ROOT_DIR/dist"
WORK_DIR="$ROOT_DIR/build/notary"
SUBMIT_TIMEOUT="${INPUTIA_NOTARY_TIMEOUT:-45m}"

echo "inputiaNotarizeAppTool=true"
echo "app=$APP"
echo "notaryProfile=$NOTARY_PROFILE"

fail() {
  local rc="$1"
  local reason="$2"
  echo "notarizeAppReady=false reason=$reason"
  echo "notarizeAppPassed=false reason=$reason"
  exit "$rc"
}

first_codesign_value() {
  local dump="$1"
  local key="$2"
  printf '%s\n' "$dump" | /usr/bin/awk -F= -v key="$key" '$1 == key { print $2; exit }'
}

if [[ ! -d "$APP" ]]; then
  fail 2 missing-app
fi

notarytool_path="$(/usr/bin/xcrun --find notarytool 2>/dev/null || true)"
stapler_path="$(/usr/bin/xcrun --find stapler 2>/dev/null || true)"
echo "notarytoolAvailable=$([[ -n "$notarytool_path" ]] && echo true || echo false)"
if [[ -n "$notarytool_path" ]]; then
  echo "notarytoolPath=$notarytool_path"
fi
echo "staplerAvailable=$([[ -n "$stapler_path" ]] && echo true || echo false)"
if [[ -n "$stapler_path" ]]; then
  echo "staplerPath=$stapler_path"
fi
[[ -n "$notarytool_path" ]] || fail 10 missing-notarytool
[[ -n "$stapler_path" ]] || fail 11 missing-stapler

set +e
codesign_verify_output="$(/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP" 2>&1)"
codesign_verify_rc=$?
set -e
echo "codesignVerify=$([[ "$codesign_verify_rc" -eq 0 ]] && echo true || echo false)"
if [[ "$codesign_verify_rc" -ne 0 ]]; then
  printf '%s\n' "$codesign_verify_output" | /usr/bin/sed 's/^/codesignVerifyOutput: /'
  echo "notarizeAppRequiredAction=rebuild-with-developer-id-application"
  fail 14 codesign-invalid
fi

codesign_dump="$(/usr/bin/codesign -dv --verbose=4 "$APP" 2>&1 || true)"
app_cdhash="$(first_codesign_value "$codesign_dump" CDHash)"
team_identifier="$(first_codesign_value "$codesign_dump" TeamIdentifier)"
runtime_line="$(printf '%s\n' "$codesign_dump" | /usr/bin/awk -F= '$1 == "Runtime Version" { print $2; exit }')"
echo "appCDHash=${app_cdhash:-unknown}"
echo "teamIdentifier=${team_identifier:-unknown}"
printf '%s\n' "$codesign_dump" |
  /usr/bin/awk -F= '$1 == "Authority" { print "codesignAuthority="$2 }'

authority_lines="$(printf '%s\n' "$codesign_dump" | /usr/bin/awk -F= '$1 == "Authority" { print $2 }')"
if ! /usr/bin/grep -q '^Developer ID Application:' <<<"$authority_lines"; then
  echo "developerIDApplicationSignature=false"
  echo "notarizeAppRequiredAction=rebuild-with-developer-id-application"
  fail 15 app-not-signed-with-developer-id
fi
echo "developerIDApplicationSignature=true"

if [[ -z "$runtime_line" ]]; then
  echo "hardenedRuntime=false"
  echo "notarizeAppRequiredAction=rebuild-with-hardened-runtime"
  fail 16 missing-hardened-runtime
fi
echo "hardenedRuntime=true"

set +e
notary_profile_output="$(/usr/bin/xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" 2>&1)"
notary_profile_rc=$?
set -e
if [[ "$notary_profile_rc" -ne 0 ]]; then
  printf '%s\n' "$notary_profile_output" | /usr/bin/sed 's/^/notaryProfileCheckOutput: /'
  if [[ "$notary_profile_output" == *"No Keychain password item found"* ]]; then
    echo "notarizeAppRequiredAction=store-notarytool-credentials"
    fail 12 missing-notarytool-profile
  fi
  echo "notarizeAppRequiredAction=fix-notarytool-profile-or-keychain"
  fail 13 notary-profile-unavailable
fi
echo "notaryProfileAvailable=true"

if [[ "${INPUTIA_NOTARIZE_APP_PREFLIGHT_ONLY:-0}" == "1" ]]; then
  echo "notarizeAppReady=true mode=preflight-only"
  echo "notarizeAppPassed=skipped reason=preflight-only"
  exit 0
fi

/bin/mkdir -p "$DIST_DIR" "$WORK_DIR"
archive_name="InputiaInputMethod-${app_cdhash:-unknown}-notary.zip"
archive_path="$DIST_DIR/$archive_name"
submission_plist="$WORK_DIR/notary-submit.plist"
/bin/rm -f "$archive_path" "$submission_plist"

echo "notarizeArchive=$archive_path"
/usr/bin/ditto -c -k --keepParent "$APP" "$archive_path"
archive_sha256="$(/usr/bin/shasum -a 256 "$archive_path" | /usr/bin/awk '{ print $1 }')"
echo "notarizeArchiveSha256=$archive_sha256"

echo "notarizeSubmitWait=true timeout=$SUBMIT_TIMEOUT"
set +e
/usr/bin/xcrun notarytool submit "$archive_path" \
  --keychain-profile "$NOTARY_PROFILE" \
  --wait \
  --timeout "$SUBMIT_TIMEOUT" \
  --output-format plist \
  >"$submission_plist" 2>&1
submit_rc=$?
set -e

if [[ "$submit_rc" -ne 0 ]]; then
  /usr/bin/sed 's/^/notarySubmitOutput: /' "$submission_plist" 2>/dev/null || true
  fail 17 notary-submit-failed
fi

submission_id="$(/usr/libexec/PlistBuddy -c 'Print :id' "$submission_plist" 2>/dev/null || true)"
submission_status="$(/usr/libexec/PlistBuddy -c 'Print :status' "$submission_plist" 2>/dev/null || true)"
echo "notarySubmissionID=${submission_id:-unknown}"
echo "notarySubmissionStatus=${submission_status:-unknown}"
if [[ "$submission_status" != "Accepted" ]]; then
  /usr/bin/sed 's/^/notarySubmitOutput: /' "$submission_plist" 2>/dev/null || true
  echo "notarizeAppRequiredAction=inspect-notary-log"
  if [[ -n "$submission_id" ]]; then
    echo "notaryLogCommand=xcrun notarytool log $submission_id --keychain-profile $NOTARY_PROFILE"
  fi
  fail 18 notary-not-accepted
fi

/usr/bin/xcrun stapler staple "$APP"
/usr/bin/xcrun stapler validate "$APP"
spctl_output="$(/usr/sbin/spctl --assess --type execute --verbose=4 "$APP" 2>&1 || true)"
printf '%s\n' "$spctl_output" | /usr/bin/sed 's/^/spctlAssessment: /'
if [[ "$spctl_output" != *": accepted"* ]]; then
  fail 19 spctl-rejected-after-staple
fi

echo "notarizeAppReady=true"
echo "notarizeAppPassed=true"
