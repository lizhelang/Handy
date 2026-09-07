#!/usr/bin/env python3
"""候选专用完整 RimeData；只展开锁定安装包，不执行任何安装脚本。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import uuid

ROOT = Path(__file__).resolve().parent
HOST = ROOT.parent
MANIFEST = "inputia-candidate-resources.json"
EXTENSIONS = ["inputia_luna_pinyin.dict.yaml", "inputia_idiom.dict.yaml",
              "inputia_poetry.dict.yaml", "inputia_classical.dict.yaml", "inputia_ext_chars.dict.yaml"]
SCHEMAS = ["luna_pinyin_simp", "double_pinyin", "double_pinyin_flypy", "double_pinyin_sogou",
           "guobiao_bispell", "double_pinyin_mspy", "double_pinyin_abc", "double_pinyin_pyjj",
           "double_pinyin_st", "stroke", "terra_pinyin"]
EXTENDED_SCHEMAS = ["luna_pinyin", "double_pinyin", "double_pinyin_flypy", "double_pinyin_sogou",
                    "double_pinyin_mspy", "double_pinyin_abc", "double_pinyin_pyjj", "double_pinyin_st"]


def sha(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def no_links(path):
    """构建路径不得借用符号链接别名；不把同 UID 文件权限声称为安全沙箱。"""
    for current in [path] + list(path.parents):
        if current.is_symlink():
            raise RuntimeError("candidate resource path contains symbolic link")


def regular(path):
    no_links(path)
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
        raise RuntimeError("candidate resource is not a single regular file: " + str(path))


def inventory(directory):
    no_links(directory)
    if not directory.is_dir():
        raise RuntimeError("resource directory missing")
    result = {}
    for path in sorted(directory.rglob("*")):
        no_links(path)
        if path.is_dir():
            continue
        regular(path)
        relative = path.relative_to(directory).as_posix()
        if relative != MANIFEST:
            result[relative] = {"sha256": sha(path), "bytes": path.stat().st_size}
    return result


def validate_output(output, run_id):
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", run_id):
        raise RuntimeError("invalid candidate run ID")
    allowed = [HOST / "candidate-builds" / run_id / "RimeData",
               ROOT / "artifacts/outputs" / run_id / "RimeData"]
    if not output.is_absolute() or output not in allowed or output.resolve() != output:
        raise RuntimeError("output must be this candidate run's exact resource directory")
    no_links(output)
    if output.exists():
        inventory(output)


def fetch(resource, cache, offline):
    path = cache / resource["file"]
    no_links(path)
    if path.exists():
        regular(path)
        if sha(path) != resource["sha256"]:
            raise RuntimeError("cached source checksum mismatch: " + resource["file"])
        return path
    if offline:
        raise RuntimeError("locked source missing in offline mode: " + resource["file"])
    cache.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix="download-", dir=cache)
    os.close(descriptor)
    temporary = Path(temporary)
    try:
        subprocess.run(["/usr/bin/curl", "--proto", "=https", "--proto-redir", "=https",
                        "--fail", "--location", "--silent", "--show-error", "--retry", "2",
                        "--connect-timeout", "10", "--max-time", "120", resource["url"],
                        "-o", str(temporary)], check=True)
        if sha(temporary) != resource["sha256"]:
            raise RuntimeError("download checksum mismatch: " + resource["file"])
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)
    return path


def copy_schema_archive(archive, resource, output):
    """仅复制锁定的普通文件；不执行 extractall，不跟随归档链接。"""
    with tarfile.open(archive) as bundle:
        members = bundle.getmembers()
        for original, destination in resource["files"].items():
            if Path(original).name != original or Path(destination).name != destination:
                raise RuntimeError("schema mapping must use simple filenames")
            name = resource["prefix"] + "/" + original
            matching = [entry for entry in members if entry.name == name]
            if len(matching) != 1 or not matching[0].isfile() or matching[0].size > 4 * 1024 * 1024:
                raise RuntimeError("locked archive member missing, duplicated or unsafe")
            with bundle.extractfile(matching[0]) as stream:
                (output / destination).write_bytes(stream.read())


def transform(output, legacy_script):
    """保留原产品 schema 列表、搜狗方案和词典扩展，不执行旧下载/安装资源脚本。"""
    script = legacy_script.read_text()
    marker = '/bin/cat >"$BUILD_DIR/double_pinyin_sogou.schema.yaml" <<\'YAML\'\n'
    if script.count(marker) != 1:
        raise RuntimeError("legacy Sogou template contract changed")
    template = script.split(marker, 1)[1].split("\nYAML\n", 1)
    if len(template) != 2:
        raise RuntimeError("Sogou template terminator missing")
    (output / "double_pinyin_sogou.schema.yaml").write_text(template[0] + "\n")

    path = output / "default.yaml"
    lines = path.read_text().splitlines(keepends=True)
    start = [index for index, line in enumerate(lines) if line == "schema_list:\n"]
    if len(start) != 1:
        raise RuntimeError("default schema list is ambiguous")
    first = start[0]
    end = first + 1
    while end < len(lines) and (not lines[end].strip() or lines[end][0].isspace()):
        end += 1
    lines[first:end] = ["schema_list:\n"] + ["  - schema: " + schema + "\n" for schema in SCHEMAS]
    path.write_text("".join(lines))

    # 与旧打包的可选 emoji 过滤器处理逐行一致；该可选依赖原来也没有随包提供。
    path = output / "guobiao_bispell.schema.yaml"
    lines = path.read_text().splitlines(keepends=True)
    result, index = [], 0
    while index < len(lines):
        line = lines[index]
        if line.rstrip("\r\n") == "  - name: emoji_suggestion":
            index += 3
        elif line.rstrip("\r\n") == "    - simplifier@emoji_suggestion":
            index += 1
        elif line.rstrip("\r\n") == "emoji_suggestion:":
            index += 4
        else:
            result.append(line)
            index += 1
    path.write_text("".join(result))
    for schema in EXTENDED_SCHEMAS:
        path = output / (schema + ".schema.yaml")
        if not path.is_file():
            raise RuntimeError("required existing schema disappeared: " + schema)
        text = path.read_text()
        text = re.sub(r"(?m)^([ \t]*dictionary:[ \t]*)luna_pinyin([ \t]*(?:#.*)?)$",
                      r"\1inputia_luna_pinyin", text)
        path.write_text(text)
    for schema in SCHEMAS:
        if not (output / (schema + ".schema.yaml")).is_file():
            raise RuntimeError("required schema missing: " + schema)


def assemble(base, schema_sources, extension_root, legacy_script, output):
    base_files = inventory(base)
    shutil.copytree(base, output)
    for resource, archive in schema_sources:
        copy_schema_archive(archive, resource, output)
    local_files = {}
    # 不只复制既有五份，仓库以后新增的普通资源也随同保留。
    for relative, details in inventory(extension_root).items():
        source = extension_root / relative
        target = output / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        local_files[relative] = details
    if not all(name in local_files for name in EXTENSIONS):
        raise RuntimeError("Inputia extension dictionaries incomplete")
    transform(output, legacy_script)
    return base_files, local_files


def verify_output(output):
    regular(output / MANIFEST)
    evidence = json.loads((output / MANIFEST).read_text())
    if evidence.get("schema_version") != 1 or evidence.get("files") != inventory(output):
        raise RuntimeError("candidate resource inventory mismatch")
    if evidence["source_lock_sha256"] != sha(ROOT / "sources.lock.json"):
        raise RuntimeError("candidate resource source lock changed")
    if evidence["prepare_script_sha256"] != sha(Path(__file__)) or evidence["legacy_transform_sha256"] != sha(HOST / "prepare-rime-data.sh"):
        raise RuntimeError("candidate resource transformation changed")
    if evidence["local_resources"] != inventory(HOST / "Resources/RimeData"):
        raise RuntimeError("repository resources changed")
    return evidence


def build(output, run_id, offline):
    validate_output(output, run_id)
    lock = json.loads((ROOT / "sources.lock.json").read_text())
    if lock["schema_version"] != 1:
        raise RuntimeError("unknown source lock version")
    cache = ROOT / "artifacts/downloads"
    sources = [(resource, fetch(resource, cache, offline)) for resource in [lock["base"]] + lock["schemas"]]
    output.parent.mkdir(parents=True, exist_ok=True)
    no_links(output.parent)
    with tempfile.TemporaryDirectory(prefix="resource-build-", dir=output.parent) as scratch:
        scratch = Path(scratch)
        expanded = scratch / "expanded"
        # pkgutil 只展开归档，不调用 installer，也不执行 Scripts/postinstall。
        subprocess.run(["/usr/sbin/pkgutil", "--expand-full", str(sources[0][1]), str(expanded)], check=True)
        staging = scratch / "RimeData"
        base = expanded / lock["base"]["shared_support"]
        base_files, local_files = assemble(base, sources[1:], HOST / "Resources/RimeData",
                                          HOST / "prepare-rime-data.sh", staging)
        license_path = expanded / lock["base"]["license"]
        regular(license_path)
        shutil.copyfile(license_path, staging / "Squirrel-release.LICENSE.txt")
        evidence = {
            "schema_version": 1, "source_lock_sha256": sha(ROOT / "sources.lock.json"),
            "prepare_script_sha256": sha(Path(__file__)),
            "legacy_transform_sha256": sha(HOST / "prepare-rime-data.sh"),
            "source_archives": {resource["file"]: resource["sha256"] for resource, _ in sources},
            "base_resources": base_files, "local_resources": local_files,
            "files": inventory(staging), "schemas": SCHEMAS,
            "daily_installation_read": False, "installer_executed": False,
        }
        (staging / MANIFEST).write_text(json.dumps(evidence, ensure_ascii=False, sort_keys=True, indent=2) + "\n")
        verify_output(staging)
        backup = None
        if output.exists():
            backup = output.with_name("RimeData.previous." + uuid.uuid4().hex)
            output.rename(backup)
        try:
            staging.rename(output)
        except BaseException:
            if backup is not None and not output.exists():
                backup.rename(output)
            raise
        if backup is not None:
            print("previousCandidateResources=" + str(backup))
    verified = verify_output(output)
    print("candidateRimeData=pass files=" + str(len(verified["files"])) + " output=" + str(output))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()
    validate_output(args.output, args.run_id)
    if args.verify_only:
        verified = verify_output(args.output)
        print("candidateRimeDataVerified=" + str(len(verified["files"])))
    else:
        build(args.output, args.run_id, args.offline)
