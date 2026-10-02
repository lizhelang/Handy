#!/usr/bin/env python3
"""绑定探针与本次链接输入；只在重新链接后签发新的验证元数据。"""
import hashlib
import json
from pathlib import Path
import sys


MANIFEST_SCHEMA_VERSION = 1


def resolve_manifest_artifact(output, metadata, field, relative, architecture):
    """把清单引用限制在当前制品根；旧绝对路径只作为布局标识兼容。"""
    value = metadata.get(field)
    expected = Path(relative)
    schema = metadata.get("schema_version")
    if schema == MANIFEST_SCHEMA_VERSION:
        if value != expected.as_posix():
            raise RuntimeError("static Rime manifest relative path mismatch: " + field)
    elif schema is None:
        legacy = Path(value) if isinstance(value, str) else Path()
        suffix = Path("native/static-rime/artifacts/output") / architecture / expected
        if not legacy.is_absolute() or legacy.parts[-len(suffix.parts) :] != suffix.parts:
            raise RuntimeError("static Rime legacy manifest path mismatch: " + field)
    else:
        raise RuntimeError("unsupported static Rime manifest schema")
    return output / expected


def manifest_changed_fields(observed, expected):
    """返回完整清单中缺失、新增或正文不同的字段。"""
    return sorted(
        key
        for key in set(observed) | set(expected)
        if key not in observed
        or key not in expected
        or observed[key] != expected[key]
    )


def sha(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def inputs(root, output):
    return {
        "library_sha256": sha(output / "lib/libinputia_rime_static.a"),
        "source_lock_sha256": sha(root / "sources.lock.json"),
        "probe_source_sha256": sha(root / "probe/main.cpp"),
        "probe_header_sha256": sha(output / "include/rime_api.h"),
        "source_snapshot_sha256": sha(output / "source-snapshot.json"),
    }


def begin(root, output):
    # 先撤销旧链接回执；失败构建不能继承上次成功证据。
    (output / "probe-link.json").unlink(missing_ok=True)
    (output / "probe-link-pending.json").write_text(json.dumps(inputs(root, output)))


def seal(root, output):
    pending = output / "probe-link-pending.json"
    before = json.loads(pending.read_text())
    if before != inputs(root, output):
        raise RuntimeError("static Rime link inputs changed while linking")
    receipt = {**before, "probe_sha256": sha(output / "static-rime-probe")}
    (output / "probe-link.json").write_text(json.dumps(receipt, indent=2) + "\n")
    pending.unlink()


def verify(root, output, metadata):
    expected = {**inputs(root, output), "probe_sha256": sha(output / "static-rime-probe")}
    for key, value in expected.items():
        if metadata.get(key) != value:
            raise RuntimeError("static Rime evidence binding mismatch: " + key)
    return expected


if __name__ == "__main__":
    root = Path(__file__).resolve().parent
    mode, arch = sys.argv[1:]
    if arch not in {"arm64", "x86_64"}:
        raise SystemExit("unsupported architecture")
    output = root / "artifacts/output" / arch
    if mode == "begin":
        begin(root, output)
    elif mode == "seal":
        seal(root, output)
    else:
        raise SystemExit("unknown link binding action")
