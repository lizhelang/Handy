#!/usr/bin/env python3
"""核验 Mach-O、签名和真实合成 Rime 会话；不启动输入法，不接触系统输入/剪贴板。"""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from artifact_binding import sha, verify

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


metadata_path = OUTPUT / ("manifest.json" if MODE == "--verify-only" else "probe-link.json")
binding = verify(ROOT, OUTPUT, json.loads(metadata_path.read_text()))
minimums = []
for artifact in [LIBRARY, PROBE]:
    listing = command(["/usr/bin/otool", "-l", str(artifact)])
    versions = [line.split()[1] for line in listing.splitlines() if line.split()[:1] == ["minos"]]
    if not versions or any(tuple(map(int, version.split("."))) > (13, 0, 0) for version in versions):
        raise RuntimeError("unexpected minimum macOS in " + str(artifact))
    minimums.append({"path": str(artifact), "versions": sorted(set(versions)), "objects": len(versions)})
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

manifest = {
    **binding,
    "architecture": ARCH,
    "minimum_macos": "13.0",
    "library": str(LIBRARY),
    "library_sha256": sha(LIBRARY),
    "probe_sha256": sha(PROBE),
    "source_lock_sha256": sha(ROOT / "sources.lock.json"),
    "minos_evidence": minimums,
    "signature": signature,
    "dynamic_dependencies": dependencies,
    "probe_log": str(run / "probe.log"),
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
    (OUTPUT / "link-flags.txt").write_text("-Wl,-force_load," + str(LIBRARY) + "\n-lc++\n-mmacosx-version-min=13.0\n")
    (OUTPUT / "codesign.txt").write_text(signature)
    (OUTPUT / "dependencies.txt").write_text(dependencies)
print("staticRimeVerification=pass manifest=" + str(OUTPUT / "manifest.json"))
