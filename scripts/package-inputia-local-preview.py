#!/usr/bin/env python3
"""给已有本机构建添加双击入口；只包装，不安装、不签名、不读取私钥。"""

import argparse
import importlib.util
from pathlib import Path
import plistlib
import re
import shutil
import sys
import tempfile


sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parent.parent
TOOL_FILES = (
    "macos/InputiaInputMethod/update-candidate.py",
    "macos/InputiaInputMethod/install-check.sh",
    "macos/InputiaInputMethod/Tools/CandidateUpdateVerify.swift",
    "macos/InputiaInputMethod/Tools/InputiaTISTool.swift",
    "native/unified-pair-auth/build_trust.py",
    "native/unified-pair-auth/UnifiedPairAuth.swift",
)


def command_text(run_id, apply):
    explanation = f"""Inputia 本机体验版：更新现有的两个组件

本入口用于升级本机已有安装；本次体验交付的原版本为 1.1.0/build84。

将替换：/Applications/Inputia.app（兼容旧名称 Inputia Candidate.app）
        ~/Library/Input Methods/InputiaUnifiedCandidate.app
继续使用原有数据域：~/Library/Application Support/HandyUnifiedCandidate/{run_id}
更新器保留原应用包和配对清单，备份位于：
  ~/Library/Application Support/HandyUnifiedBuilds/permission-update-*
上述是程序备份，不是个人数据库、录音或模型的完整备份。
更新时会暂时切到 ABC、停止并重启两个组件，随后校验并恢复原输入源。
签名身份、配对或维护检查失败会阻止更新；失败信息及备份路径会保留在终端。

这是用于当前 Mac、当前账号的体验包，需要本机 Python 3.9+ 和 Swift 编译器。
无需原源码仓库；尚未完成 Apple 公证或干净机器安装验收。

本版旧学习库交接尚未接通：旧学习排序、个人记忆英文补全、输入法剪贴召回、
旧学习语音热词、此域新学习与旧历史导入暂不可用；现有个人数据保留。
基础 Rime、语音输出、主应用剪贴历史及显式热词沿用当前实现，待安装后体验验收。
"""
    if apply:
        action = """printf '\n输入“安装”并回车才开始；直接回车或输入其他内容将取消：'
answer=''
if ! IFS= read -r answer || [[ "$answer" != '安装' ]]; then
  printf '\n已取消；未调用更新器，未创建更新锁或修改安装。\n'
  exit 0
fi
"""
    else:
        action = """cat <<'CHECK'
本入口仅检查，不替换应用或切换输入源。
检查仍会在现有数据域创建/打开 .candidate-update.lock，并编译临时校验程序。
CHECK
"""
    apply_flag = " --apply" if apply else ""
    return f"""#!/bin/bash
set -euo pipefail
PACKAGE_DIR="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd -P)"
cat <<'EXPLANATION'
{explanation}EXPLANATION
{action}
pause_terminal() {{
  if [[ -t 0 ]]; then
    printf '\n按回车关闭此窗口。'
    IFS= read -r ignored || true
  fi
}}
if [[ ! -x /usr/bin/python3 ]] || ! /usr/bin/python3 -c 'import sys; sys.exit(sys.version_info < (3, 9))'; then
  printf '\n缺少 /usr/bin/python3 3.9+；请安装本机开发工具后重试。\n' >&2
  pause_terminal
  exit 1
fi
if [[ ! -x /usr/bin/swiftc ]] || ! /usr/bin/xcrun --find swiftc >/dev/null 2>&1; then
  printf '\n缺少 Swift 编译器；请安装 Xcode Command Line Tools 后重试。\n' >&2
  pause_terminal
  exit 1
fi
status=0
/usr/bin/python3 -B "$PACKAGE_DIR/payload/tools/macos/InputiaInputMethod/update-candidate.py" \\
  --run-id '{run_id}' \\
  --control-app "$PACKAGE_DIR/Inputia.app" \\
  --inputia-app "$PACKAGE_DIR/InputiaUnifiedCandidate.app" \\
  --pair-manifest "$PACKAGE_DIR/pair-manifest.json" \\
  --public-build "$PACKAGE_DIR/payload/public-build.json"{apply_flag} || status=$?
if [[ "$status" -eq 0 ]]; then
  printf '\n本次操作完成；以上更新器输出为准。\n'
else
  printf '\n操作失败（退出码 %s）。请保留上方信息及 updateBackup 路径，勿手动删除备份或重放安装。\n' "$status" >&2
fi
pause_terminal
exit "$status"
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-dir", type=Path, required=True, help="已有两组件和 pair-manifest.json 的构建目录")
    parser.add_argument("--public-build", type=Path, required=True, help="已有配对公开构建元数据；不能传私钥")
    parser.add_argument("--run-id", default="trial-20260905")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", args.run_id):
        parser.error("无效 profile ID")
    release = args.release_dir.expanduser().resolve(strict=True)
    metadata = args.public_build.expanduser().absolute()
    entries = ("安装体验版.command", "仅检查.command", "体验版说明.txt")
    for name in ("payload", *entries):
        if (release / name).exists() or (release / name).is_symlink():
            parser.error(f"目标已存在，拒绝覆盖：{release / name}")
    versions = []
    for name, prefix in (("Inputia.app", "Handy"), ("InputiaUnifiedCandidate.app", "Inputia")):
        app = release / name
        if app.is_symlink() or not app.is_dir():
            parser.error(f"构建组件缺失或是符号链接：{app}")
        with (app / "Contents/Info.plist").open("rb") as stream:
            info = plistlib.load(stream)
        if info.get(prefix + "ProfileRunID") != args.run_id or info.get(prefix + "DevelopmentCandidate") is not True:
            parser.error(f"组件不属于指定的本机 trial 数据域：{name}")
        versions.append((info["CFBundleShortVersionString"], info["CFBundleVersion"]))
    if versions[0] != versions[1]:
        parser.error("两个组件的版本或 build 不一致")
    if not (release / "pair-manifest.json").is_file():
        parser.error("缺少 pair-manifest.json")
    for relative in TOOL_FILES:
        if not (REPO / relative).is_file():
            parser.error(f"缺少更新器依赖：{relative}")
    spec = importlib.util.spec_from_file_location("preview_build_trust", REPO / "native/unified-pair-auth/build_trust.py")
    trust = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(trust)
    trust.load(metadata, args.run_id)

    # 只复制公开元数据和既有更新器依赖；不复制签名材料，不改动 app 包。
    with tempfile.TemporaryDirectory(prefix=".preview-package-", dir=release) as temporary:
        staging = Path(temporary)
        payload = staging / "payload"
        payload.mkdir(mode=0o700)
        for relative in TOOL_FILES:
            destination = payload / "tools" / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / relative, destination)
            destination.chmod(0o755 if destination.suffix in (".py", ".sh") else 0o644)
        public_copy = payload / "public-build.json"
        shutil.copyfile(metadata, public_copy)
        public_copy.chmod(0o600)
        trust.load(public_copy, args.run_id)
        for name, apply in ((entries[0], True), (entries[1], False)):
            command = staging / name
            command.write_text(command_text(args.run_id, apply), encoding="utf-8")
            command.chmod(0o755)
        version, build = versions[0]
        (staging / entries[2]).write_text(
            f"Inputia {version} / build {build} 本机体验版\n\n"
            "请将整个目录保留在一起，不要单独拖动 app 或 command。\n"
            "双击“安装体验版.command”，阅读说明并输入“安装”后才调用已有受控更新器。\n"
            "直接回车、输入其他内容或关闭确认窗口均取消；确认前不调用更新器。\n"
            "“仅检查.command”不会替换程序，但会创建/打开 profile 更新锁及临时校验程序。\n"
            f"本包只更新已有的两组件安装，保留 {args.run_id} 数据域；不是新账号安装器。\n"
            "程序及配对清单备份位于 ~/Library/Application Support/HandyUnifiedBuilds/permission-update-*。\n"
            "这不是数据库、录音或模型的完整备份。失败时保留终端信息和备份路径。\n"
            "Inputia 设置.app 可从包内单独打开；本入口复用两组件更新器，不额外安装此辅助入口。\n"
            "无需原源码仓库，需要 /usr/bin/python3 3.9+ 与 Swift 编译器（Xcode Command Line Tools）。\n"
            "尚未完成 Apple 公证和干净机器安装验收；仅供当前 Mac 的当前账号体验。\n"
            "旧学习库交接尚未接通：旧学习排序、个人记忆英文补全、输入法剪贴召回、旧学习语音热词、此域新学习及旧历史导入暂不可用；现有个人数据保留。\n"
            "基础 Rime、语音输出、主应用剪贴历史、显式热词及独立主库共享词不依赖此交接。\n"
            "安装包准备完成不代表已经安装或实际输入、语音和剪贴板体验验收通过。\n",
            encoding="utf-8",
        )
        (staging / entries[2]).chmod(0o644)
        for name in ("payload", *entries):
            (staging / name).rename(release / name)
    print(f"previewDirectory={release}")
    print(f"installCommand={release / entries[0]}")
    print(f"checkCommand={release / entries[1]}")
    print("installed=false")


if __name__ == "__main__":
    main()
