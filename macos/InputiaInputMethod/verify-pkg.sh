#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$ROOT_DIR/build-artifact-lock.sh"
PKG_PATH="${1:-$ROOT_DIR/dist/InputiaInputMethod-latest.pkg}"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
BUILD_SETTINGS_APP="$ROOT_DIR/build/Inputia 设置.app"
SOURCE_POSTINSTALL="$ROOT_DIR/Packaging/scripts/postinstall"
TMP_ROOT=""

cleanup() {
  if [[ -n "$TMP_ROOT" ]]; then
    /bin/rm -rf "$TMP_ROOT"
  fi
}
trap cleanup EXIT

section() {
  printf '\n== %s ==\n' "$1"
}

fail() {
  echo "pkgVerificationPassed=false reason=$1"
  exit 1
}

plist_value() {
  local plist="$1"
  local key="$2"
  if [[ -f "$plist" ]]; then
    /usr/libexec/PlistBuddy -c "Print :$key" "$plist" 2>/dev/null || true
  fi
}

app_version() {
  plist_value "$1/Contents/Info.plist" CFBundleVersion
}

app_cdhash() {
  if [[ -d "$1" ]]; then
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 |
      /usr/bin/awk -F= '/^CDHash=/{print $2}'
  fi
}

sha256() {
  /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
}

package_info_version() {
  /usr/bin/sed -n 's/.*<pkg-info .* version="\([^"]*\)".*/\1/p' "$1" |
    /usr/bin/head -n 1
}

require_grep() {
  local pattern="$1"
  local path="$2"
  local reason="$3"
  if ! /usr/bin/grep -q -- "$pattern" "$path"; then
    fail "$reason"
  fi
}

reject_grep() {
  local pattern="$1"
  local path="$2"
  local reason="$3"
  if /usr/bin/grep -q -- "$pattern" "$path"; then
    fail "$reason"
  fi
}

require_order() {
  local first_pattern="$1"
  local second_pattern="$2"
  local path="$3"
  local reason="$4"
  local first_line second_line
  first_line="$(/usr/bin/grep -n "$first_pattern" "$path" | /usr/bin/head -n 1 | /usr/bin/cut -d: -f1 || true)"
  second_line="$(/usr/bin/grep -n "$second_pattern" "$path" | /usr/bin/head -n 1 | /usr/bin/cut -d: -f1 || true)"
  if [[ -z "$first_line" || -z "$second_line" || "$first_line" -ge "$second_line" ]]; then
    fail "$reason"
  fi
}

if [[ ! -f "$PKG_PATH" ]]; then
  fail "missing-pkg path=$PKG_PATH"
fi

inputia_build_artifact_acquire_lock verifyPkg
trap 'inputia_build_artifact_release_lock; cleanup' EXIT

TMP_ROOT="$(/usr/bin/mktemp -d /tmp/inputia-pkg-verify.XXXXXX)"
EXPANDED_DIR="$TMP_ROOT/pkg"
ARCHIVE_ROOT="$TMP_ROOT/archive"

section "package"
echo "path=$PKG_PATH"
echo "exists=true"
echo "sha256=$(sha256 "$PKG_PATH")"
if /usr/sbin/pkgutil --check-signature "$PKG_PATH" >/dev/null 2>&1; then
  echo "pkgSignature=present"
else
  echo "pkgSignature=none"
fi

/usr/sbin/pkgutil --expand "$PKG_PATH" "$EXPANDED_DIR"
PACKAGE_INFO="$EXPANDED_DIR/PackageInfo"
PKG_POSTINSTALL="$EXPANDED_DIR/Scripts/postinstall"
PKG_APP_ARCHIVE="$EXPANDED_DIR/Scripts/InputiaInputMethod.app.tar.gz"
PKG_SETTINGS_ARCHIVE="$EXPANDED_DIR/Scripts/InputiaSettings.app.tar.gz"

section "package info"
pkg_version="$(package_info_version "$PACKAGE_INFO")"
build_version="$(app_version "$BUILD_APP")"
echo "packageVersion=$pkg_version"
echo "buildVersion=$build_version"
[[ -n "$build_version" && "$pkg_version" == "$build_version" ]] ||
  fail "package-version-mismatch"

section "scripts"
if [[ ! -x "$PKG_POSTINSTALL" ]]; then
  fail "postinstall-not-executable"
fi
echo "postinstallExecutable=true"
source_postinstall_sha="$(sha256 "$SOURCE_POSTINSTALL")"
pkg_postinstall_sha="$(sha256 "$PKG_POSTINSTALL")"
echo "sourcePostinstallSHA256=$source_postinstall_sha"
echo "pkgPostinstallSHA256=$pkg_postinstall_sha"
[[ "$pkg_postinstall_sha" == "$source_postinstall_sha" ]] ||
  fail "postinstall-sha-mismatch"

require_grep 'INPUTIA_POSTINSTALL_SKIP_TIS' "$PKG_POSTINSTALL" "postinstall-missing-skip-tis"
require_grep 'inputiaUserHostRemoved=true' "$PKG_POSTINSTALL" "postinstall-missing-user-host-cleanup"
require_grep 'INPUTIA_POSTINSTALL_SELF_CHECK' "$PKG_POSTINSTALL" "postinstall-missing-self-check"
require_grep 'spctl --assess --type execute' "$PKG_POSTINSTALL" "postinstall-missing-spctl-assessment"
require_grep 'INPUTIA_ALLOW_REJECTED_SIGNATURE' "$PKG_POSTINSTALL" "postinstall-missing-rejected-signature-override"
require_grep 'inputiaPostinstallUsable=false reason=signature-rejected' "$PKG_POSTINSTALL" "postinstall-missing-signature-rejected-gate"
require_grep 'inputiaPostinstallRequiredAction=sign-with-accepted-identity' "$PKG_POSTINSTALL" "postinstall-missing-signature-required-action"
require_grep 'inputiaPostinstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts' "$PKG_POSTINSTALL" "postinstall-missing-signature-hint"
require_grep 'inputiaPostinstallAction=stop-before-tis-registration' "$PKG_POSTINSTALL" "postinstall-missing-signature-rejected-stop-action"
require_grep 'inputiaPostinstallEnabledTIS:' "$PKG_POSTINSTALL" "postinstall-missing-enabled-tis-dump"
require_grep 'inputiaPostinstallCurrentTIS:' "$PKG_POSTINSTALL" "postinstall-missing-current-tis-dump"
require_grep 'inputiaPostinstallCurrentMatchesTarget=' "$PKG_POSTINSTALL" "postinstall-missing-current-target-summary"
require_grep 'inputiaPostinstallRegistered=true' "$PKG_POSTINSTALL" "postinstall-missing-registered-summary"
require_grep 'inputiaPostinstallTISReady=false reason=manual-add-required' "$PKG_POSTINSTALL" "postinstall-missing-manual-add-tis-ready"
require_grep 'inputiaPostinstallRequiredAction=add-input-source-in-system-settings' "$PKG_POSTINSTALL" "postinstall-missing-manual-add-required-action"
require_grep 'inputiaPostinstallNextStep=System Settings > Keyboard > Text Input > Edit > Add Inputia' "$PKG_POSTINSTALL" "postinstall-missing-manual-add-next-step"
reject_grep '--clear-input-source-preferences' "$PKG_POSTINSTALL" "postinstall-still-clears-input-source-preferences"
reject_grep '--enable-input-source' "$PKG_POSTINSTALL" "postinstall-still-enables-input-source"
reject_grep '--normalize-hitoolbox' "$PKG_POSTINSTALL" "postinstall-still-normalizes-hitoolbox"
reject_grep '--select-input-source' "$PKG_POSTINSTALL" "postinstall-still-selects-input-source"
reject_grep 'TextInputMenuAgent' "$PKG_POSTINSTALL" "postinstall-still-restarts-textinputmenuagent"
reject_grep 'SystemUIServer' "$PKG_POSTINSTALL" "postinstall-still-restarts-systemuiserver"
require_order 'inputiaPostinstallSignatureAccepted=true' 'run_best_effort 12 inputia-register ' "$PKG_POSTINSTALL" "postinstall-register-before-signature-gate"
require_order 'run_best_effort 12 inputia-register ' 'inputiaPostinstallEnabledTIS:' "$PKG_POSTINSTALL" "postinstall-tis-dump-before-register"
require_order 'inputiaPostinstallCurrentTIS:' 'inputiaPostinstallRequiredAction=add-input-source-in-system-settings' "$PKG_POSTINSTALL" "postinstall-manual-action-before-current-dump"
echo "postinstallBehaviorChecks=true"

section "postinstall self-check"
postinstall_self_check_output="$(INPUTIA_POSTINSTALL_SELF_CHECK=1 "$PKG_POSTINSTALL")"
printf '%s\n' "$postinstall_self_check_output" | /usr/bin/sed 's/^/postinstallSelfCheck: /'
for case_name in ready missing id-mismatch not-enabled not-selectable icon-mismatch; do
  if ! /usr/bin/grep -q "inputiaPostinstallSelfCheckCase=$case_name passed=true" <<<"$postinstall_self_check_output"; then
    fail "postinstall-self-check-missing-case-$case_name"
  fi
done
if ! /usr/bin/grep -q '^inputiaPostinstallSelfCheck=true$' <<<"$postinstall_self_check_output"; then
  fail "postinstall-self-check-missing-success"
fi

section "embedded archives"
[[ -f "$PKG_APP_ARCHIVE" ]] || fail "missing-app-archive"
[[ -f "$PKG_SETTINGS_ARCHIVE" ]] || fail "missing-settings-archive"
echo "appArchivePresent=true"
echo "settingsArchivePresent=true"
/bin/mkdir -p "$ARCHIVE_ROOT"
COPYFILE_DISABLE=1 /usr/bin/tar -xzf "$PKG_APP_ARCHIVE" -C "$ARCHIVE_ROOT"
COPYFILE_DISABLE=1 /usr/bin/tar -xzf "$PKG_SETTINGS_ARCHIVE" -C "$ARCHIVE_ROOT"

ARCHIVE_APP="$ARCHIVE_ROOT/InputiaInputMethod.app"
ARCHIVE_SETTINGS_APP="$ARCHIVE_ROOT/Inputia 设置.app"
archive_version="$(app_version "$ARCHIVE_APP")"
archive_cdhash="$(app_cdhash "$ARCHIVE_APP")"
build_cdhash="$(app_cdhash "$BUILD_APP")"
archive_settings_version="$(app_version "$ARCHIVE_SETTINGS_APP")"
build_settings_version="$(app_version "$BUILD_SETTINGS_APP")"
archive_settings_expected_host_cdhash="$(plist_value "$ARCHIVE_SETTINGS_APP/Contents/Info.plist" InputiaExpectedHostCDHash)"
build_settings_expected_host_cdhash="$(plist_value "$BUILD_SETTINGS_APP/Contents/Info.plist" InputiaExpectedHostCDHash)"
build_source_commit="$(plist_value "$BUILD_APP/Contents/Info.plist" InputiaSourceCommit)"
archive_source_commit="$(plist_value "$ARCHIVE_APP/Contents/Info.plist" InputiaSourceCommit)"
archive_settings_source_commit="$(plist_value "$ARCHIVE_SETTINGS_APP/Contents/Info.plist" InputiaSourceCommit)"
archive_source_branch="$(plist_value "$ARCHIVE_APP/Contents/Info.plist" InputiaSourceBranch)"
archive_settings_source_branch="$(plist_value "$ARCHIVE_SETTINGS_APP/Contents/Info.plist" InputiaSourceBranch)"
archive_source_dirty="$(plist_value "$ARCHIVE_APP/Contents/Info.plist" InputiaSourceDirty)"
archive_settings_source_dirty="$(plist_value "$ARCHIVE_SETTINGS_APP/Contents/Info.plist" InputiaSourceDirty)"
echo "archiveAppVersion=$archive_version"
echo "archiveAppCDHash=$archive_cdhash"
echo "buildAppCDHash=$build_cdhash"
echo "archiveSettingsVersion=$archive_settings_version"
echo "buildSettingsVersion=$build_settings_version"
echo "archiveSettingsExpectedHostCDHash=${archive_settings_expected_host_cdhash:-unknown}"
echo "buildSettingsExpectedHostCDHash=${build_settings_expected_host_cdhash:-unknown}"
echo "buildSourceCommit=${build_source_commit:-unknown}"
echo "archiveSourceCommit=${archive_source_commit:-unknown}"
echo "archiveSettingsSourceCommit=${archive_settings_source_commit:-unknown}"
echo "archiveSourceBranch=${archive_source_branch:-unknown}"
echo "archiveSettingsSourceBranch=${archive_settings_source_branch:-unknown}"
echo "archiveSourceDirty=${archive_source_dirty:-unknown}"
echo "archiveSettingsSourceDirty=${archive_settings_source_dirty:-unknown}"
[[ "$archive_version" == "$build_version" ]] || fail "archive-app-version-mismatch"
[[ -n "$build_cdhash" && "$archive_cdhash" == "$build_cdhash" ]] ||
  fail "archive-app-cdhash-mismatch"
[[ "$archive_settings_version" == "$build_settings_version" ]] ||
  fail "archive-settings-version-mismatch"
[[ "$build_settings_expected_host_cdhash" == "$build_cdhash" ]] ||
  fail "build-settings-expected-host-cdhash-mismatch"
[[ "$archive_settings_expected_host_cdhash" == "$archive_cdhash" ]] ||
  fail "archive-settings-expected-host-cdhash-mismatch"
[[ -n "$build_source_commit" ]] || fail "build-source-commit-missing"
[[ "$archive_source_commit" == "$build_source_commit" ]] ||
  fail "archive-source-commit-mismatch"
[[ "$archive_settings_source_commit" == "$archive_source_commit" ]] ||
  fail "archive-settings-source-commit-mismatch"
[[ "$archive_settings_source_branch" == "$archive_source_branch" ]] ||
  fail "archive-settings-source-branch-mismatch"
[[ "$archive_settings_source_dirty" == "$archive_source_dirty" ]] ||
  fail "archive-settings-source-dirty-mismatch"
[[ -x "$ARCHIVE_APP/Contents/MacOS/InputiaInputMethod" ]] ||
  fail "archive-host-executable-missing"
[[ -x "$ARCHIVE_SETTINGS_APP/Contents/MacOS/InputiaSettingsLauncher" ]] ||
  fail "archive-settings-executable-missing"
echo "archiveExecutables=true"

section "result"
echo "pkgVerificationPassed=true"
