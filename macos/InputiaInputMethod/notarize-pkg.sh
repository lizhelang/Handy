#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
PKG="${1:-$ROOT_DIR/dist/InputiaInputMethod-latest.pkg}"
NOTARY_PROFILE="${INPUTIA_NOTARY_PROFILE:-Inputia}"
WORK_DIR="$ROOT_DIR/build/notary"
SUBMIT_TIMEOUT="${INPUTIA_NOTARY_TIMEOUT:-45m}"

echo "inputiaNotarizePkgTool=true"
echo "pkg=$PKG"
echo "notaryProfile=$NOTARY_PROFILE"

append_reason() {
  local existing="$1"
  local reason="$2"
  if [[ -z "$existing" ]]; then
    printf '%s\n' "$reason"
  elif [[ ",$existing," == *",$reason,"* ]]; then
    printf '%s\n' "$existing"
  else
    printf '%s,%s\n' "$existing" "$reason"
  fi
}

first_reason() {
  local reasons="$1"
  printf '%s\n' "${reasons%%,*}"
}

fail_with_reasons() {
  local rc="$1"
  local reasons="$2"
  local reason
  reason="$(first_reason "$reasons")"
  echo "notarizePkgReady=false reason=$reason"
  echo "notarizePkgBlockReasons=$reasons"
  echo "notarizePkgPassed=false reason=$reason"
  exit "$rc"
}

if [[ ! -f "$PKG" ]]; then
  echo "notarizePkgReady=false reason=missing-pkg"
  echo "notarizePkgPassed=false reason=missing-pkg"
  exit 2
fi

pkg_sha256="$(/usr/bin/shasum -a 256 "$PKG" | /usr/bin/awk '{ print $1 }')"
echo "pkgSha256=$pkg_sha256"

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

set +e
pkg_signature_output="$(/usr/sbin/pkgutil --check-signature "$PKG" 2>&1)"
pkg_signature_rc=$?
set -e
echo "pkgSignatureRc=$pkg_signature_rc"
printf '%s\n' "$pkg_signature_output" | /usr/bin/sed 's/^/pkgSignatureOutput: /'
if [[ "$pkg_signature_output" == *"Developer ID Installer:"* ]]; then
  developer_id_installer_signature=true
else
  developer_id_installer_signature=false
fi
echo "developerIDInstallerSignature=$developer_id_installer_signature"

set +e
spctl_output="$(/usr/sbin/spctl --assess --type install --verbose=4 "$PKG" 2>&1)"
spctl_rc=$?
set -e
if [[ "$spctl_rc" -eq 0 && "$spctl_output" == *": accepted"* ]]; then
  spctl_install_accepted=true
else
  spctl_install_accepted=false
fi
echo "spctlInstallAccepted=$spctl_install_accepted"
printf '%s\n' "$spctl_output" | /usr/bin/sed 's/^/spctlInstallAssessment: /'

notary_profile_available=false
if [[ -n "$notarytool_path" ]]; then
  set +e
  notary_profile_output="$(/usr/bin/xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" 2>&1)"
  notary_profile_rc=$?
  set -e
  if [[ "$notary_profile_rc" -eq 0 ]]; then
    notary_profile_available=true
  else
    printf '%s\n' "$notary_profile_output" | /usr/bin/sed 's/^/notaryProfileCheckOutput: /'
  fi
fi
echo "notaryProfileAvailable=$notary_profile_available"

block_reasons=""
if [[ -z "$notarytool_path" ]]; then
  block_reasons="$(append_reason "$block_reasons" missing-notarytool)"
fi
if [[ -z "$stapler_path" ]]; then
  block_reasons="$(append_reason "$block_reasons" missing-stapler)"
fi
if [[ "$developer_id_installer_signature" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" pkg-not-signed-with-developer-id-installer)"
fi
if [[ "$notary_profile_available" != "true" ]]; then
  block_reasons="$(append_reason "$block_reasons" missing-notarytool-profile)"
fi

if [[ -n "$block_reasons" ]]; then
  if [[ "$block_reasons" == *"pkg-not-signed-with-developer-id-installer"* ]]; then
    echo "notarizePkgRequiredAction=rebuild-pkg-with-developer-id-installer"
    fail_with_reasons 15 "$block_reasons"
  fi
  if [[ "$block_reasons" == *"missing-notarytool-profile"* ]]; then
    echo "notarizePkgRequiredAction=store-notarytool-credentials"
    fail_with_reasons 12 "$block_reasons"
  fi
  fail_with_reasons 10 "$block_reasons"
fi

if [[ "${INPUTIA_NOTARIZE_PKG_PREFLIGHT_ONLY:-0}" == "1" ]]; then
  echo "notarizePkgReady=true mode=preflight-only"
  echo "notarizePkgPassed=skipped reason=preflight-only"
  exit 0
fi

/bin/mkdir -p "$WORK_DIR"
submission_plist="$WORK_DIR/pkg-notary-submit.plist"
/bin/rm -f "$submission_plist"

echo "notarizeSubmitWait=true timeout=$SUBMIT_TIMEOUT"
set +e
/usr/bin/xcrun notarytool submit "$PKG" \
  --keychain-profile "$NOTARY_PROFILE" \
  --wait \
  --timeout "$SUBMIT_TIMEOUT" \
  --output-format plist \
  >"$submission_plist" 2>&1
submit_rc=$?
set -e

if [[ "$submit_rc" -ne 0 ]]; then
  /usr/bin/sed 's/^/notarySubmitOutput: /' "$submission_plist" 2>/dev/null || true
  echo "notarizePkgReady=false reason=notary-submit-failed"
  echo "notarizePkgPassed=false reason=notary-submit-failed"
  exit 17
fi

submission_id="$(/usr/libexec/PlistBuddy -c 'Print :id' "$submission_plist" 2>/dev/null || true)"
submission_status="$(/usr/libexec/PlistBuddy -c 'Print :status' "$submission_plist" 2>/dev/null || true)"
echo "notarySubmissionID=${submission_id:-unknown}"
echo "notarySubmissionStatus=${submission_status:-unknown}"
if [[ "$submission_status" != "Accepted" ]]; then
  /usr/bin/sed 's/^/notarySubmitOutput: /' "$submission_plist" 2>/dev/null || true
  echo "notarizePkgRequiredAction=inspect-notary-log"
  if [[ -n "$submission_id" ]]; then
    echo "notaryLogCommand=xcrun notarytool log $submission_id --keychain-profile $NOTARY_PROFILE"
  fi
  echo "notarizePkgReady=false reason=notary-not-accepted"
  echo "notarizePkgPassed=false reason=notary-not-accepted"
  exit 18
fi

/usr/bin/xcrun stapler staple "$PKG"
/usr/bin/xcrun stapler validate "$PKG"

post_spctl_output="$(/usr/sbin/spctl --assess --type install --verbose=4 "$PKG" 2>&1 || true)"
printf '%s\n' "$post_spctl_output" | /usr/bin/sed 's/^/spctlInstallAssessment: /'
if [[ "$post_spctl_output" != *": accepted"* ]]; then
  echo "notarizePkgReady=false reason=spctl-rejected-after-staple"
  echo "notarizePkgPassed=false reason=spctl-rejected-after-staple"
  exit 19
fi

echo "notarizePkgReady=true"
echo "notarizePkgPassed=true"
