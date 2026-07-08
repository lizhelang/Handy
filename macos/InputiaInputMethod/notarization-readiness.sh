#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP="${1:-/Library/Input Methods/InputiaInputMethod.app}"
NOTARY_PROFILE="${INPUTIA_NOTARY_PROFILE:-Inputia}"

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

bool_from_rc() {
  local rc="$1"
  if [[ "$rc" -eq 0 ]]; then
    echo true
  else
    echo false
  fi
}

first_codesign_value() {
  local dump="$1"
  local key="$2"
  printf '%s\n' "$dump" | /usr/bin/awk -F= -v key="$key" '$1 == key { print $2; exit }'
}

count_identities() {
  local pattern="$1"
  local identities
  identities="$(/usr/bin/security find-identity -v 2>/dev/null || true)"
  printf '%s\n' "$identities" |
    /usr/bin/awk -v pattern="$pattern" 'index($0, pattern) { count += 1 } END { print count + 0 }'
}

echo "notarizationReadinessTool=true"
echo "app=$APP"

if [[ ! -d "$APP" ]]; then
  echo "appExists=false"
  echo "inputiaGatekeeperReady=false"
  echo "inputiaNotarySubmissionReady=false"
  echo "notarizationReadinessBlockReasons=missing-app"
  echo "notarizationRequiredAction=build-and-install-inputia-app"
  exit 0
fi
echo "appExists=true"

set +e
codesign_verify_output="$(/usr/bin/codesign --verify --deep --strict --verbose=2 "$APP" 2>&1)"
codesign_verify_rc=$?
set -e
echo "codesignVerify=$(bool_from_rc "$codesign_verify_rc")"
if [[ "$codesign_verify_rc" -ne 0 ]]; then
  printf '%s\n' "$codesign_verify_output" | /usr/bin/sed 's/^/codesignVerifyOutput: /'
fi

codesign_dump="$(/usr/bin/codesign -dv --verbose=4 "$APP" 2>&1 || true)"
cdhash="$(first_codesign_value "$codesign_dump" CDHash)"
team_identifier="$(first_codesign_value "$codesign_dump" TeamIdentifier)"
runtime_line="$(printf '%s\n' "$codesign_dump" | /usr/bin/awk -F= '$1 == "Runtime Version" { print $2; exit }')"
echo "appCDHash=${cdhash:-unknown}"
echo "teamIdentifier=${team_identifier:-unknown}"
if [[ -n "$runtime_line" ]]; then
  echo "hardenedRuntime=true"
else
  echo "hardenedRuntime=false"
fi
printf '%s\n' "$codesign_dump" |
  /usr/bin/awk -F= '$1 == "Authority" { print "codesignAuthority="$2 }'

set +e
spctl_output="$(/usr/sbin/spctl --assess --type execute --verbose=4 "$APP" 2>&1)"
spctl_rc=$?
set -e
if [[ "$spctl_rc" -eq 0 && "$spctl_output" == *": accepted"* ]]; then
  spctl_accepted=true
else
  spctl_accepted=false
fi
echo "spctlAccepted=$spctl_accepted"
printf '%s\n' "$spctl_output" | /usr/bin/sed 's/^/spctlAssessment: /'

syspolicy_available=false
syspolicy_notary_ticket_missing=false
if /usr/bin/command -v syspolicy_check >/dev/null 2>&1; then
  syspolicy_available=true
  syspolicy_output="$(syspolicy_check distribution "$APP" 2>&1 || true)"
  if [[ "$syspolicy_output" == *"Notary Ticket Missing"* ]]; then
    syspolicy_notary_ticket_missing=true
  fi
fi
echo "syspolicyCheckAvailable=$syspolicy_available"
echo "syspolicyNotaryTicketMissing=$syspolicy_notary_ticket_missing"

developer_id_application_count="$(count_identities "Developer ID Application:")"
developer_id_installer_count="$(count_identities "Developer ID Installer:")"
echo "developerIDApplicationIdentityCount=$developer_id_application_count"
echo "developerIDApplicationIdentityPresent=$([[ "$developer_id_application_count" -gt 0 ]] && echo true || echo false)"
echo "developerIDInstallerIdentityCount=$developer_id_installer_count"
echo "developerIDInstallerIdentityPresent=$([[ "$developer_id_installer_count" -gt 0 ]] && echo true || echo false)"

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
echo "notaryProfile=$NOTARY_PROFILE"

notary_profile_available=false
if [[ -n "$notarytool_path" ]]; then
  set +e
  notary_profile_output="$(/usr/bin/xcrun notarytool history --keychain-profile "$NOTARY_PROFILE" 2>&1)"
  notary_profile_rc=$?
  set -e
  if [[ "$notary_profile_rc" -eq 0 ]]; then
    notary_profile_available=true
  elif [[ "$notary_profile_output" == *"No Keychain password item found"* ]]; then
    notary_profile_available=false
  else
    notary_profile_available=false
    printf '%s\n' "$notary_profile_output" | /usr/bin/sed 's/^/notaryProfileCheckOutput: /'
  fi
fi
echo "notaryProfileAvailable=$notary_profile_available"

gatekeeper_reasons=""
if [[ "$codesign_verify_rc" -ne 0 ]]; then
  gatekeeper_reasons="$(append_reason "$gatekeeper_reasons" codesign-invalid)"
fi
if [[ "$spctl_accepted" != "true" ]]; then
  gatekeeper_reasons="$(append_reason "$gatekeeper_reasons" spctl-rejected)"
fi
if [[ "$syspolicy_notary_ticket_missing" == "true" ]]; then
  gatekeeper_reasons="$(append_reason "$gatekeeper_reasons" notary-ticket-missing)"
fi

submission_reasons=""
if [[ "$developer_id_application_count" -eq 0 ]]; then
  submission_reasons="$(append_reason "$submission_reasons" missing-developer-id-application)"
fi
if [[ -z "$notarytool_path" ]]; then
  submission_reasons="$(append_reason "$submission_reasons" missing-notarytool)"
fi
if [[ -z "$stapler_path" ]]; then
  submission_reasons="$(append_reason "$submission_reasons" missing-stapler)"
fi
if [[ "$notary_profile_available" != "true" ]]; then
  submission_reasons="$(append_reason "$submission_reasons" missing-notarytool-profile)"
fi

if [[ -z "$gatekeeper_reasons" ]]; then
  echo "inputiaGatekeeperReady=true"
else
  echo "inputiaGatekeeperReady=false"
  echo "inputiaGatekeeperBlockReasons=$gatekeeper_reasons"
fi

if [[ -z "$submission_reasons" ]]; then
  echo "inputiaNotarySubmissionReady=true"
else
  echo "inputiaNotarySubmissionReady=false"
  echo "inputiaNotarySubmissionBlockReasons=$submission_reasons"
fi

combined_reasons="$gatekeeper_reasons"
if [[ -n "$submission_reasons" ]]; then
  IFS=',' read -rA reason_parts <<<"$submission_reasons"
  for reason in "${reason_parts[@]}"; do
    combined_reasons="$(append_reason "$combined_reasons" "$reason")"
  done
fi
if [[ -n "$combined_reasons" ]]; then
  echo "notarizationReadinessBlockReasons=$combined_reasons"
fi

if [[ "$spctl_accepted" == "true" ]]; then
  echo "notarizationRequiredAction=none"
elif [[ "$developer_id_application_count" -eq 0 ]]; then
  echo "notarizationRequiredAction=import-developer-id-application-identity"
elif [[ "$notary_profile_available" != "true" ]]; then
  echo "notarizationRequiredAction=store-notarytool-credentials"
else
  echo "notarizationRequiredAction=submit-notarize-and-staple"
fi
