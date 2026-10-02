#!/usr/bin/env python3
"""核验 Mach-O、签名和真实合成 Rime 会话；不启动输入法，不接触系统输入/剪贴板。"""
import json
import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from artifact_binding import (
    MANIFEST_SCHEMA_VERSION,
    manifest_changed_fields,
    resolve_manifest_artifact,
    sha,
    verify,
)

ROOT = Path(__file__).resolve().parent
ARCH = sys.argv[1]
MODE = sys.argv[2] if len(sys.argv) == 3 else ""
if MODE not in {"--verify-only", "--publish"}:
    raise SystemExit("explicit --verify-only or --publish required")
if ARCH not in {"arm64", "x86_64"}:
    raise SystemExit("unsupported architecture")
OUTPUT = ROOT / "artifacts/output" / ARCH
LIBRARY = OUTPUT / "lib/libinputia_rime_static.a"
PROBE = OUTPUT / "static-rime-probe"


def command(args):
    result = subprocess.run(args, capture_output=True, text=True, check=True)
    return result.stdout + result.stderr


def text_sha(value):
    return hashlib.sha256(value.encode()).hexdigest()


def canonical_probe_log(value):
    return re.sub(
        r'synthetic_user_dir="[^"]+"',
        'synthetic_user_dir="$PROBE_RUN/user"',
        value,
    )


metadata_path = OUTPUT / ("manifest.json" if MODE == "--verify-only" else "probe-link.json")
published_metadata = json.loads(metadata_path.read_text())
binding = verify(ROOT, OUTPUT, published_metadata)
if MODE == "--verify-only":
    if (
        published_metadata.get("architecture") != ARCH
        or published_metadata.get("minimum_macos") != "13.0"
    ):
        raise RuntimeError("static Rime target metadata mismatch")
    if resolve_manifest_artifact(
        OUTPUT,
        published_metadata,
        "library",
        "lib/libinputia_rime_static.a",
        ARCH,
    ) != LIBRARY:
        raise RuntimeError("static Rime manifest library escaped its output root")
    if published_metadata.get("schema_version") == MANIFEST_SCHEMA_VERSION:
        probe_log = resolve_manifest_artifact(
            OUTPUT,
            published_metadata,
            "probe_log",
            "evidence/probe.log",
            ARCH,
        )
        if (
            not probe_log.is_file()
            or published_metadata.get("probe_log_sha256") != sha(probe_log)
        ):
            raise RuntimeError("static Rime probe log does not match its manifest")
minimums = []
for artifact in [LIBRARY, PROBE]:
    listing = command(["/usr/bin/otool", "-l", str(artifact)])
    versions = [line.split()[1] for line in listing.splitlines() if line.split()[:1] == ["minos"]]
    if not versions or any(tuple(map(int, version.split("."))) > (13, 0, 0) for version in versions):
        raise RuntimeError("unexpected minimum macOS in " + str(artifact))
    minimums.append(
        {
            "path": artifact.relative_to(OUTPUT).as_posix(),
            "versions": sorted(set(versions)),
            "objects": len(versions),
        }
    )
dependencies = command(["/usr/bin/otool", "-L", str(PROBE)])
for line in dependencies.splitlines()[1:]:
    dependency = line.strip().split(" (")[0]
    if dependency and not dependency.startswith(("/usr/lib/", "/System/Library/")):
        raise RuntimeError("non-system dynamic dependency: " + dependency)
signature = command(["/usr/bin/codesign", "-dv", "--verbose=4", str(PROBE)])
if "adhoc,runtime" not in signature:
    raise RuntimeError("probe is not hardened ad-hoc")
command(["/usr/bin/codesign", "--verify", "--strict", str(PROBE)])
entitlements = command(["/usr/bin/codesign", "-d", "--entitlements", ":-", str(PROBE)])
if "disable-library-validation" in entitlements or "allow-unsigned-executable-memory" in entitlements:
    raise RuntimeError("probe weakens hardened runtime")

run = Path(tempfile.mkdtemp(prefix="probe-run.", dir=ROOT / "artifacts"))
shared = run / "shared"
shutil.copytree(ROOT / "probe/fixtures", shared)
shutil.copyfile(shared / "static_probe.dict.yaml.in", shared / "static_probe.dict.yaml")
execution = subprocess.run([str(PROBE), str(shared), str(run)], capture_output=True, text=True)
log = execution.stdout + execution.stderr
(run / "probe.log").write_text(log)
print(log, end="")
if execution.returncode != 0:
    raise RuntimeError("signed Rime probe failed; see " + str(run / "probe.log"))
required = ["schema=static_pinyin keys=nihao candidate=true commit=true lua_filter=true", "schema=static_double keys=nihc candidate=true commit=true lua_filter=true", "rime_version=1.16.0 external_dylib_fallback=false"]
if not all(line in log for line in required):
    raise RuntimeError("native evidence incomplete")
if "Missing ascii bindings" in log:
    raise RuntimeError("synthetic fixture was incompletely deployed")

canonical_log = canonical_probe_log(
    log.replace(str(shared), "$SHARED_DATA").replace(str(run), "$PROBE_RUN")
)
canonical_signature = re.sub(
    r"^Executable=.*$",
    "Executable=static-rime-probe",
    signature,
    flags=re.MULTILINE,
)
canonical_dependencies = re.sub(
    r"^.*static-rime-probe:$", "static-rime-probe:", dependencies, count=1, flags=re.MULTILINE
)
evidence_dir = OUTPUT / "evidence"
if MODE == "--publish":
    evidence_dir.mkdir(exist_ok=True)
    (evidence_dir / "probe.log").write_text(canonical_log)

manifest = {
    **binding,
    "schema_version": MANIFEST_SCHEMA_VERSION,
    "architecture": ARCH,
    "minimum_macos": "13.0",
    "library": "lib/libinputia_rime_static.a",
    "library_sha256": sha(LIBRARY),
    "probe_sha256": sha(PROBE),
    "source_lock_sha256": sha(ROOT / "sources.lock.json"),
    "minos_evidence": minimums,
    "signature": canonical_signature,
    "dynamic_dependencies": canonical_dependencies,
    "probe_log": "evidence/probe.log",
    "probe_log_sha256": text_sha(canonical_log),
    "statically_merged_plugins": ["lua", "octagram", "predict"],
    "tested_modules": ["lua", "octagram", "grammar", "predict"],
    "lua_filter_executed": True,
    "synthetic_pinyin_and_double_pinyin_committed": True,
    "external_rime_dylib_fallback": False,
    "host_integration_verified": False,
    "grammar_model_quality_verified": False,
    "license_compliance_conclusion": "未核验；保留组件许可事实与对应源码，未改仓库根许可证",
    "licenses": {path.name: sha(path) for path in sorted((OUTPUT / "licenses").iterdir())},
}
# 运行期间发生替换同样拒绝。verify-only绝不重签发/覆写旧manifest。
verify(ROOT, OUTPUT, binding)
if MODE == "--publish":
    (OUTPUT / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    (OUTPUT / "link-flags.txt").write_text("-Wl,-force_load,lib/libinputia_rime_static.a\n-lc++\n-mmacosx-version-min=13.0\n")
    (OUTPUT / "codesign.txt").write_text(canonical_signature)
    (OUTPUT / "dependencies.txt").write_text(canonical_dependencies)
elif published_metadata.get("schema_version") == MANIFEST_SCHEMA_VERSION:
    changed = manifest_changed_fields(published_metadata, manifest)
    if changed:
        raise RuntimeError(
            "static Rime manifest evidence mismatch: " + ",".join(changed)
        )
    expected_files = {
        "codesign.txt": canonical_signature,
        "dependencies.txt": canonical_dependencies,
        "link-flags.txt": "-Wl,-force_load,lib/libinputia_rime_static.a\n-lc++\n-mmacosx-version-min=13.0\n",
        "evidence/probe.log": canonical_log,
    }
    for relative, expected in expected_files.items():
        path = OUTPUT / relative
        if not path.is_file() or path.read_text() != expected:
            raise RuntimeError("static Rime evidence file mismatch: " + relative)
else:
    # schema 0 只为已生成缓存保留复验入口；重新 publish 后一律写 schema 1。
    legacy_minimums = published_metadata.get("minos_evidence")
    if not isinstance(legacy_minimums, list) or [
        {"versions": value.get("versions"), "objects": value.get("objects")}
        for value in legacy_minimums
    ] != [
        {"versions": value["versions"], "objects": value["objects"]}
        for value in minimums
    ]:
        raise RuntimeError("static Rime legacy minimum-system evidence mismatch")
    legacy_signature = re.sub(
        r"^Executable=.*$",
        "Executable=static-rime-probe",
        str(published_metadata.get("signature", "")),
        flags=re.MULTILINE,
    )
    legacy_dependencies = re.sub(
        r"^.*static-rime-probe:$",
        "static-rime-probe:",
        str(published_metadata.get("dynamic_dependencies", "")),
        count=1,
        flags=re.MULTILINE,
    )
    fixed = {
        "signature": canonical_signature,
        "dynamic_dependencies": canonical_dependencies,
        "statically_merged_plugins": manifest["statically_merged_plugins"],
        "tested_modules": manifest["tested_modules"],
        "lua_filter_executed": True,
        "synthetic_pinyin_and_double_pinyin_committed": True,
        "external_rime_dylib_fallback": False,
        "host_integration_verified": False,
        "grammar_model_quality_verified": False,
        "license_compliance_conclusion": manifest["license_compliance_conclusion"],
        "licenses": manifest["licenses"],
    }
    observed = {
        **{key: published_metadata.get(key) for key in fixed},
        "signature": legacy_signature,
        "dynamic_dependencies": legacy_dependencies,
    }
    changed = sorted(key for key, value in fixed.items() if observed.get(key) != value)
    if changed:
        raise RuntimeError(
            "static Rime legacy manifest evidence mismatch: " + ",".join(changed)
        )
    legacy_log = Path(str(published_metadata.get("probe_log", "")))
    if not legacy_log.is_absolute() or legacy_log.is_symlink():
        raise RuntimeError("static Rime legacy probe log is not a safe file")
    try:
        legacy_log.resolve().relative_to((ROOT / "artifacts").resolve())
    except ValueError as error:
        raise RuntimeError("static Rime legacy probe log escaped artifacts") from error
    if (
        not legacy_log.is_file()
        or canonical_probe_log(legacy_log.read_text()) != canonical_log
    ):
        raise RuntimeError("static Rime legacy probe log evidence mismatch")
print("staticRimeVerification=pass manifest=" + str(OUTPUT / "manifest.json"))
