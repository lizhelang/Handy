#!/bin/zsh
set -eu
set -o pipefail
umask 022

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$ROOT_DIR/build-artifact-lock.sh"
BUILD_DIR="$ROOT_DIR/build"
APP_DIR="$BUILD_DIR/InputiaInputMethod.app"
SETTINGS_APP_DIR="$BUILD_DIR/Inputia 设置.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
SETTINGS_CONTENTS_DIR="$SETTINGS_APP_DIR/Contents"
SETTINGS_MACOS_DIR="$SETTINGS_CONTENTS_DIR/MacOS"
SETTINGS_RESOURCES_DIR="$SETTINGS_CONTENTS_DIR/Resources"
RIME_DATA_BUILD_DIR="$BUILD_DIR/RimeData"
SIGN_IDENTITY="${INPUTIA_CODESIGN_IDENTITY:--}"
if [[ -n "${INPUTIA_CODESIGN_OPTIONS+x}" ]]; then
  SIGN_OPTIONS="$INPUTIA_CODESIGN_OPTIONS"
elif [[ "$SIGN_IDENTITY" == "-" ]]; then
  SIGN_OPTIONS=""
else
  SIGN_OPTIONS="--options runtime"
fi
ENTITLEMENTS="${INPUTIA_CODESIGN_ENTITLEMENTS:-$ROOT_DIR/InputiaInputMethod.entitlements}"
if [[ "${INPUTIA_CODESIGN_AS_ROOT:-0}" == "1" ]]; then
  CODESIGN_KEYCHAIN="${INPUTIA_CODESIGN_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
fi
BUILD_USER="$(/usr/bin/id -un)"
BUILD_GROUP="$(/usr/bin/id -gn)"
TARGET_TRIPLE="$(uname -m)-apple-macos13.0"
CAPI_MANIFEST="$ROOT_DIR/../../crates/inputia-capi/Cargo.toml"
CAPI_LIB="$ROOT_DIR/../../crates/inputia-capi/target/release/libinputia_capi.a"
RUST_TOOLCHAIN="${INPUTIA_RUST_TOOLCHAIN:-1.96.0}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"

run_cargo() {
  if [[ -n "${CARGO:-}" ]]; then
    "$CARGO" "$@"
  elif /usr/bin/command -v rustup >/dev/null 2>&1; then
    rustup run "$RUST_TOOLCHAIN" cargo "$@"
  else
    cargo "$@"
  fi
}

detect_verification_processes() {
  local process_list
  if [[ -n "${INPUTIA_BUILD_PROCESS_LIST_FOR_TEST:-}" ]]; then
    process_list="$INPUTIA_BUILD_PROCESS_LIST_FOR_TEST"
  else
    process_list="$(/bin/ps -axo pid=,command=)"
  fi
  printf '%s\n' "$process_list" |
    /usr/bin/awk -v root="$ROOT_DIR" -v self="$$" -v owner="${INPUTIA_VERIFICATION_OWNER_PID:-}" '
      $1 == self { next }
      owner != "" && $1 == owner { next }
      $0 ~ /SkyComputerUseClient/ { next }
      $0 ~ /notify-hook\.js/ { next }
      $0 ~ /agent-turn-complete/ { next }
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
    echo "buildReady=false reason=verification-running"
    printf '%s\n' "$blocking_processes" | /usr/bin/sed 's/^/buildBlockingProcess: /'
    exit 20
  fi
}

if [[ "${INPUTIA_BUILD_PREFLIGHT_SELF_CHECK:-0}" == "1" ]]; then
  original_process_list="${INPUTIA_BUILD_PROCESS_LIST_FOR_TEST:-}"
  INPUTIA_BUILD_PROCESS_LIST_FOR_TEST="123 /usr/bin/true"
  clear_processes="$(detect_verification_processes)"
  INPUTIA_BUILD_PROCESS_LIST_FOR_TEST="456 $ROOT_DIR/dev-fast.sh"
  blocked_processes="$(detect_verification_processes)"
  INPUTIA_BUILD_PROCESS_LIST_FOR_TEST="$original_process_list"
  if [[ -z "$clear_processes" && -n "$blocked_processes" ]]; then
    echo "buildPreflightSelfCheck clear=true"
    echo "buildPreflightSelfCheck blocked=true"
    echo "buildPreflightSelfCheck=true"
    exit 0
  fi
  echo "buildPreflightSelfCheck=false"
  exit 1
fi

inputia_build_artifact_acquire_lock build
trap inputia_build_artifact_release_lock EXIT
require_no_verification_processes

rm -rf "$APP_DIR" "$SETTINGS_APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR" "$SETTINGS_MACOS_DIR" "$SETTINGS_RESOURCES_DIR"

if [[ ! -f "$CAPI_MANIFEST" ]]; then
  echo "missing inputia-capi manifest: $CAPI_MANIFEST" >&2
  exit 1
fi

run_cargo build --release --manifest-path "$CAPI_MANIFEST"
if [[ ! -f "$CAPI_LIB" ]]; then
  echo "missing inputia-capi staticlib: $CAPI_LIB" >&2
  exit 1
fi

/usr/bin/swiftc \
  "$ROOT_DIR/Sources/InputiaInputMethod/main.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHostTextPolicy.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHandyMemorySync.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaInputTextRouter.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceInputLauncher.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaCandidatePanel.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaSettingsWindow.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "$CAPI_LIB" \
  -parse-as-library \
  -target "$TARGET_TRIPLE" \
  -module-name InputiaInputMethod \
  -framework Cocoa \
  -framework InputMethodKit \
  -o "$MACOS_DIR/InputiaInputMethod"

cp "$ROOT_DIR/Info.plist" "$CONTENTS_DIR/Info.plist"
cp -R "$ROOT_DIR/Resources/." "$RESOURCES_DIR/"
INPUTIA_RIME_DATA_BUILD_DIR="$RIME_DATA_BUILD_DIR" "$ROOT_DIR/prepare-rime-data.sh" >/dev/null
cp -R "$RIME_DATA_BUILD_DIR" "$RESOURCES_DIR/RimeData"
/usr/bin/plutil -lint "$CONTENTS_DIR/Info.plist"

/usr/bin/swiftc \
  "$ROOT_DIR/SettingsLauncher/main.swift" \
  -parse-as-library \
  -target "$TARGET_TRIPLE" \
  -module-name InputiaSettingsLauncher \
  -framework AppKit \
  -o "$SETTINGS_MACOS_DIR/InputiaSettingsLauncher"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaShortcutSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaInputTextRouter.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-shortcut-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaTISTool.swift" \
  -parse-as-library \
  -target "$TARGET_TRIPLE" \
  -framework Carbon \
  -o "$BUILD_DIR/inputia-tis-tool"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaInputTextRouterSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaInputTextRouter.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "$CAPI_LIB" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-input-text-router-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaHandyMemorySyncSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHandyMemorySync.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "$CAPI_LIB" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-handy-memory-sync-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaVoiceInputLauncherSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceInputLauncher.swift" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-voice-input-launcher-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaHostTextPolicySelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHostTextPolicy.swift" \
  -target "$TARGET_TRIPLE" \
  -framework Foundation \
  -o "$BUILD_DIR/inputia-host-text-policy-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaBridgePrivacySelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "$CAPI_LIB" \
  -target "$TARGET_TRIPLE" \
  -framework Foundation \
  -o "$BUILD_DIR/inputia-bridge-privacy-self-check"

cp "$ROOT_DIR/SettingsLauncher/Info.plist" "$SETTINGS_CONTENTS_DIR/Info.plist"
cp "$ROOT_DIR/Resources/Inputia.icns" "$SETTINGS_RESOURCES_DIR/Inputia.icns"
/usr/bin/plutil -lint "$SETTINGS_CONTENTS_DIR/Info.plist"

/usr/bin/find "$APP_DIR" "$SETTINGS_APP_DIR" -type d -exec /bin/chmod 755 {} +
/usr/bin/find "$APP_DIR" "$SETTINGS_APP_DIR" -type f -exec /bin/chmod 644 {} +
/bin/chmod 755 "$MACOS_DIR/InputiaInputMethod" "$SETTINGS_MACOS_DIR/InputiaSettingsLauncher"

codesign_args=(--force --sign "$SIGN_IDENTITY")
if [[ -n "$SIGN_OPTIONS" ]]; then
  extra_sign_options=(${=SIGN_OPTIONS})
  codesign_args+=("${extra_sign_options[@]}")
fi
if [[ -f "$ENTITLEMENTS" ]]; then
  codesign_args+=(--entitlements "$ENTITLEMENTS")
fi

repair_root_codesign_artifacts() {
  local bundle="$1"
  local signature_dir="$bundle/Contents/_CodeSignature"
  if [[ "${INPUTIA_CODESIGN_AS_ROOT:-0}" != "1" || ! -d "$signature_dir" ]]; then
    return
  fi
  /usr/bin/sudo -n /usr/sbin/chown -R "$BUILD_USER:$BUILD_GROUP" "$signature_dir"
  /usr/bin/find "$signature_dir" -type d -exec /bin/chmod 755 {} +
  /usr/bin/find "$signature_dir" -type f -exec /bin/chmod 644 {} +
}

run_codesign() {
  if [[ "${INPUTIA_CODESIGN_AS_ROOT:-0}" == "1" ]]; then
    if [[ -n "${INPUTIA_SUDO_PASSWORD:-}" ]]; then
      /usr/bin/printf '%s\n' "$INPUTIA_SUDO_PASSWORD" |
        /usr/bin/sudo -S -p '' /usr/bin/codesign --keychain "$CODESIGN_KEYCHAIN" "$@"
    else
      /usr/bin/sudo -n /usr/bin/codesign --keychain "$CODESIGN_KEYCHAIN" "$@"
    fi
  else
    /usr/bin/codesign "$@"
  fi
}

codesign_output="$(/usr/bin/mktemp "${TMPDIR:-/tmp}/inputia-codesign.XXXXXX")"
if run_codesign "${codesign_args[@]}" "$APP_DIR" >"$codesign_output" 2>&1; then
  /bin/rm -f "$codesign_output"
  repair_root_codesign_artifacts "$APP_DIR"
  /usr/bin/codesign --verify --deep --strict --verbose=2 "$APP_DIR"
else
  /usr/bin/sed 's/^/codesignOutput: /' "$codesign_output" >&2 || true
  /bin/rm -f "$codesign_output"
  echo "warning: codesign failed with identity '$SIGN_IDENTITY'; build artifact still exists at $APP_DIR" >&2
  if [[ "$SIGN_IDENTITY" != "-" ]]; then
    echo "buildSigned=false reason=codesign-failed target=input-method identity=$SIGN_IDENTITY" >&2
    exit 31
  fi
fi

codesign_output="$(/usr/bin/mktemp "${TMPDIR:-/tmp}/inputia-codesign.XXXXXX")"
if run_codesign "${codesign_args[@]}" "$SETTINGS_APP_DIR" >"$codesign_output" 2>&1; then
  /bin/rm -f "$codesign_output"
  repair_root_codesign_artifacts "$SETTINGS_APP_DIR"
  /usr/bin/codesign --verify --deep --strict --verbose=2 "$SETTINGS_APP_DIR"
else
  /usr/bin/sed 's/^/codesignOutput: /' "$codesign_output" >&2 || true
  /bin/rm -f "$codesign_output"
  echo "warning: codesign failed with identity '$SIGN_IDENTITY'; settings launcher still exists at $SETTINGS_APP_DIR" >&2
  if [[ "$SIGN_IDENTITY" != "-" ]]; then
    echo "buildSigned=false reason=codesign-failed target=settings-launcher identity=$SIGN_IDENTITY" >&2
    exit 32
  fi
fi

"$LSREGISTER" -u "$APP_DIR" >/dev/null 2>&1 || true
"$LSREGISTER" -u "$SETTINGS_APP_DIR" >/dev/null 2>&1 || true

echo "$APP_DIR"
echo "$SETTINGS_APP_DIR"
