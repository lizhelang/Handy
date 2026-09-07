#!/usr/bin/env python3
"""Cargo 构建期只读门禁：不下载、不重建、不运行任何探针。"""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
from artifact_binding import verify

ROOT = Path(__file__).resolve().parent
supplied = Path(sys.argv[1])
architecture = sys.argv[2]
if not supplied.is_absolute() or supplied.resolve() != supplied:
    raise SystemExit("INPUTIA_STATIC_RIME_DIR must be an explicit canonical absolute path")


def private_owned(path):
    metadata = path.lstat()
    if stat.S_ISLNK(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o022:
        raise SystemExit("static Rime input is linked, foreign-owned or writable by others: " + str(path))
    if stat.S_ISREG(metadata.st_mode) and metadata.st_nlink != 1:
        raise SystemExit("static Rime input cannot be a hard link")


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


library = supplied / "lib/libinputia_rime_static.a"
for path in [supplied, supplied / "lib", library, supplied / "manifest.json", supplied / "sources.lock.json", supplied / "source-snapshot.json", supplied / "static-rime-probe", supplied / "include", supplied / "include/rime_api.h"]:
    private_owned(path)
manifest = json.loads((supplied / "manifest.json").read_text())
verify(ROOT, supplied, manifest)
if manifest["architecture"] != architecture or manifest["minimum_macos"] != "13.0":
    raise SystemExit("static Rime target metadata mismatch")
if manifest["library"] != str(library) or manifest["library_sha256"] != digest(library):
    raise SystemExit("static Rime archive does not match its verified manifest")
if manifest["source_lock_sha256"] != digest(ROOT / "sources.lock.json") or digest(supplied / "sources.lock.json") != digest(ROOT / "sources.lock.json"):
    raise SystemExit("static Rime archive was built from a different source lock")
actual_arch = subprocess.check_output(["/usr/bin/lipo", "-archs", str(library)], text=True).strip()
if actual_arch != architecture:
    raise SystemExit("static Rime archive architecture mismatch: " + actual_arch)
print("verifiedStaticRimeLinkInput=" + str(library))
