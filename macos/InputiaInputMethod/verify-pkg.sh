#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
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
require_grep 'inputia-select' "$PKG_POSTINSTALL" "postinstall-missing-select"
require_grep 'TextInputMenuAgent' "$PKG_POSTINSTALL" "postinstall-missing-textinputmenuagent-refresh"
require_grep 'SystemUIServer' "$PKG_POSTINSTALL" "postinstall-missing-systemuiserver-refresh"
require_grep 'INPUTIA_POSTINSTALL_SELF_CHECK' "$PKG_POSTINSTALL" "postinstall-missing-self-check"
require_grep 'spctl --assess --type execute' "$PKG_POSTINSTALL" "postinstall-missing-spctl-assessment"
require_grep 'INPUTIA_ALLOW_REJECTED_SIGNATURE' "$PKG_POSTINSTALL" "postinstall-missing-rejected-signature-override"
require_grep 'inputiaPostinstallUsable=false reason=signature-rejected' "$PKG_POSTINSTALL" "postinstall-missing-signature-rejected-gate"
require_grep 'inputiaPostinstallRequiredAction=sign-with-accepted-identity' "$PKG_POSTINSTALL" "postinstall-missing-signature-required-action"
require_grep 'inputiaPostinstallSigningHint=rerun-build-with-INPUTIA_CODESIGN_IDENTITY-that-spctl-accepts' "$PKG_POSTINSTALL" "postinstall-missing-signature-hint"
require_grep 'inputiaPostinstallAction=clear-inputia-preferences' "$PKG_POSTINSTALL" "postinstall-missing-signature-rejected-cleanup-action"
require_grep '--clear-input-source-preferences' "$PKG_POSTINSTALL" "postinstall-missing-clear-input-source-preferences"
require_grep 'inputiaPostRefreshEnabledTIS:' "$PKG_POSTINSTALL" "postinstall-missing-post-refresh-enabled-tis"
require_grep 'inputiaPostRefreshTISReady=' "$PKG_POSTINSTALL" "postinstall-missing-post-refresh-tis-ready"
require_grep 'inputiaPostRefreshCurrentTIS:' "$PKG_POSTINSTALL" "postinstall-missing-post-refresh-current-tis"
require_grep 'inputiaPostRefreshTISStableCheck' "$PKG_POSTINSTALL" "postinstall-missing-post-refresh-stable-check"
require_grep 'inputiaPostRefreshCurrentMatchesTarget=true' "$PKG_POSTINSTALL" "postinstall-missing-current-target-check"
require_order 'inputiaPostinstallSignatureAccepted=true' 'run_best_effort 12 inputia-register ' "$PKG_POSTINSTALL" "postinstall-register-before-signature-gate"
require_order 'inputia-select' 'TextInputMenuAgent' "$PKG_POSTINSTALL" "postinstall-menu-refresh-before-select"
require_order 'TextInputMenuAgent' 'SystemUIServer' "$PKG_POSTINSTALL" "postinstall-systemui-before-textinputmenuagent"
require_order 'SystemUIServer' 'inputia-normalize-after-refresh' "$PKG_POSTINSTALL" "postinstall-normalize-before-systemui"
require_order 'inputia-normalize-after-refresh' 'wait_for_postinstall_tis_selection "$(target_mode_id)"' "$PKG_POSTINSTALL" "postinstall-stable-check-before-normalize"
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
echo "archiveAppVersion=$archive_version"
echo "archiveAppCDHash=$archive_cdhash"
echo "buildAppCDHash=$build_cdhash"
echo "archiveSettingsVersion=$archive_settings_version"
echo "buildSettingsVersion=$build_settings_version"
[[ "$archive_version" == "$build_version" ]] || fail "archive-app-version-mismatch"
[[ -n "$build_cdhash" && "$archive_cdhash" == "$build_cdhash" ]] ||
  fail "archive-app-cdhash-mismatch"
[[ "$archive_settings_version" == "$build_settings_version" ]] ||
  fail "archive-settings-version-mismatch"
[[ -x "$ARCHIVE_APP/Contents/MacOS/InputiaInputMethod" ]] ||
  fail "archive-host-executable-missing"
[[ -x "$ARCHIVE_SETTINGS_APP/Contents/MacOS/InputiaSettingsLauncher" ]] ||
  fail "archive-settings-executable-missing"
echo "archiveExecutables=true"

section "result"
echo "pkgVerificationPassed=true"
