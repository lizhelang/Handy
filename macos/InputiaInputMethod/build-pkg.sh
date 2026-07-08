#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_NAME="InputiaInputMethod.app"
SETTINGS_APP_NAME="Inputia 设置.app"
APP_DIR="$ROOT_DIR/build/$APP_NAME"
SETTINGS_APP_DIR="$ROOT_DIR/build/$SETTINGS_APP_NAME"
PKG_SCRIPTS_DIR="$ROOT_DIR/build/pkg-scripts"
DIST_DIR="$ROOT_DIR/dist"
PKG_ID="com.inputia.inputmethod.Inputia.pkg"
VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$ROOT_DIR/Info.plist")"

export COPYFILE_DISABLE=1

detect_verification_processes() {
  local process_list
  if [[ -n "${INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST:-}" ]]; then
    process_list="$INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST"
  else
    process_list="$(/bin/ps -axo pid=,command=)"
  fi
  printf '%s\n' "$process_list" |
    /usr/bin/awk -v root="$ROOT_DIR" -v self="$$" '
      $1 == self { next }
      index($0, root) &&
        $0 ~ /\/(verify-nongui|post-install-regression|verify-system|verify-pkg|await-system-install|smoke-preflight|smoke-textedit|smoke-textedit-command-shortcuts|smoke-clipboard-recall|smoke-safari[^ ]*|diagnose-safari-input-source|gui-smoke-readiness|gui-smoke-suite|status|tis-readiness)\.sh( |$)/ {
          print
        }
    '
}

require_no_verification_processes() {
  local blocking_processes
  blocking_processes="$(detect_verification_processes)"
  if [[ -n "$blocking_processes" ]]; then
    echo "buildPkgReady=false reason=verification-running"
    printf '%s\n' "$blocking_processes" | /usr/bin/sed 's/^/buildPkgBlockingProcess: /'
    exit 20
  fi
}

if [[ "${INPUTIA_BUILD_PKG_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  original_process_list="${INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST:-}"
  INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST="123 /usr/bin/true"
  clear_processes="$(detect_verification_processes)"
  INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/gui-smoke-suite.sh"
  blocked_processes="$(detect_verification_processes)"
  INPUTIA_BUILD_PKG_PROCESS_LIST_FOR_TEST="$original_process_list"
  if [[ -z "$clear_processes" && -n "$blocked_processes" ]]; then
    echo "buildPkgPreflightSelfCheck clear=true"
    echo "buildPkgPreflightSelfCheck blocked=true"
    echo "buildPkgPreflightSelfCheck=true"
    exit 0
  fi
  echo "buildPkgPreflightSelfCheck=false"
  exit 1
fi

require_no_verification_processes

/bin/zsh "$ROOT_DIR/build.sh" >/dev/null

APP_CDHASH="$(/usr/bin/codesign -dv --verbose=4 "$APP_DIR" 2>&1 | /usr/bin/awk -F= '/^CDHash=/{print $2}')"
APP_CDHASH_SHORT="$(/usr/bin/printf '%.12s' "$APP_CDHASH")"
PKG_PATH="$DIST_DIR/InputiaInputMethod-v${VERSION}-${APP_CDHASH_SHORT}.pkg"
LATEST_PKG_PATH="$DIST_DIR/InputiaInputMethod-latest.pkg"

rm -rf "$PKG_SCRIPTS_DIR" "$DIST_DIR"
mkdir -p "$PKG_SCRIPTS_DIR" "$DIST_DIR"
cp "$ROOT_DIR/Packaging/scripts/postinstall" "$PKG_SCRIPTS_DIR/postinstall"
chmod +x "$PKG_SCRIPTS_DIR/postinstall"
COPYFILE_DISABLE=1 /usr/bin/tar -czf "$PKG_SCRIPTS_DIR/InputiaInputMethod.app.tar.gz" \
  -C "$ROOT_DIR/build" \
  "$APP_NAME"
COPYFILE_DISABLE=1 /usr/bin/tar -czf "$PKG_SCRIPTS_DIR/InputiaSettings.app.tar.gz" \
  -C "$ROOT_DIR/build" \
  "$SETTINGS_APP_NAME"
/usr/bin/xattr -cr "$PKG_SCRIPTS_DIR" >/dev/null 2>&1 || true

/usr/bin/pkgbuild \
  --nopayload \
  --scripts "$PKG_SCRIPTS_DIR" \
  --identifier "$PKG_ID" \
  --version "$VERSION" \
  --install-location "/" \
  "$PKG_PATH"

if [[ -n "${INPUTIA_PKG_SIGN_IDENTITY:-}" ]]; then
  signed_pkg="$DIST_DIR/InputiaInputMethod-v${VERSION}-${APP_CDHASH_SHORT}-signed.pkg"
  /usr/bin/productsign --sign "$INPUTIA_PKG_SIGN_IDENTITY" "$PKG_PATH" "$signed_pkg"
  mv "$signed_pkg" "$PKG_PATH"
fi

/bin/cp "$PKG_PATH" "$LATEST_PKG_PATH"

/usr/sbin/pkgutil --check-signature "$PKG_PATH" || true
if /usr/sbin/pkgutil --payload-files "$PKG_PATH" | grep -q .; then
  /usr/sbin/pkgutil --payload-files "$PKG_PATH" | sed -n '1,80p'
else
  echo "payloadFiles=0"
fi
echo "appCDHash=$APP_CDHASH"
if [[ "${INPUTIA_SKIP_PKG_VERIFY:-0}" != "1" ]]; then
  /bin/zsh "$ROOT_DIR/verify-pkg.sh" "$LATEST_PKG_PATH"
else
  echo "pkgVerificationSkipped=true"
fi
echo "$PKG_PATH"
