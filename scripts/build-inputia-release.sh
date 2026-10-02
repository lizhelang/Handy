#!/bin/bash
# 构建融合正式版与最终配对清单；不安装、不启动应用、不修改用户数据。
set -euo pipefail
umask 077

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RELEASE_PYTHON="${INPUTIA_RELEASE_PYTHON:-python3}"
if [[ "${1:-}" == "--help" ]]; then
  cat <<'HELP'
用法：scripts/build-inputia-release.sh [--preflight | --preflight-public | --build-local | --build-local-v2]
默认只读预检；--build-local 构建 v1 本机体验版，--build-local-v2 构建绑定 releaseId 的 v2 本机配对制品。
本机构建必须显式提供：
  INPUTIA_PAIR_BUILD_METADATA  已有配对公开构建元数据的绝对路径
  INPUTIA_PAIR_PRIVATE_KEY     配对签名私钥的绝对路径（不复制、不输出内容）
  INPUTIA_CODESIGN_IDENTITY    已有稳定证书身份，不能为临时签名 -
可选：
  INPUTIA_PROFILE_RUN_ID       v1 默认 trial-20260905；v2 禁止设置
  CARGO_TARGET_DIR             控制中心编译输出目录
  INPUTIA_RELEASE_PYTHON       Python >= 3.11（默认 PATH python3）
公共签名、公证、独立安装器和制品验收尚未完成时，公共预检始终阻断。
本机构建输出不表示公开发布资格；不安装。
HELP
  exit 0
fi
if [[ $# -gt 1 ]]; then
  echo "未知参数；使用 --help 查看用法" >&2
  exit 2
fi
PAIR_IS_RELEASE_V2=0
case "${1:---preflight}" in
  --preflight) exec "$RELEASE_PYTHON" "$REPO_ROOT/scripts/inputia_release.py" preflight --mode local ;;
  --preflight-public) exec "$RELEASE_PYTHON" "$REPO_ROOT/scripts/inputia_release.py" preflight --mode public ;;
  --build-local) ;;
  --build-local-v2) PAIR_IS_RELEASE_V2=1 ;;
  *) echo "未知参数；使用 --help 查看用法" >&2; exit 2 ;;
esac
# 元数据与工具链检查发生在读取签名输入及创建输出之前。
"$RELEASE_PYTHON" "$REPO_ROOT/scripts/inputia_release.py" preflight --mode local
: "${INPUTIA_PAIR_PRIVATE_KEY:?必须提供配对签名私钥路径}"
: "${INPUTIA_CODESIGN_IDENTITY:?必须提供已有稳定签名证书身份}"
if [[ "$INPUTIA_CODESIGN_IDENTITY" == "-" ]]; then
  echo "正式版拒绝临时签名身份" >&2
  exit 2
fi
if [[ "$PAIR_IS_RELEASE_V2" == "1" ]]; then
  if [[ -n "${INPUTIA_PROFILE_RUN_ID:-}" ]]; then
    echo "v2 构建不能携带 v1 profile ID" >&2
    exit 2
  fi
  unset INPUTIA_PROFILE_RUN_ID || true
else
  export INPUTIA_PROFILE_RUN_ID="${INPUTIA_PROFILE_RUN_ID:-trial-20260905}"
  if [[ ! "$INPUTIA_PROFILE_RUN_ID" =~ ^[A-Za-z0-9_-]{1,64}$ ]]; then
    echo "无效 profile ID" >&2
    exit 2
  fi
fi
if [[ "$PAIR_IS_RELEASE_V2" == "0" ]]; then
  : "${INPUTIA_PAIR_BUILD_METADATA:?必须提供配对公开构建元数据路径}"
  input_paths=("$INPUTIA_PAIR_BUILD_METADATA" "$INPUTIA_PAIR_PRIVATE_KEY")
else
  input_paths=("$INPUTIA_PAIR_PRIVATE_KEY")
fi
for input_path in "${input_paths[@]}"; do
  if [[ "$input_path" != /* || ! -f "$input_path" || -L "$input_path" ]]; then
    echo "配对输入必须是绝对路径的普通文件，不能使用符号链接" >&2
    exit 2
  fi
done

cd "$REPO_ROOT"
BUILD_BASE="$HOME/Library/Application Support/HandyUnifiedBuilds"
mkdir -p "$BUILD_BASE"
RELEASE_DIR="$(/usr/bin/mktemp -d "$BUILD_BASE/release-XXXXXXXX")"
chmod 700 "$RELEASE_DIR"
METADATA_DIR="$RELEASE_DIR/metadata"
"$RELEASE_PYTHON" scripts/inputia_release.py prepare --mode local --output-dir "$METADATA_DIR"
export INPUTIA_RELEASE_CONTEXT="$METADATA_DIR/build-context.json"
export INPUTIA_RELEASE_PYTHON="$RELEASE_PYTHON"
if [[ "$PAIR_IS_RELEASE_V2" == "1" ]]; then
  # v2 信任材料绑定本次已准备的 release context；只从私钥工具读取公钥，不把私钥复制到构建目录。
  /usr/bin/swiftc -parse-as-library \
    native/unified-pair-auth/UnifiedPairAuth.swift \
    native/unified-pair-auth/PairAuthTool.swift \
    -o "$RELEASE_DIR/PairAuthTool"
  PUBLIC_KEY_HEX="$("$RELEASE_DIR/PairAuthTool" public-key "$INPUTIA_PAIR_PRIVATE_KEY")"
  if [[ ! "$PUBLIC_KEY_HEX" =~ ^04[0-9a-fA-F]{128}$ ]]; then
    echo "配对工具返回的公钥格式无效" >&2
    exit 2
  fi
  /usr/bin/printf '%s' "$PUBLIC_KEY_HEX" | /usr/bin/xxd -r -p > "$RELEASE_DIR/public-key.x963"
  chmod 600 "$RELEASE_DIR/public-key.x963"
  /usr/bin/python3 native/unified-pair-auth/build_trust.py \
    --metadata "$RELEASE_DIR/public-build.json" \
    --public-key "$RELEASE_DIR/public-key.x963" \
    --release-context "$INPUTIA_RELEASE_CONTEXT"
  INPUTIA_PAIR_BUILD_METADATA="$RELEASE_DIR/public-build.json"
  export INPUTIA_PAIR_BUILD_METADATA
  : "${INPUTIA_UPDATER_APP:?v2 构建必须提供已构建的 Inputia Updater.app 路径}"
  : "${INPUTIA_BOOTSTRAP_APP:?v2 构建必须提供已构建的 Inputia Installer.app 路径}"
  for required_app in "$INPUTIA_UPDATER_APP" "$INPUTIA_BOOTSTRAP_APP"; do
    if [[ "$required_app" != /* || ! -d "$required_app" || -L "$required_app" ]]; then
      echo "v2 独立更新组件必须是绝对路径下的普通 .app 目录: $required_app" >&2
      exit 2
    fi
  done
else
  # 校验公开元数据合同；只生成公钥常量，不读取私钥。
  /usr/bin/python3 native/unified-pair-auth/build_trust.py \
    --metadata "$INPUTIA_PAIR_BUILD_METADATA" --run-id "$INPUTIA_PROFILE_RUN_ID" --emit rust >/dev/null
fi
SIGN_OVERLAY="$RELEASE_DIR/signing-overlay.json"
"$RELEASE_PYTHON" - "$SIGN_OVERLAY" "$INPUTIA_CODESIGN_IDENTITY" "$RELEASE_DIR" "${INPUTIA_PROFILE_RUN_ID:-}" "$PAIR_IS_RELEASE_V2" <<'PY'
import json
import plistlib
import sys
from pathlib import Path
with Path("src-tauri/InputiaReleaseInfo.plist").open("rb") as stream:
    info = plistlib.load(stream)
if sys.argv[5] == "1":
    info["HandyProfileRunID"] = sys.argv[4]
    # 仅保留当前本机 v1 信任桥；渠道不进入不可变程序。
    info["HandyDevelopmentCandidate"] = True
else:
    info.pop("HandyProfileRunID", None)
    info.pop("HandyDevelopmentCandidate", None)
release_plist = Path(sys.argv[3]) / "InputiaReleaseInfo.plist"
with release_plist.open("wb") as stream:
    plistlib.dump(info, stream)
Path(sys.argv[1]).write_text(json.dumps({"bundle": {"macOS": {"signingIdentity": sys.argv[2], "infoPlist": str(release_plist)}}}) + "\n")
PY
"$RELEASE_PYTHON" scripts/inputia_release.py apply-plist --role control \
  --plist "$RELEASE_DIR/InputiaReleaseInfo.plist" --context "$INPUTIA_RELEASE_CONTEXT"
export HANDY_UNIFIED_PAIR_BUILD="$INPUTIA_PAIR_BUILD_METADATA"
# 每次发布构建使用独立的 Cargo target，避免并行/重入构建互相覆盖
# proc-macro/dylib 产物。调用方仍可显式指定目录，但默认不得共享源码树
# 的 src-tauri/target；该目录可能同时被开发构建或另一份发布任务使用。
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$RELEASE_DIR/cargo-target}"
# 固定绝对输出路径，避免 Rust/Tauri 对相对目录有不同解释。
CARGO_TARGET_DIR="$(/usr/bin/python3 - "$CARGO_TARGET_DIR" <<'PY'
import os
import sys
print(os.path.abspath(sys.argv[1]))
PY
)"
export CARGO_TARGET_DIR
export CMAKE_POLICY_VERSION_MINIMUM="${CMAKE_POLICY_VERSION_MINIMUM:-3.5}"
# macOS 27 SDK 配合 release profile 的 strip=true 会生成无法被 rustc
# 加载的 proc-macro dylib（mis-aligned LINKEDIT string pool）。构建期保留
# 中间产物符号；最终应用仍由各自的打包与签名步骤处理。
export CARGO_PROFILE_RELEASE_STRIP=none
# 保留标准 beforeBuildCommand，包括语音辅助程序准备。
bun run tauri build --bundles app \
  --config src-tauri/tauri.unified-candidate.conf.json \
  --config src-tauri/tauri.inputia-release.conf.json \
  --config "$SIGN_OVERLAY"
CONTROL_APP="$CARGO_TARGET_DIR/release/bundle/macos/Inputia.app"
[[ -d "$CONTROL_APP" ]] || { echo "控制中心构建产物缺失" >&2; exit 1; }
if [[ "$PAIR_IS_RELEASE_V2" == "1" ]]; then
  # Tauri 会把候选配置中的标记合并回最终 Info.plist；v2 不得携带
  # 可复用的 trial 身份，因此在最终签名后再次删除并重新封装签名。
  control_plist="$CONTROL_APP/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Delete :HandyProfileRunID" "$control_plist" 2>/dev/null || true
  /usr/libexec/PlistBuddy -c "Delete :HandyDevelopmentCandidate" "$control_plist" 2>/dev/null || true
  codesign --force --deep --sign "$INPUTIA_CODESIGN_IDENTITY" "$CONTROL_APP" >/dev/null
fi

INPUTIA_UNIFIED_CANDIDATE=1 INPUTIA_RELEASE=1 \
  zsh macos/InputiaInputMethod/build.sh
if [[ "$PAIR_IS_RELEASE_V2" == "1" ]]; then
  V2_RUN_ID="release-$(/usr/bin/shasum -a 256 "$INPUTIA_PAIR_BUILD_METADATA" | /usr/bin/awk '{print substr($1,1,24)}')"
  IME_BUILD="$REPO_ROOT/macos/InputiaInputMethod/candidate-builds/$V2_RUN_ID"
else
  IME_BUILD="$REPO_ROOT/macos/InputiaInputMethod/candidate-builds/$INPUTIA_PROFILE_RUN_ID"
fi
# 固定最终包副本，避免后续构建改变清单对应的文件。
/usr/bin/ditto "$CONTROL_APP" "$RELEASE_DIR/Inputia.app"
/usr/bin/ditto "$IME_BUILD/InputiaUnifiedCandidate.app" "$RELEASE_DIR/InputiaUnifiedCandidate.app"
/usr/bin/ditto "$IME_BUILD/Inputia 候选设置.app" "$RELEASE_DIR/Inputia 设置.app"
if [[ "$PAIR_IS_RELEASE_V2" == "1" ]]; then
  /usr/bin/ditto "$INPUTIA_UPDATER_APP" "$RELEASE_DIR/Inputia Updater.app"
  /usr/bin/ditto "$INPUTIA_BOOTSTRAP_APP" "$RELEASE_DIR/Inputia Installer.app"
fi
"$RELEASE_PYTHON" scripts/inputia_release.py verify-bundles \
  --directory "$RELEASE_DIR" --context "$INPUTIA_RELEASE_CONTEXT" \
  --scope "$([[ "$PAIR_IS_RELEASE_V2" == "1" ]] && echo release || echo local-legacy)"
if [[ "$PAIR_IS_RELEASE_V2" == "1" ]]; then
  PAIR_BINDING_ARGS=(--release-context "$INPUTIA_RELEASE_CONTEXT")
else
  PAIR_BINDING_ARGS=(--run-id "$INPUTIA_PROFILE_RUN_ID")
fi
/usr/bin/python3 native/unified-pair-auth/build_trust.py \
  --metadata "$INPUTIA_PAIR_BUILD_METADATA" "${PAIR_BINDING_ARGS[@]}" \
  --sign-pair --handy "$RELEASE_DIR/Inputia.app" \
  --inputia "$RELEASE_DIR/InputiaUnifiedCandidate.app" \
  --build-tool "$RELEASE_DIR/PairAuthTool" \
  --private-key "$INPUTIA_PAIR_PRIVATE_KEY" \
  --manifest "$RELEASE_DIR/pair-manifest.json"
if [[ "$PAIR_IS_RELEASE_V2" == "1" ]]; then
  VERIFY_DIR="$(/usr/bin/mktemp -d "$RELEASE_DIR/.pair-verify-XXXXXX")"
  /usr/bin/swiftc -parse-as-library \
    native/unified-pair-auth/UnifiedPairAuth.swift \
    native/unified-pair-auth/ReleasePairAuthVerify.swift \
    -o "$VERIFY_DIR/verify"
  "$VERIFY_DIR/verify" \
    "$RELEASE_DIR/public-build.json" "$RELEASE_DIR/public-build.json" \
    "$RELEASE_DIR/pair-manifest.json" "$RELEASE_DIR/Inputia.app" \
    "$RELEASE_DIR/InputiaUnifiedCandidate.app"
  /bin/rm -rf "$VERIFY_DIR"
fi
printf 'releaseDirectory=%s\ncontrolApp=%s\ninputiaApp=%s\nsettingsApp=%s\npairManifest=%s\ninstalled=false\npublicReleaseEligible=false\n' \
  "$RELEASE_DIR" "$RELEASE_DIR/Inputia.app" "$RELEASE_DIR/InputiaUnifiedCandidate.app" \
  "$RELEASE_DIR/Inputia 设置.app" "$RELEASE_DIR/pair-manifest.json"
