#!/bin/zsh
set -eu
set -o pipefail

append_reason() {
  local reasons="$1"
  local reason="$2"
  if [[ ",$reasons," == *",$reason,"* ]]; then
    echo "$reasons"
  elif [[ -z "$reasons" ]]; then
    echo "$reason"
  else
    echo "$reasons,$reason"
  fi
}

current_uid="$(/usr/bin/id -u)"
current_user_name="$({ /usr/bin/id -un 2>/dev/null || true; } | /usr/bin/head -n 1)"
console_uid="$(/usr/bin/stat -f%u /dev/console 2>/dev/null || echo unknown)"
console_user="$(/usr/bin/stat -f%Su /dev/console 2>/dev/null || echo unknown)"

echo "directoryServiceCurrentUID=$current_uid"
echo "directoryServiceCurrentUserName=${current_user_name:-unknown}"
echo "directoryServiceConsoleUID=$console_uid"
echo "directoryServiceConsoleUser=$console_user"

pwd_output="$(/usr/bin/python3 <<'PY'
import os
import pwd

try:
    record = pwd.getpwuid(os.getuid())
except KeyError:
    print("directoryServicePwdRecord=false")
    print("directoryServicePwdName=unknown")
    print("directoryServicePwdHome=unknown")
else:
    print("directoryServicePwdRecord=true")
    print(f"directoryServicePwdName={record.pw_name}")
    print(f"directoryServicePwdHome={record.pw_dir}")
PY
)"
printf '%s\n' "$pwd_output"
pwd_record="$(/usr/bin/awk -F= '$1 == "directoryServicePwdRecord" { print $2; exit }' <<<"$pwd_output")"

dscache_output="$(/usr/bin/dscacheutil -q user -a uid "$current_uid" 2>&1 || true)"
if [[ -n "$dscache_output" && "$dscache_output" == *"uid: $current_uid"* ]]; then
  echo "directoryServiceDscacheRecord=true"
else
  echo "directoryServiceDscacheRecord=false"
fi

if /bin/ps -axo comm= 2>/dev/null | /usr/bin/awk '$0 == "opendirectoryd" || $0 == "/usr/libexec/opendirectoryd" { found = 1 } END { exit found ? 0 : 1 }'; then
  echo "directoryServiceOpendirectorydProcess=true"
else
  echo "directoryServiceOpendirectorydProcess=false"
fi

odutil_output="$(/usr/bin/odutil show cache statistics 2>&1 || true)"
if [[ "$odutil_output" == *"opendirectoryd not available"* ]]; then
  echo "directoryServiceOdutilAvailable=false"
elif [[ -n "$odutil_output" ]]; then
  echo "directoryServiceOdutilAvailable=true"
else
  echo "directoryServiceOdutilAvailable=unknown"
fi

sudo_output="$(/usr/bin/sudo -n true 2>&1 || true)"
if [[ -z "$sudo_output" ]]; then
  echo "directoryServiceSudoNonInteractive=true"
else
  echo "directoryServiceSudoNonInteractive=false"
  if [[ "$sudo_output" == *"you do not exist in the passwd database"* ]]; then
    echo "directoryServiceSudoBlockReason=missing-passwd-record"
  else
    echo "directoryServiceSudoBlockReason=admin-required"
  fi
fi

reasons=""
if [[ "$pwd_record" != "true" ]]; then
  reasons="$(append_reason "$reasons" "missing-passwd-record")"
fi
if [[ "$odutil_output" == *"opendirectoryd not available"* ]]; then
  reasons="$(append_reason "$reasons" "opendirectoryd-unavailable")"
fi

if [[ -z "$reasons" ]]; then
  echo "directoryServiceReady=true"
  echo "directoryServiceBlockReason=none"
  echo "directoryServiceRequiredAction=none"
else
  echo "directoryServiceReady=false"
  echo "directoryServiceBlockReason=$reasons"
  echo "directoryServiceRequiredAction=repair-current-user-directory-service"
fi
