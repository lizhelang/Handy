#!/usr/bin/env python3
"""固定 v2 成套本地制品为 ZIP；只校验和包装，不安装、不签名、不读取私钥。"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile


APPS = ("Inputia.app", "InputiaUnifiedCandidate.app", "Inputia 设置.app")
REQUIRED = (*APPS, "public-build.json", "public-key.x963", "pair-manifest.json", "metadata/build-context.json")
TOOL_FILES = (
    "macos/InputiaInputMethod/update-candidate.py",
    "macos/InputiaInputMethod/install-check.sh",
    "macos/InputiaInputMethod/Tools/InputiaTISTool.swift",
    "native/unified-pair-auth/UnifiedPairAuth.swift",
    "native/unified-pair-auth/ReleasePairAuthVerify.swift",
    "native/unified-pair-auth/build_trust.py",
)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def command_text(run_id, apply):
    action = " --apply" if apply else ""
    prompt = "" if not apply else "printf '\\n输入“安装”并回车才开始；其他输入取消：'; IFS= read -r answer; [[ \"$answer\" == '安装' ]] || { echo '已取消；未调用更新器。'; exit 0; }"
    return f'''#!/bin/bash
set -euo pipefail
PACKAGE_DIR="$(cd "$(dirname "${{BASH_SOURCE[0]}}")" && pwd -P)"
{prompt}
/usr/bin/python3 -B "$PACKAGE_DIR/payload/tools/macos/InputiaInputMethod/update-candidate.py" \\
  --run-id '{run_id}' --release-v2 \\
  --control-app "$PACKAGE_DIR/Inputia.app" \\
  --inputia-app "$PACKAGE_DIR/InputiaUnifiedCandidate.app" \\
  --settings-app "$PACKAGE_DIR/Inputia 设置.app" \\
  --pair-manifest "$PACKAGE_DIR/pair-manifest.json" \\
  --public-build "$PACKAGE_DIR/public-build.json" \\
  --build-context "$PACKAGE_DIR/metadata/build-context.json"{action}
'''


def real_directory(path):
    path = Path(path).expanduser().absolute()
    if path.resolve() != path or path.is_symlink() or not path.is_dir():
        raise ValueError(f"构建目录必须是无符号链接的目录：{path}")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--run-id", default="trial-20260905")
    args = parser.parse_args()
    release = real_directory(args.release_dir)
    output = args.output.expanduser().absolute()
    if output.exists() or output.is_symlink():
        raise ValueError(f"拒绝覆盖已有输出：{output}")
    if output.parent.resolve() != output.parent.absolute():
        raise ValueError("输出父目录不能是符号链接")
    for relative in REQUIRED:
        path = release / relative
        if path.is_symlink() or not path.exists():
            raise ValueError(f"缺少 v2 制品或存在符号链接：{relative}")
    public = json.loads((release / "public-build.json").read_text(encoding="utf-8"))
    context = json.loads((release / "metadata/build-context.json").read_text(encoding="utf-8"))
    if public.get("schema_version") != 2 or context.get("schema_version") != 1:
        raise ValueError("不是受支持的 v2 本地构建元数据")
    if public.get("release_id") != context.get("release_id") or public.get("source_commit") != context.get("source_commit"):
        raise ValueError("public-build 与 release context 未绑定同一 release")
    if public.get("product_id") != "com.inputia" or context.get("public_release_eligible") is not False:
        raise ValueError("产品身份或公开发布门禁不符合本地制品合同")
    if not isinstance(args.run_id, str) or not args.run_id.replace("-", "").replace("_", "").isalnum() or not (1 <= len(args.run_id) <= 64):
        raise ValueError("无效目标数据 profile")
    if any(path.name.lower() in {"signing-private.x963", "private-key.x963"} or path.suffix == ".pkcs8" or (path.suffix == ".pem" and path.name != "cacert.pem") for path in release.rglob("*")):
        raise ValueError("输出目录疑似包含私钥材料")
    for app_name in APPS:
        app = release / app_name
        subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(app)], check=True)
        with (app / "Contents/Info.plist").open("rb") as stream:
            info = plistlib.load(stream)
        if info.get("InputiaReleaseID") != public["release_id"] or info.get("InputiaSourceCommit") != public["source_commit"]:
            raise ValueError(f"{app_name} 的 release 元数据不一致")
        if any(key in info for key in ("HandyProfileRunID", "HandyDevelopmentCandidate", "InputiaProfileRunID", "InputiaDevelopmentCandidate")):
            raise ValueError(f"{app_name} 仍带有 v1 开发身份标记")
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".inputia-v2-package-", dir=output.parent) as temporary:
        staging = Path(temporary) / release.name
        subprocess.run(["/usr/bin/ditto", str(release), str(staging)], check=True)
        payload = staging / "payload" / "tools"
        for relative in TOOL_FILES:
            destination = payload / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            source = Path(__file__).resolve().parents[1] / relative
            if not source.is_file():
                raise ValueError(f"缺少 v2 更新器依赖：{relative}")
            destination.write_bytes(source.read_bytes())
            destination.chmod(0o755 if source.suffix in (".py", ".sh") else 0o644)
        for name, apply in (("安装v2体验版.command", True), ("仅检查v2更新.command", False)):
            command = staging / name
            command.write_text(command_text(args.run_id, apply), encoding="utf-8")
            command.chmod(0o755)
        subprocess.run([
            "/usr/bin/ditto", "-c", "-k", "--keepParent", "--norsrc", "--noextattr", "--noacl", "--noqtn",
            str(staging), str(output),
        ], check=True)
    output.chmod(0o600)
    sidecar = output.with_suffix(output.suffix + ".sha256")
    sidecar.write_text(f"{digest(output)}  {output.name}\n", encoding="utf-8")
    sidecar.chmod(0o600)
    print(json.dumps({"zip": str(output), "sha256": digest(output), "release_id": public["release_id"], "installed": False, "public_release_eligible": False}, ensure_ascii=False))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"v2 本地制品包装失败：{error}")
