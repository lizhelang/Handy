#!/bin/bash
# 构建融合正式版与最终配对清单；不安装、不启动应用、不修改用户数据。
set -euo pipefail
umask 077

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RELEASE_PYTHON="${INPUTIA_RELEASE_PYTHON:-python3}"
if [[ "${1:-}" == "--help" ]]; then
  cat <<'HELP'
用法：scripts/build-inputia-release.sh [--preflight | --preflight-public | --build-local]
默认只读预检；--build-local 才执行已有本机 v1 配对构建。
本机构建必须显式提供：
  INPUTIA_PAIR_BUILD_METADATA  已有配对公开构建元数据的绝对路径
  INPUTIA_PAIR_PRIVATE_KEY     配对签名私钥的绝对路径（不复制、不输出内容）
  INPUTIA_CODESIGN_IDENTITY    已有稳定证书身份，不能为临时签名 -
可选：
  INPUTIA_PROFILE_RUN_ID       默认 trial-20260905
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
case "${1:---preflight}" in
  --preflight) exec "$RELEASE_PYTHON" "$REPO_ROOT/scripts/inputia_release.py" preflight --mode local ;;
  --preflight-public) exec "$RELEASE_PYTHON" "$REPO_ROOT/scripts/inputia_release.py" preflight --mode public ;;
  --build-local) ;;
  *) echo "未知参数；使用 --help 查看用法" >&2; exit 2 ;;
esac
# 元数据与工具链检查发生在读取签名输入及创建输出之前。
"$RELEASE_PYTHON" "$REPO_ROOT/scripts/inputia_release.py" preflight --mode local
: "${INPUTIA_PAIR_BUILD_METADATA:?必须提供配对公开构建元数据路径}"
: "${INPUTIA_PAIR_PRIVATE_KEY:?必须提供配对签名私钥路径}"
: "${INPUTIA_CODESIGN_IDENTITY:?必须提供已有稳定签名证书身份}"
if [[ "$INPUTIA_CODESIGN_IDENTITY" == "-" ]]; then
  echo "正式版拒绝临时签名身份" >&2
  exit 2
fi
export INPUTIA_PROFILE_RUN_ID="${INPUTIA_PROFILE_RUN_ID:-trial-20260905}"
if [[ ! "$INPUTIA_PROFILE_RUN_ID" =~ ^[A-Za-z0-9_-]{1,64}$ ]]; then
  echo "无效 profile ID" >&2
  exit 2
fi
for input_path in "$INPUTIA_PAIR_BUILD_METADATA" "$INPUTIA_PAIR_PRIVATE_KEY"; do
  if [[ "$input_path" != /* || ! -f "$input_path" || -L "$input_path" ]]; then
    echo "配对输入必须是绝对路径的普通文件，不能使用符号链接" >&2
    exit 2
  fi
done

cd "$REPO_ROOT"
# 校验公开元数据合同；只生成公钥常量，不读取私钥。
/usr/bin/python3 native/unified-pair-auth/build_trust.py \
  --metadata "$INPUTIA_PAIR_BUILD_METADATA" --run-id "$INPUTIA_PROFILE_RUN_ID" --emit rust >/dev/null
BUILD_BASE="$HOME/Library/Application Support/HandyUnifiedBuilds"
mkdir -p "$BUILD_BASE"
RELEASE_DIR="$(/usr/bin/mktemp -d "$BUILD_BASE/release-XXXXXXXX")"
chmod 700 "$RELEASE_DIR"
METADATA_DIR="$RELEASE_DIR/metadata"
"$RELEASE_PYTHON" scripts/inputia_release.py prepare --mode local --output-dir "$METADATA_DIR"
export INPUTIA_RELEASE_CONTEXT="$METADATA_DIR/build-context.json"
export INPUTIA_RELEASE_PYTHON="$RELEASE_PYTHON"
SIGN_OVERLAY="$RELEASE_DIR/signing-overlay.json"
"$RELEASE_PYTHON" - "$SIGN_OVERLAY" "$INPUTIA_CODESIGN_IDENTITY" "$RELEASE_DIR" "$INPUTIA_PROFILE_RUN_ID" <<'PY'
import json
import plistlib
import sys
from pathlib import Path
with Path("src-tauri/InputiaReleaseInfo.plist").open("rb") as stream:
    info = plistlib.load(stream)
info["HandyProfileRunID"] = sys.argv[4]
# 仅保留当前本机 v1 信任桥；渠道不进入不可变程序。
info["HandyDevelopmentCandidate"] = True
release_plist = Path(sys.argv[3]) / "InputiaReleaseInfo.plist"
with release_plist.open("wb") as stream:
    plistlib.dump(info, stream)
Path(sys.argv[1]).write_text(json.dumps({"bundle": {"macOS": {"signingIdentity": sys.argv[2], "infoPlist": str(release_plist)}}}) + "\n")
PY
"$RELEASE_PYTHON" scripts/inputia_release.py apply-plist --role control \
  --plist "$RELEASE_DIR/InputiaReleaseInfo.plist" --context "$INPUTIA_RELEASE_CONTEXT"
export HANDY_UNIFIED_PAIR_BUILD="$INPUTIA_PAIR_BUILD_METADATA"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/src-tauri/target}"
# 固定绝对输出路径，避免 Rust/Tauri 对相对目录有不同解释。
CARGO_TARGET_DIR="$(/usr/bin/python3 - "$CARGO_TARGET_DIR" <<'PY'
import os
import sys
print(os.path.abspath(sys.argv[1]))
PY
)"
export CARGO_TARGET_DIR
export CMAKE_POLICY_VERSION_MINIMUM="${CMAKE_POLICY_VERSION_MINIMUM:-3.5}"
# 保留标准 beforeBuildCommand，包括语音辅助程序准备。
bun run tauri build --bundles app \
  --config src-tauri/tauri.unified-candidate.conf.json \
  --config src-tauri/tauri.inputia-release.conf.json \
  --config "$SIGN_OVERLAY"
CONTROL_APP="$CARGO_TARGET_DIR/release/bundle/macos/Inputia.app"
[[ -d "$CONTROL_APP" ]] || { echo "控制中心构建产物缺失" >&2; exit 1; }

INPUTIA_UNIFIED_CANDIDATE=1 INPUTIA_RELEASE=1 \
  zsh macos/InputiaInputMethod/build.sh
IME_BUILD="$REPO_ROOT/macos/InputiaInputMethod/candidate-builds/$INPUTIA_PROFILE_RUN_ID"
# 固定最终包副本，避免后续构建改变清单对应的文件。
/usr/bin/ditto "$CONTROL_APP" "$RELEASE_DIR/Inputia.app"
/usr/bin/ditto "$IME_BUILD/InputiaUnifiedCandidate.app" "$RELEASE_DIR/InputiaUnifiedCandidate.app"
/usr/bin/ditto "$IME_BUILD/Inputia 候选设置.app" "$RELEASE_DIR/Inputia 设置.app"
"$RELEASE_PYTHON" scripts/inputia_release.py verify-bundles \
  --directory "$RELEASE_DIR" --context "$INPUTIA_RELEASE_CONTEXT"
/usr/bin/swiftc -parse-as-library \
  native/unified-pair-auth/UnifiedPairAuth.swift \
  native/unified-pair-auth/PairAuthTool.swift \
  -o "$RELEASE_DIR/PairAuthTool"
/usr/bin/python3 native/unified-pair-auth/build_trust.py \
  --metadata "$INPUTIA_PAIR_BUILD_METADATA" --run-id "$INPUTIA_PROFILE_RUN_ID" \
  --sign-pair --handy "$RELEASE_DIR/Inputia.app" \
  --inputia "$RELEASE_DIR/InputiaUnifiedCandidate.app" \
  --build-tool "$RELEASE_DIR/PairAuthTool" \
  --private-key "$INPUTIA_PAIR_PRIVATE_KEY" \
  --manifest "$RELEASE_DIR/pair-manifest.json"
printf 'releaseDirectory=%s\ncontrolApp=%s\ninputiaApp=%s\nsettingsApp=%s\npairManifest=%s\ninstalled=false\npublicReleaseEligible=false\n' \
  "$RELEASE_DIR" "$RELEASE_DIR/Inputia.app" "$RELEASE_DIR/InputiaUnifiedCandidate.app" \
  "$RELEASE_DIR/Inputia 设置.app" "$RELEASE_DIR/pair-manifest.json"
