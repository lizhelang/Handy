#!/usr/bin/env python3
"""只取得锁定的上游资源；下载、源码和许可证均保留在本目录 artifacts 中。"""
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import argparse
import tempfile
import os
from source_snapshot import publish, verify

ROOT = Path(__file__).resolve().parent
ARTIFACTS = ROOT / "artifacts"
LOCK = json.loads((ROOT / "sources.lock.json").read_text())


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def fetch(url, expected, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        partial = path.with_name(path.name + ".part")
        subprocess.run(["/usr/bin/curl", "--fail", "--location", "--silent", "--show-error", "--retry", "3", url, "-o", str(partial)], check=True)
        if digest(partial) != expected:
            raise RuntimeError("download checksum mismatch: " + path.name)
        partial.replace(path)
    if digest(path) != expected:
        raise RuntimeError("cached resource checksum mismatch: " + str(path))
    print("verifiedSource=" + path.name + " sha256=" + expected, flush=True)


def safe_member(member):
    for name in [member.name] + ([member.linkname] if member.issym() or member.islnk() else []):
        path = PurePosixPath(name)
        if path.is_absolute() or ".." in path.parts:
            raise RuntimeError("unsafe archive path: " + name)
    if not (member.isfile() or member.isdir() or member.issym() or member.islnk()):
        raise RuntimeError("unsupported archive entry: " + member.name)


def prepare_tree(tree):
  for resource in LOCK["archives"]:
    archive = ARTIFACTS / "downloads" / resource["file"]
    fetch(resource["url"], resource["sha256"], archive)
    destination = tree / resource["destination"]
    destination.mkdir(parents=True, exist_ok=True)
    # 每轮在新树中展开，不接受旧 stamp 作为来源证明。
    with tarfile.open(archive) as bundle:
        members = bundle.getmembers()
        if "include" in resource:
            members = [m for m in members if any(m.name == prefix or m.name.startswith(prefix + "/") for prefix in resource["include"])]
        for member in members:
            safe_member(member)
            resolved = (destination / member.name).resolve()
            if destination.resolve() != resolved and destination.resolve() not in resolved.parents:
                raise RuntimeError("archive path escapes extraction root")
            bundle.extract(member, destination)

  for resource in LOCK["files"]:
    fetch(resource["url"], resource["sha256"], ARTIFACTS / resource["path"])
    target = tree / resource["path"]
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ARTIFACTS / resource["path"], target)

  sources = tree / "sources"
  core = sources / ("librime-" + LOCK["librime_commit"])
  plugins = {
    "lua": "librime-lua-68f9c364a2d25a04c7d4794981d7c796b05ab627",
    "octagram": "librime-octagram-dfcc15115788c828d9dd7b4bff68067d3ce2ffb8",
    "predict": "librime-predict-920bd41",
}
  for name, directory in plugins.items():
    destination = core / "plugins" / name
    if not destination.exists():
        shutil.copytree(sources / directory, destination)
  lua = core / "plugins/lua/thirdparty"
  if not lua.exists():
    shutil.copytree(sources / "librime-lua-0752fb3246c528f07feceb68972e5ac6b7f0436a", lua)
  for filename, source in {
    "librime-LICENSE": core / "LICENSE",
    "librime-lua-LICENSE": sources / plugins["lua"] / "LICENSE",
    "librime-octagram-LICENSE": sources / plugins["octagram"] / "LICENSE",
    "librime-predict-LICENSE": sources / plugins["predict"] / "LICENSE",
    "Boost-LICENSE_1_0.txt": sources / "boost_1_89_0/LICENSE_1_0.txt",
    "Lua-copyright-in-lua.h": lua / "lua5.4/lua.h",
    "darts-clone-COPYING": tree / "deps/include/COPYING.darts-clone",
    "X11-copyright-in-keysym.h": tree / "deps/include/X11/keysym.h",
    "X11-copyright-in-keysymdef.h": tree / "deps/include/X11/keysymdef.h",
}.items():
    shutil.copy2(source, tree / "licenses" / filename)
  return {"source_root": str(core.relative_to(tree)), "deps_root": "deps",
          "release_root": "release", "licenses_root": "licenses",
          "boost_root": "sources/boost_1_89_0"}


def main():
    # 直接运行与外层build运行采用同一权限，避免umask改变树身份或暴露暂存材料。
    os.umask(0o077)
    parser = argparse.ArgumentParser()
    parser.add_argument("command", nargs="?", default="fetch", choices=["fetch", "verify-current"])
    parser.add_argument("--snapshot", type=Path, default=ARTIFACTS / "source-snapshot.json")
    args = parser.parse_args()
    lock_path = ROOT / "sources.lock.json"
    if args.command == "verify-current":
        snapshot = verify(args.snapshot, lock_path)
        print("sourceSnapshotVerified=" + snapshot["generation_id"] + " treeDigest=" + snapshot["tree_digest"])
        return
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="source-expand-", dir=ARTIFACTS) as temporary:
        tree = Path(temporary) / "tree"
        tree.mkdir()
        roots = prepare_tree(tree)
        snapshot, reused = publish(tree, ARTIFACTS, lock_path, roots)
    print("sourceGeneration=" + snapshot["generation_id"] + " reused=" + str(reused).lower())
    print("sourceSnapshot=" + str(ARTIFACTS / "source-snapshot.json"))


if __name__ == "__main__":
    main()
