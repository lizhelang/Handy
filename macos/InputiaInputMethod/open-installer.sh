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

open_package() {
  local pkg_path="$1"
  if [[ "${INPUTIA_OPEN_INSTALLER_DRY_RUN:-0}" == "1" ]]; then
    echo "openInstallerDryRun=true path=$pkg_path"
  else
    /usr/bin/open "$pkg_path"
  fi
}

build_or_test_pkg_path() {
  if [[ -n "${INPUTIA_OPEN_INSTALLER_PACKAGE_FOR_TEST:-}" ]]; then
    echo "$INPUTIA_OPEN_INSTALLER_PACKAGE_FOR_TEST"
    return 0
  fi
  /bin/zsh "$ROOT_DIR/build-pkg.sh" |
    /usr/bin/tee /dev/stderr |
    /usr/bin/tail -n 1
}

if [[ "${INPUTIA_OPEN_INSTALLER_DRY_RUN_SELF_CHECK:-0}" == "1" ]]; then
  self_check_root="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-open-installer-self-check.XXXXXX")"
  trap '/bin/rm -rf "$self_check_root" >/dev/null 2>&1 || true' EXIT
  fake_pkg="$self_check_root/InputiaInputMethod.pkg"
  : > "$fake_pkg"
  dry_run_output="$(
    INPUTIA_OPEN_INSTALLER_DRY_RUN_SELF_CHECK=0 \
      INPUTIA_OPEN_INSTALLER_PACKAGE_FOR_TEST="$fake_pkg" \
      INPUTIA_OPEN_INSTALLER_DRY_RUN=1 \
      "$0" 2>&1
  )"
  if ! /usr/bin/grep -q "^openingInstallerPackage=$fake_pkg$" <<<"$dry_run_output"; then
    echo "openInstallerDryRunSelfCheck=false reason=missing-opening-package"
    exit 1
  fi
  if ! /usr/bin/grep -q "^openInstallerDryRun=true path=$fake_pkg$" <<<"$dry_run_output"; then
    echo "openInstallerDryRunSelfCheck=false reason=missing-dry-run-open"
    exit 1
  fi
  echo "openInstallerDryRunSelfCheck=true"
  exit 0
fi

require_no_verification_processes

pkg_path="$(build_or_test_pkg_path)"

if [[ ! -f "$pkg_path" ]]; then
  echo "installerPackageFound=false path=$pkg_path" >&2
  exit 1
fi

echo "openingInstallerPackage=$pkg_path"
open_package "$pkg_path"
