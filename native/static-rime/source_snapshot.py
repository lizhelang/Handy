"""源码 generation 的完整树校验；未知旧目录不能自行晋升为受检来源。"""
import hashlib
import json
import os
from pathlib import Path
import stat
import uuid


def file_digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def tree_entries(root):
    root = Path(root)
    if root.is_symlink() or not root.is_dir():
        raise RuntimeError("source tree root is not a real directory")
    entries = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in sorted(dirs + files):
            path = Path(directory) / name
            info = path.lstat()
            entry = {"path": path.relative_to(root).as_posix(), "mode": stat.S_IMODE(info.st_mode)}
            if stat.S_ISLNK(info.st_mode):
                resolved = path.resolve()
                if not resolved.is_relative_to(root.resolve()):
                    raise RuntimeError("source link escapes tree: " + str(path))
                entry.update(kind="symlink", target=os.readlink(path))
            elif stat.S_ISREG(info.st_mode):
                if info.st_nlink != 1:
                    raise RuntimeError("hardlinked source file: " + str(path))
                entry.update(kind="file", sha256=file_digest(path))
            elif stat.S_ISDIR(info.st_mode):
                entry.update(kind="directory")
            else:
                raise RuntimeError("unsupported source file: " + str(path))
            entries.append(entry)
    return sorted(entries, key=lambda entry: entry["path"])


def entries_digest(entries):
    return hashlib.sha256(json.dumps(entries, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def verify(snapshot_path, lock_path):
    snapshot_path = Path(snapshot_path)
    if snapshot_path.is_symlink():
        raise RuntimeError("source snapshot cannot be a symlink")
    snapshot = json.loads(snapshot_path.read_text())
    if snapshot.get("schema_version") != 1 or snapshot["lock_sha256"] != file_digest(Path(lock_path)):
        raise RuntimeError("source snapshot lock/schema mismatch")
    root = Path(snapshot["tree_root"])
    if not root.is_absolute() or root.resolve() != root:
        raise RuntimeError("source tree must be canonical")
    if root.parent.name != snapshot["generation_id"] or root.name != "tree":
        raise RuntimeError("source generation identity mismatch")
    for key, relative in snapshot["relative_roots"].items():
        expected = root / relative
        if not expected.resolve().is_relative_to(root) or snapshot[key] != str(expected) or not expected.is_dir():
            raise RuntimeError("source root mapping mismatch: " + key)
    actual = tree_entries(root)
    if actual != snapshot["entries"] or entries_digest(actual) != snapshot["tree_digest"]:
        raise RuntimeError("source tree changed since extraction")
    return snapshot


def publish(staging_tree, artifacts, lock_path, relative_roots):
    """调用者只能传入本轮从已校验归档重新展开的独立树。"""
    staging_tree, artifacts = Path(staging_tree), Path(artifacts).resolve()
    entries = tree_entries(staging_tree)
    current = artifacts / "source-snapshot.json"
    if current.exists():
        try:
            old = verify(current, lock_path)
            if old["entries"] == entries and old["relative_roots"] == relative_roots:
                return old, True
        except (RuntimeError, ValueError, KeyError, OSError):
            pass  # 已改动 generation 保留，但不能再作为本轮构建来源。
    generation = uuid.uuid4().hex
    generation_root = artifacts / "source-generations" / generation
    generation_root.mkdir(parents=True)
    tree = generation_root / "tree"
    staging_tree.rename(tree)
    snapshot = {"schema_version": 1, "generation_id": generation,
                "tree_root": str(tree), "lock_sha256": file_digest(Path(lock_path)),
                "tree_digest": entries_digest(entries), "entries": entries,
                "relative_roots": relative_roots}
    snapshot.update({key: str(tree / relative) for key, relative in relative_roots.items()})
    encoded = json.dumps(snapshot, indent=2, sort_keys=True) + "\n"
    (generation_root / "source-snapshot.json").write_text(encoded)
    temporary = artifacts / (".source-snapshot-" + generation)
    temporary.write_text(encoded)
    temporary.replace(current)
    return snapshot, False
