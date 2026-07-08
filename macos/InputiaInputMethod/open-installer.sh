#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"

detect_verification_processes() {
  local process_list
  if [[ -n "${INPUTIA_OPEN_INSTALLER_PROCESS_LIST_FOR_TEST:-}" ]]; then
    process_list="$INPUTIA_OPEN_INSTALLER_PROCESS_LIST_FOR_TEST"
  else
    process_list="$(/bin/ps -axo pid=,command=)"
  fi
  printf '%s\n' "$process_list" |
    /usr/bin/awk -v root="$ROOT_DIR" -v self="$$" -v owner="${INPUTIA_VERIFICATION_OWNER_PID:-}" '
      $1 == self { next }
      owner != "" && $1 == owner { next }
      index($0, root) &&
        $0 ~ /\/(dev-fast|install-check|release\/full-check|verify-nongui|post-install-regression|verify-system|verify-pkg|await-system-install|smoke-preflight|smoke-textedit|smoke-textedit-command-shortcuts|smoke-clipboard-recall|smoke-safari[^ ]*|diagnose-safari-input-source|gui-smoke-readiness|gui-smoke-suite|status|tis-readiness)\.sh( |$)/ {
          print
        }
    '
}

require_no_verification_processes() {
  local blocking_processes
  blocking_processes="$(detect_verification_processes)"
  if [[ -n "$blocking_processes" ]]; then
    echo "openInstallerReady=false reason=verification-running"
    printf '%s\n' "$blocking_processes" | /usr/bin/sed 's/^/openInstallerBlockingProcess: /'
    exit 20
  fi
}

if [[ "${INPUTIA_OPEN_INSTALLER_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  original_process_list="${INPUTIA_OPEN_INSTALLER_PROCESS_LIST_FOR_TEST:-}"
  INPUTIA_OPEN_INSTALLER_PROCESS_LIST_FOR_TEST="123 /usr/bin/true"
  clear_processes="$(detect_verification_processes)"
  INPUTIA_OPEN_INSTALLER_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/release/full-check.sh"
  blocked_processes="$(detect_verification_processes)"
  INPUTIA_OPEN_INSTALLER_PROCESS_LIST_FOR_TEST="$original_process_list"
  if [[ -z "$clear_processes" && -n "$blocked_processes" ]]; then
    echo "openInstallerPreflightSelfCheck clear=true"
    echo "openInstallerPreflightSelfCheck blocked=true"
    echo "openInstallerPreflightSelfCheck=true"
    exit 0
  fi
  echo "openInstallerPreflightSelfCheck=false"
  exit 1
fi

require_no_verification_processes

pkg_path="$(
  /bin/zsh "$ROOT_DIR/build-pkg.sh" |
    /usr/bin/tee /dev/stderr |
    /usr/bin/tail -n 1
)"

if [[ ! -f "$pkg_path" ]]; then
  echo "installerPackageFound=false path=$pkg_path" >&2
  exit 1
fi

echo "openingInstallerPackage=$pkg_path"
/usr/bin/open "$pkg_path"
