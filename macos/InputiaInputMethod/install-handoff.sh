#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_APP="$ROOT_DIR/build/InputiaInputMethod.app"
SYSTEM_APP="/Library/Input Methods/InputiaInputMethod.app"
PKG_PATH="$ROOT_DIR/dist/InputiaInputMethod-latest.pkg"
HANDOFF_PATH="${INPUTIA_INSTALL_HANDOFF_PATH:-$ROOT_DIR/build/install-handoff.txt}"

quote() {
  /usr/bin/python3 - "$1" <<'PY'
import shlex
import sys

print(shlex.quote(sys.argv[1]))
PY
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
  if [[ -f "$1" ]]; then
    /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
  fi
}

admin_ready=false
admin_reason=admin-required
if [[ -w "/Library/Input Methods" && -w "/Applications" ]]; then
  admin_ready=true
  admin_reason=writable
elif /usr/bin/sudo -n true >/dev/null 2>&1; then
  admin_ready=true
  admin_reason=sudo-noninteractive
fi

/bin/zsh "$ROOT_DIR/build-pkg.sh" >/tmp/inputia-install-handoff-build-pkg.log 2>&1

build_version="$(app_version "$BUILD_APP")"
build_cdhash="$(app_cdhash "$BUILD_APP")"
system_version="$(app_version "$SYSTEM_APP")"
system_cdhash="$(app_cdhash "$SYSTEM_APP")"
pkg_sha256="$(sha256 "$PKG_PATH")"
pkg_quoted="$(quote "$PKG_PATH")"
repo_quoted="$(quote "$ROOT_DIR")"

if [[ -n "$build_cdhash" && "$system_cdhash" == "$build_cdhash" ]]; then
  system_matches_build=true
else
  system_matches_build=false
fi

terminal_installer_command="sudo /usr/sbin/installer -pkg $pkg_quoted -target /"
open_installer_command="/usr/bin/open $pkg_quoted"
install_check_command="cd $repo_quoted && ./install-check.sh"
await_command="cd $repo_quoted && ./await-system-install.sh"

/bin/mkdir -p "$(/usr/bin/dirname "$HANDOFF_PATH")"
/bin/cat >"$HANDOFF_PATH" <<EOF
Inputia 安装交接清单

packagePath=$PKG_PATH
packageSHA256=$pkg_sha256
buildVersion=$build_version
buildCDHash=$build_cdhash
systemVersion=${system_version:-unknown}
systemCDHash=${system_cdhash:-unknown}
systemMatchesBuild=$system_matches_build
adminReady=$admin_ready
adminReason=$admin_reason

管理员终端安装：
$terminal_installer_command

打开 Installer 安装：
$open_installer_command

安装后等待/验证：
$await_command
$install_check_command
EOF

echo "installHandoffReady=true"
echo "installHandoffPath=$HANDOFF_PATH"
echo "packagePath=$PKG_PATH"
echo "packageSHA256=$pkg_sha256"
echo "buildVersion=$build_version"
echo "buildCDHash=$build_cdhash"
echo "systemVersion=${system_version:-unknown}"
echo "systemCDHash=${system_cdhash:-unknown}"
echo "systemMatchesBuild=$system_matches_build"
echo "adminReady=$admin_ready reason=$admin_reason"
echo "terminalInstallerCommand=$terminal_installer_command"
echo "openInstallerCommand=$open_installer_command"
echo "awaitInstallCommand=$await_command"
echo "installCheckCommand=$install_check_command"
