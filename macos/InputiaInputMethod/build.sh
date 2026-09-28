#!/bin/zsh
set -eu
set -o pipefail
umask 022

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$ROOT_DIR/build-artifact-lock.sh"
IS_CANDIDATE="${INPUTIA_UNIFIED_CANDIDATE:-0}"
RUN_ID="${INPUTIA_PROFILE_RUN_ID:-}"
IS_RELEASE="${INPUTIA_RELEASE:-0}"
if [[ "$IS_RELEASE" != "0" && "$IS_RELEASE" != "1" ]]; then
  echo "INPUTIA_RELEASE must be 0 or 1" >&2
  exit 2
fi
# 正式版继续使用已授权的配对身份和数据域，不通过改名绕过认证。
if [[ "$IS_RELEASE" == "1" ]]; then
  if [[ "$IS_CANDIDATE" != "1" || -z "${INPUTIA_PAIR_BUILD_METADATA:-}" || -z "${INPUTIA_CODESIGN_IDENTITY:-}" || "$INPUTIA_CODESIGN_IDENTITY" == "-" ]]; then
    echo "release requires paired candidate mode, public build metadata and a persistent signing certificate" >&2
    exit 2
  fi
fi
if [[ "$IS_CANDIDATE" != "0" && "$IS_CANDIDATE" != "1" ]]; then
  echo "INPUTIA_UNIFIED_CANDIDATE must be 0 or 1" >&2
  exit 2
fi
if [[ "$IS_CANDIDATE" == "1" ]]; then
  if [[ ! "$RUN_ID" =~ '^[A-Za-z0-9_-]{1,64}$' ]]; then
    echo "candidate requires a safe explicit profile run ID" >&2
    exit 2
  fi
  BUILD_DIR="$ROOT_DIR/candidate-builds/$RUN_ID"
  if [[ -n "${INPUTIA_BUILD_DIR:-}" && "$INPUTIA_BUILD_DIR" != "$BUILD_DIR" ]]; then
    echo "candidate build directory must match its repository-owned run directory" >&2
    exit 2
  fi
  APP_DIR="$BUILD_DIR/InputiaUnifiedCandidate.app"
  SETTINGS_APP_DIR="$BUILD_DIR/Inputia 候选设置.app"
else
  if [[ -n "$RUN_ID" ]]; then
    echo "profile run ID requires explicit candidate mode" >&2
    exit 2
  fi
  BUILD_DIR="${INPUTIA_BUILD_DIR-$ROOT_DIR/build}"
  APP_DIR="$BUILD_DIR/InputiaInputMethod.app"
  SETTINGS_APP_DIR="$BUILD_DIR/Inputia 设置.app"
fi
# 删除旧构建前固定归属；不接受安装目录、路径穿越或符号链接别名。
if [[ "$BUILD_DIR" != "$ROOT_DIR/build" && "$BUILD_DIR" != "$ROOT_DIR/build/"* && "$BUILD_DIR" != "$ROOT_DIR/candidate-builds/"* ]]; then
  echo "build output must remain below this checkout's build roots" >&2
  exit 2
fi
if [[ "$BUILD_DIR" != "${BUILD_DIR:A}" ]]; then
  echo "build output must be canonical and cannot use symlink aliases" >&2
  exit 2
fi
build_cursor="$BUILD_DIR"
while [[ "$build_cursor" != "/" ]]; do
  if [[ -L "$build_cursor" ]]; then
    echo "build output contains a symbolic link" >&2
    exit 2
  fi
  build_cursor="${build_cursor:h}"
done
if [[ -L "$APP_DIR" || -L "$SETTINGS_APP_DIR" ]]; then
  echo "build app output cannot be a symbolic link" >&2
  exit 2
fi
if [[ "${INPUTIA_BUILD_PATH_CHECK_ONLY:-0}" == "1" ]]; then
  echo "buildPathSafe=true candidate=$IS_CANDIDATE output=$BUILD_DIR"
  exit 0
fi
if [[ "$IS_CANDIDATE" == "1" ]]; then
  mkdir -p "$BUILD_DIR"
  INPUTIA_BUILD_ARTIFACT_LOCK_DIR="$BUILD_DIR/.build-artifacts.lock"
  INPUTIA_BUILD_ARTIFACT_LOCK_HELD=0
  INPUTIA_BUILD_ARTIFACT_LOCK_ACQUIRED=0
fi
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
elif [[ "$SIGN_IDENTITY" == "-" && "$IS_CANDIDATE" != "1" ]]; then
  SIGN_OPTIONS=""
else
  SIGN_OPTIONS="--options runtime"
fi
ENTITLEMENTS="${INPUTIA_CODESIGN_ENTITLEMENTS:-$ROOT_DIR/InputiaInputMethod.entitlements}"
if [[ "$IS_CANDIDATE" == "1" ]]; then
  if [[ -n "${INPUTIA_CODESIGN_ENTITLEMENTS:-}" && "$INPUTIA_CODESIGN_ENTITLEMENTS" != "$ROOT_DIR/InputiaUnifiedCandidate.entitlements" ]]; then
    echo "candidate requires its strict runtime entitlements" >&2
    exit 2
  fi
  if [[ "$SIGN_OPTIONS" != "--options runtime" ]]; then
    echo "candidate requires hardened runtime signing" >&2
    exit 2
  fi
  ENTITLEMENTS="$ROOT_DIR/InputiaUnifiedCandidate.entitlements"
fi
if [[ "${INPUTIA_CODESIGN_AS_ROOT:-0}" == "1" ]]; then
  CODESIGN_KEYCHAIN="${INPUTIA_CODESIGN_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"
fi
BUILD_USER="$(/usr/bin/id -un)"
BUILD_GROUP="$(/usr/bin/id -gn)"
MIN_MACOS_VERSION="13.0"
TARGET_TRIPLE="$(uname -m)-apple-macos$MIN_MACOS_VERSION"
CAPI_MANIFEST="$ROOT_DIR/../../crates/inputia-capi/Cargo.toml"
CAPI_FEATURE_ARGS=()
HOST_SWIFT_DEFINES=()
SETTINGS_SWIFT_DEFINES=()
PAIR_SWIFT_SOURCES=()
if [[ -n "${INPUTIA_PAIR_BUILD_METADATA:-}" && "$IS_CANDIDATE" != "1" ]]; then
  echo "pair build metadata requires explicit candidate mode" >&2
  exit 2
fi
if [[ "$IS_CANDIDATE" == "1" ]]; then
  HOST_SWIFT_DEFINES=(-D INPUTIA_UNIFIED_CANDIDATE)
  SETTINGS_SWIFT_DEFINES=(-D INPUTIA_UNIFIED_CANDIDATE -D INPUTIA_SETTINGS_LAUNCHER)
  static_repository_root="$(cd "$ROOT_DIR/../.." && pwd -P)"
  export INPUTIA_STATIC_RIME_DIR="${INPUTIA_STATIC_RIME_DIR:-$static_repository_root/native/static-rime/artifacts/output/$(uname -m)}"
  if [[ ! -f "$INPUTIA_STATIC_RIME_DIR/lib/libinputia_rime_static.a" ]]; then
    echo "build static Rime first with native/static-rime/build.sh; no dynamic fallback is allowed" >&2
    exit 2
  fi
  CAPI_FEATURE_ARGS=(--features bundled-static-rime)
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/../../crates/inputia-capi/target}"
if [[ "$IS_CANDIDATE" == "1" ]]; then
  export CARGO_TARGET_DIR="$BUILD_DIR/cargo-target"
fi
RUST_TOOLCHAIN="${INPUTIA_RUST_TOOLCHAIN:-1.96.0}"
if [[ -n "${MACOSX_DEPLOYMENT_TARGET:-}" && "$MACOSX_DEPLOYMENT_TARGET" != "$MIN_MACOS_VERSION" && "$MACOSX_DEPLOYMENT_TARGET" != "13" && "$MACOSX_DEPLOYMENT_TARGET" != "13.0.0" ]]; then
  echo "MACOSX_DEPLOYMENT_TARGET must match the supported macOS $MIN_MACOS_VERSION minimum" >&2
  exit 2
fi
# Swift、Rust 和 cc 编译的 bundled SQLite 必须共享部署下限。
# 删除该变量会让 cc 使用 SDK 默认值，并产生高于最终 Host 下限的静态对象。
export MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS_VERSION"

run_cargo() {
  if [[ -n "${CARGO:-}" ]]; then
    "$CARGO" "$@"
  elif /usr/bin/command -v rustup >/dev/null 2>&1; then
    rustup run "$RUST_TOOLCHAIN" cargo "$@"
  else
    cargo "$@"
  fi
}

if [[ "${INPUTIA_BUILD_DEPLOYMENT_SELF_CHECK:-0}" == "1" ]]; then
  observed_target="$(CARGO=/usr/bin/printenv run_cargo MACOSX_DEPLOYMENT_TARGET)"
  if [[ "$observed_target" != "$MIN_MACOS_VERSION" ]]; then
    echo "buildDeploymentSelfCheck=false observed=$observed_target" >&2
    exit 2
  fi
  echo "buildDeploymentSelfCheck=true cargoTarget=$observed_target swiftTarget=$TARGET_TRIPLE"
  exit 0
fi

check_macos_deployment() {
  local artifact="$1"
  local exact="${2:-0}"
  /usr/bin/otool -l "$artifact" | /usr/bin/python3 -c '
import sys
expected, exact, artifact = sys.argv[1:]
def version(value):
    parts = tuple(int(part) for part in value.split("."))
    return parts + (0,) * (3 - len(parts))
versions = []
command = None
for line in sys.stdin:
    fields = line.split()
    if len(fields) == 2 and fields[0] == "cmd":
        command = fields[1]
    if len(fields) == 2 and ((command == "LC_BUILD_VERSION" and fields[0] == "minos") or (command == "LC_VERSION_MIN_MACOSX" and fields[0] == "version")):
        versions.append(fields[1])
if not versions or any(version(value) > version(expected) or (exact == "1" and version(value) != version(expected)) for value in versions):
    sys.exit(f"macOSDeploymentCheck=false expected={expected} observed={sorted(set(versions))} path={artifact}")
observed = ",".join(sorted(set(versions)))
print(f"macOSDeploymentCheck=true expected={expected} observed={observed} entries={len(versions)} path={artifact}")
' "$MIN_MACOS_VERSION" "$exact" "$artifact"
}

localize_candidate_resources() {
  local resources="$1" candidate_id="$2" localization strings_file candidate_name key
  for localization in en zh-Hans zh-Hant; do
    strings_file="$resources/$localization.lproj/InfoPlist.strings"
    [[ -f "$strings_file" ]] || return 1
    case "$localization" in
      en) candidate_name="Inputia (Test)" ;;
      zh-Hans) candidate_name="Inputia（测试版）" ;;
      zh-Hant) candidate_name="Inputia（測試版）" ;;
    esac
    [[ "$IS_RELEASE" == "1" ]] && candidate_name="Inputia"
    for key in CFBundleDisplayName CFBundleName "$candidate_id" "$candidate_id.Hans"; do
      /usr/libexec/PlistBuddy -c "Delete :$key" "$strings_file" >/dev/null 2>&1 || true
      /usr/libexec/PlistBuddy -c "Add :$key string $candidate_name" "$strings_file"
    done
    /usr/bin/plutil -lint "$strings_file" >/dev/null
  done
}

if [[ "${INPUTIA_BUILD_CANDIDATE_LOCALIZATION_SELF_CHECK:-0}" == "1" ]]; then
  candidate_host_id="com.inputia.inputmethod.Inputia.UnifiedCandidate"
  tmp_root="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/inputia-candidate-localization.XXXXXX")"
  trap '/bin/rm -rf "$tmp_root"' EXIT
  self_check_plist="$tmp_root/Info.plist"
  self_check_resources="$tmp_root/Resources"
  /bin/cp "$ROOT_DIR/Info.plist" "$self_check_plist"
  /bin/mkdir -p "$self_check_resources/en.lproj" "$self_check_resources/zh-Hans.lproj" "$self_check_resources/zh-Hant.lproj"
  for localization in en zh-Hans zh-Hant; do
    /bin/cp "$ROOT_DIR/Resources/$localization.lproj/InfoPlist.strings" "$self_check_resources/$localization.lproj/InfoPlist.strings"
  done
  /usr/libexec/PlistBuddy -c "Copy :ComponentInputModeDict:tsInputModeListKey:com.inputia.inputmethod.Inputia.Hans :ComponentInputModeDict:tsInputModeListKey:$candidate_host_id.Hans" "$self_check_plist"
  /usr/libexec/PlistBuddy -c "Delete :ComponentInputModeDict:tsInputModeListKey:com.inputia.inputmethod.Inputia.Hans" "$self_check_plist"
  /usr/libexec/PlistBuddy -c "Set :ComponentInputModeDict:tsInputModeListKey:$candidate_host_id.Hans:TISInputSourceID $candidate_host_id.Hans" "$self_check_plist"
  /usr/libexec/PlistBuddy -c "Set :ComponentInputModeDict:tsVisibleInputModeOrderedArrayKey:0 $candidate_host_id.Hans" "$self_check_plist"
  observed_modes="$(/usr/libexec/PlistBuddy -c "Print :ComponentInputModeDict:tsVisibleInputModeOrderedArrayKey:0" "$self_check_plist")"
  expected_modes="$candidate_host_id.Hans"
  if [[ "$observed_modes" != "$expected_modes" ]]; then
    echo "candidateLocalizationSelfCheck=false reason=mode-mismatch expectedModes=$expected_modes observedModes=$observed_modes" >&2
    exit 1
  fi
  if /usr/libexec/PlistBuddy -c "Print :ComponentInputModeDict:tsInputModeListKey:$candidate_host_id.Hant" "$self_check_plist" >/dev/null 2>&1; then
    echo "candidateLocalizationSelfCheck=false reason=unexpected-hant-mode" >&2
    exit 1
  fi
  localize_candidate_resources "$self_check_resources" "$candidate_host_id"
  for localization in en zh-Hans zh-Hant; do
    strings_file="$self_check_resources/$localization.lproj/InfoPlist.strings"
    case "$localization" in
      en) expected_name="Inputia (Test)" ;;
      zh-Hans) expected_name="Inputia（测试版）" ;;
      zh-Hant) expected_name="Inputia（測試版）" ;;
    esac
    [[ "$IS_RELEASE" == "1" ]] && expected_name="Inputia"
    for suffix in "" ".Hans"; do
      observed_name="$(/usr/libexec/PlistBuddy -c "Print :$candidate_host_id$suffix" "$strings_file")"
      if [[ "$observed_name" != "$expected_name" ]]; then
        echo "candidateLocalizationSelfCheck=false localization=$localization key=$candidate_host_id$suffix expected=$expected_name observed=$observed_name" >&2
        exit 1
      fi
    done
    for key in CFBundleDisplayName CFBundleName; do
      observed_name="$(/usr/libexec/PlistBuddy -c "Print :$key" "$strings_file")"
      if [[ "$observed_name" != "$expected_name" ]]; then
        echo "candidateLocalizationSelfCheck=false localization=$localization key=$key expected=$expected_name observed=$observed_name" >&2
        exit 1
      fi
    done
    if /usr/libexec/PlistBuddy -c "Print :$candidate_host_id.Hant" "$strings_file" >/dev/null 2>&1; then
      echo "candidateLocalizationSelfCheck=false localization=$localization reason=unexpected-hant-localization" >&2
      exit 1
    fi
  done
  echo "candidateLocalizationSelfCheck=true modes=$observed_modes"
  exit 0
fi

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

if [[ -n "${INPUTIA_PAIR_BUILD_METADATA:-}" ]]; then
  pair_source="$BUILD_DIR/InputiaEmbeddedPairTrust.swift"
  /usr/bin/python3 "$ROOT_DIR/../../native/unified-pair-auth/build_trust.py" \
    --metadata "$INPUTIA_PAIR_BUILD_METADATA" --run-id "$RUN_ID" --emit swift > "$pair_source"
  PAIR_SWIFT_SOURCES=("$pair_source" "$ROOT_DIR/../../native/unified-pair-auth/UnifiedPairAuth.swift")
  HOST_SWIFT_DEFINES+=(-D INPUTIA_PAIRED_BUILD)
fi

rm -rf "$APP_DIR" "$SETTINGS_APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR" "$SETTINGS_MACOS_DIR" "$SETTINGS_RESOURCES_DIR"

if [[ ! -f "$CAPI_MANIFEST" ]]; then
  echo "missing inputia-capi manifest: $CAPI_MANIFEST" >&2
  exit 1
fi

/bin/zsh "$ROOT_DIR/Tools/verify-imk-event-route.sh" \
  "$ROOT_DIR/Sources/InputiaInputMethod/main.swift"

CAPI_LIB="$(run_cargo build --release --manifest-path "$CAPI_MANIFEST" "${CAPI_FEATURE_ARGS[@]}" --message-format=json-render-diagnostics |
  /usr/bin/python3 -c '
import json, sys
libraries = []
for line in sys.stdin:
    event = json.loads(line)
    target = event.get("target", {})
    if event.get("reason") == "compiler-artifact" and target.get("name") == "inputia_capi" and "staticlib" in target.get("crate_types", []):
        libraries.extend(path for path in event.get("filenames", []) if path.endswith(".a"))
if len(libraries) != 1:
    sys.exit("expected exactly one inputia_capi staticlib from this cargo build")
print(libraries[0])
')"
if [[ ! -f "$CAPI_LIB" ]]; then
  echo "missing inputia-capi staticlib: $CAPI_LIB" >&2
  exit 1
fi
# 不仅检查最终可执行文件：链接器仍可能接受带更高 minOS 的 archive 成员。
check_macos_deployment "$CAPI_LIB"
CAPI_LINK_ARGS=("$CAPI_LIB")
if [[ "$IS_CANDIDATE" == "1" ]]; then
  CAPI_LINK_ARGS=(-Xlinker -force_load -Xlinker "$CAPI_LIB" -lc++)
fi

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/UnifiedInputProfileSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  -parse-as-library \
  -target "$TARGET_TRIPLE" \
  -o "$BUILD_DIR/unified-input-profile-self-check"

/usr/bin/swiftc -parse-as-library \
  "$ROOT_DIR/Tools/InputiaTypedCaptureSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaTypedCapture.swift" \
  -target "$TARGET_TRIPLE" -o "$BUILD_DIR/inputia-typed-capture-self-check"
"$BUILD_DIR/inputia-typed-capture-self-check"

/usr/bin/swiftc -parse-as-library \
  "$ROOT_DIR/Tools/InputiaPersonalizationSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPersonalization.swift" \
  -target "$TARGET_TRIPLE" -o "$BUILD_DIR/inputia-personalization-self-check"
"$BUILD_DIR/inputia-personalization-self-check"

/usr/bin/swiftc -parse-as-library -D INPUTIA_PAIRED_BUILD \
  "$ROOT_DIR/Tools/InputiaPersonalizationFlowSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPersonalization.swift" \
  -target "$TARGET_TRIPLE" -o "$BUILD_DIR/inputia-personalization-flow-self-check"
"$BUILD_DIR/inputia-personalization-flow-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Sources/InputiaInputMethod/main.swift" \
  "${HOST_SWIFT_DEFINES[@]}" \
  "${PAIR_SWIFT_SOURCES[@]}" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRuntimeDiagnostics.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaTypedCapture.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPersonalization.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaExplicitHotwords.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHostTextPolicy.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHandyMemorySync.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaInputTextRouter.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaExpandedCandidateGridNavigation.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceInputLauncher.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPermissionLifecycle.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceTargetSnapshot.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaFramedConnection.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceServiceConnection.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaCandidatePanel.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaSettingsWindow.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "${CAPI_LINK_ARGS[@]}" \
  -parse-as-library \
  -target "$TARGET_TRIPLE" \
  -module-name InputiaInputMethod \
  -framework Cocoa \
  -framework InputMethodKit \
  -framework ApplicationServices \
  -framework Security \
  -o "$MACOS_DIR/InputiaInputMethod"

/usr/bin/python3 "$ROOT_DIR/Tools/check_ime_permission_boundary.py" --binary "$MACOS_DIR/InputiaInputMethod"

cp "$ROOT_DIR/Info.plist" "$CONTENTS_DIR/Info.plist"
if [[ "$IS_CANDIDATE" == "1" ]]; then
  candidate_host_id="com.inputia.inputmethod.Inputia.UnifiedCandidate"
  host_plist="$CONTENTS_DIR/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $candidate_host_id" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleName Inputia候选" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName Inputia候选" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion 73" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString 0.1.0" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :InputMethodConnectionName ${candidate_host_id}_Connection" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :TISInputSourceID $candidate_host_id" "$host_plist"
  /usr/libexec/PlistBuddy -c "Copy :ComponentInputModeDict:tsInputModeListKey:com.inputia.inputmethod.Inputia.Hans :ComponentInputModeDict:tsInputModeListKey:$candidate_host_id.Hans" "$host_plist"
  /usr/libexec/PlistBuddy -c "Delete :ComponentInputModeDict:tsInputModeListKey:com.inputia.inputmethod.Inputia.Hans" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :ComponentInputModeDict:tsInputModeListKey:$candidate_host_id.Hans:TISInputSourceID $candidate_host_id.Hans" "$host_plist"
  /usr/libexec/PlistBuddy -c "Set :ComponentInputModeDict:tsVisibleInputModeOrderedArrayKey:0 $candidate_host_id.Hans" "$host_plist"
  /usr/libexec/PlistBuddy -c "Add :InputiaDevelopmentCandidate bool true" "$host_plist"
  /usr/libexec/PlistBuddy -c "Add :InputiaProfileRunID string $RUN_ID" "$host_plist"
  if [[ "$IS_RELEASE" == "1" ]]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleName Inputia" "$host_plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName Inputia" "$host_plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion 82" "$host_plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString 1.0.8" "$host_plist"
    /usr/libexec/PlistBuddy -c "Add :InputiaReleaseChannel string stable" "$host_plist"
  fi
fi
cp -R "$ROOT_DIR/Resources/." "$RESOURCES_DIR/"
if [[ "$IS_CANDIDATE" == "1" ]]; then
  localize_candidate_resources "$RESOURCES_DIR" "$candidate_host_id"
fi
/usr/bin/python3 "$ROOT_DIR/Tools/generate_inputia_icons.py" --resources-dir "$RESOURCES_DIR"
/bin/rm -rf "$RESOURCES_DIR/RimeData"
if [[ "$IS_CANDIDATE" == "1" ]]; then
  /usr/bin/python3 "$ROOT_DIR/candidate-rime-data/prepare.py" --run-id "$RUN_ID" --output "$RIME_DATA_BUILD_DIR"
else
  INPUTIA_RIME_DATA_BUILD_DIR="$RIME_DATA_BUILD_DIR" "$ROOT_DIR/prepare-rime-data.sh" >/dev/null
fi
cp -R "$RIME_DATA_BUILD_DIR" "$RESOURCES_DIR/RimeData"
/usr/bin/plutil -lint "$CONTENTS_DIR/Info.plist"

/usr/bin/swiftc \
  "$ROOT_DIR/SettingsLauncher/main.swift" \
  "${SETTINGS_SWIFT_DEFINES[@]}" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  -parse-as-library \
  -target "$TARGET_TRIPLE" \
  -module-name InputiaSettingsLauncher \
  -framework AppKit \
  -o "$SETTINGS_MACOS_DIR/InputiaSettingsLauncher"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaShortcutSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaInputTextRouter.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaExpandedCandidateGridNavigation.swift" \
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
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaInputTextRouter.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaShortcutClassifier.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "${CAPI_LINK_ARGS[@]}" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-input-text-router-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaHandyMemorySyncSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHandyMemorySync.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "${CAPI_LINK_ARGS[@]}" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-handy-memory-sync-self-check"

/usr/bin/swiftc -parse-as-library \
  "$ROOT_DIR/Tools/InputiaCandidateIdentitySelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHandyMemorySync.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "${CAPI_LINK_ARGS[@]}" \
  -target "$TARGET_TRIPLE" -framework AppKit \
  -o "$BUILD_DIR/inputia-candidate-identity-self-check"
"$BUILD_DIR/inputia-candidate-identity-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaVoiceInputLauncherSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceInputLauncher.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPermissionLifecycle.swift" \
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
  "${HOST_SWIFT_DEFINES[@]}" \
  "$ROOT_DIR/Tools/InputiaVoiceTargetSnapshotSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceTargetSnapshot.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPermissionLifecycle.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceServiceConnection.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaFramedConnection.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "${PAIR_SWIFT_SOURCES[@]}" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -framework InputMethodKit \
  -framework ApplicationServices \
  -framework Security \
  -o "$BUILD_DIR/inputia-voice-target-snapshot-self-check"

# 合成权限测试：不查询或修改系统权限，不访问用户文本。
/usr/bin/swiftc "$ROOT_DIR/Tools/InputiaPermissionLifecycleSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPermissionLifecycle.swift" \
  -target "$TARGET_TRIPLE" -o "$BUILD_DIR/inputia-permission-lifecycle-self-check"
"$BUILD_DIR/inputia-permission-lifecycle-self-check"

# 有界后台查询自检：模拟挂起任务，不访问真实窗口。
/usr/bin/swiftc "$ROOT_DIR/Tools/InputiaWindowTitleQuerySelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceTargetSnapshot.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaPermissionLifecycle.swift" \
  -target "$TARGET_TRIPLE" -o "$BUILD_DIR/inputia-window-title-query-self-check"
"$BUILD_DIR/inputia-window-title-query-self-check"

# 配对协议/短词缓存使用合成socket自检；不启动GUI、麦克风或读取用户词库。
for check in InputiaVoiceServiceSelfCheck InputiaSharedTermsSelfCheck InputiaExplicitHotwordsSelfCheck; do
  /usr/bin/swiftc \
    -parse-as-library -D INPUTIA_PAIRED_BUILD -D INPUTIA_CONNECTION_SELF_CHECK \
    -target "$TARGET_TRIPLE" \
    "$ROOT_DIR/Tools/$check.swift" \
    "$ROOT_DIR/Sources/InputiaInputMethod/InputiaExplicitHotwords.swift" \
    "$ROOT_DIR/Sources/InputiaInputMethod/InputiaVoiceServiceConnection.swift" \
    "$ROOT_DIR/Sources/InputiaInputMethod/InputiaFramedConnection.swift" \
    "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
    "$ROOT_DIR/../../native/unified-pair-auth/UnifiedPairAuth.swift" \
    -framework Security -o "$BUILD_DIR/$check"
  "$BUILD_DIR/$check"
done

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaCandidatePanelLayoutSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaCandidatePanel.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaExpandedCandidateGridNavigation.swift" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-candidate-panel-layout-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaSettingsWindowSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaSettingsWindow.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaHandyMemorySync.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "${CAPI_LINK_ARGS[@]}" \
  -target "$TARGET_TRIPLE" \
  -framework AppKit \
  -o "$BUILD_DIR/inputia-settings-window-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaBridgePrivacySelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "${CAPI_LINK_ARGS[@]}" \
  -target "$TARGET_TRIPLE" \
  -framework Foundation \
  -o "$BUILD_DIR/inputia-bridge-privacy-self-check"

/usr/bin/swiftc \
  "$ROOT_DIR/Tools/InputiaBridgeCandidateCountSelfCheck.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaProfile.swift" \
  "$ROOT_DIR/Sources/InputiaInputMethod/InputiaRustBridge.swift" \
  "${CAPI_LINK_ARGS[@]}" \
  -target "$TARGET_TRIPLE" \
  -framework Foundation \
  -o "$BUILD_DIR/inputia-bridge-candidate-count-self-check"

cp "$ROOT_DIR/SettingsLauncher/Info.plist" "$SETTINGS_CONTENTS_DIR/Info.plist"
if [[ "$IS_CANDIDATE" == "1" ]]; then
  settings_plist="$SETTINGS_CONTENTS_DIR/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier com.inputia.settings.UnifiedCandidate" "$settings_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleName Inputia候选设置" "$settings_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName Inputia候选设置" "$settings_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion 73" "$settings_plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString 0.1.0" "$settings_plist"
  /usr/libexec/PlistBuddy -c "Add :InputiaDevelopmentCandidate bool true" "$settings_plist"
  /usr/libexec/PlistBuddy -c "Add :InputiaProfileRunID string $RUN_ID" "$settings_plist"
  if [[ "$IS_RELEASE" == "1" ]]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleName Inputia设置" "$settings_plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName Inputia设置" "$settings_plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion 82" "$settings_plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString 1.0.8" "$settings_plist"
    /usr/libexec/PlistBuddy -c "Add :InputiaReleaseChannel string stable" "$settings_plist"
  fi
fi
cp "$RESOURCES_DIR/Inputia.icns" "$SETTINGS_RESOURCES_DIR/Inputia.icns"
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
  expected_host_cdhash="$(/usr/bin/codesign -dv --verbose=4 "$APP_DIR" 2>&1 | /usr/bin/awk -F= '/^CDHash=/{print $2}')"
  /usr/libexec/PlistBuddy -c "Delete :InputiaExpectedHostCDHash" "$SETTINGS_CONTENTS_DIR/Info.plist" >/dev/null 2>&1 || true
  /usr/libexec/PlistBuddy -c "Add :InputiaExpectedHostCDHash string $expected_host_cdhash" "$SETTINGS_CONTENTS_DIR/Info.plist"
  /usr/libexec/PlistBuddy -c "Print :InputiaExpectedHostCDHash" "$SETTINGS_CONTENTS_DIR/Info.plist" >/dev/null
  /usr/bin/plutil -lint "$SETTINGS_CONTENTS_DIR/Info.plist" >/dev/null
else
  /usr/bin/sed 's/^/codesignOutput: /' "$codesign_output" >&2 || true
  /bin/rm -f "$codesign_output"
  echo "warning: codesign failed with identity '$SIGN_IDENTITY'; build artifact still exists at $APP_DIR" >&2
  if [[ "$SIGN_IDENTITY" != "-" || "$IS_CANDIDATE" == "1" ]]; then
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
  if [[ "$SIGN_IDENTITY" != "-" || "$IS_CANDIDATE" == "1" ]]; then
    echo "buildSigned=false reason=codesign-failed target=settings-launcher identity=$SIGN_IDENTITY" >&2
    exit 32
  fi
fi

if [[ "$IS_CANDIDATE" != "1" ]]; then
  "$LSREGISTER" -u "$APP_DIR" >/dev/null 2>&1 || true
  "$LSREGISTER" -u "$SETTINGS_APP_DIR" >/dev/null 2>&1 || true
fi

for candidate_binary in "$MACOS_DIR/InputiaInputMethod" "$SETTINGS_MACOS_DIR/InputiaSettingsLauncher"; do
  check_macos_deployment "$candidate_binary" 1
done
for candidate_plist in "$CONTENTS_DIR/Info.plist" "$SETTINGS_CONTENTS_DIR/Info.plist"; do
  if [[ "$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$candidate_plist")" != "$MIN_MACOS_VERSION" ]]; then
    echo "bundle minimum macOS version does not match its binaries: $candidate_plist" >&2
    exit 2
  fi
done

echo "$APP_DIR"
echo "$SETTINGS_APP_DIR"
