#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$ROOT_DIR/../.." && pwd)"

/usr/bin/python3 - "$ROOT_DIR" "$REPO_ROOT" <<'PY'
from pathlib import Path
import re
import sys

root = Path(sys.argv[1])
repo_root = Path(sys.argv[2])

settings_window = root / "Sources/InputiaInputMethod/InputiaSettingsWindow.swift"
settings_window_self_check = root / "Tools/InputiaSettingsWindowSelfCheck.swift"
schema_smoke = repo_root / "crates/inputia-rime/tests/schema_smoke.rs"
rime_data_dirs = [
    root / "build/InputiaInputMethod.app/Contents/Resources/RimeData",
    root / "build/RimeData",
]

required_schema_ids = [
    "luna_pinyin_simp",
    "double_pinyin",
    "double_pinyin_flypy",
    "double_pinyin_sogou",
    "guobiao_bispell",
    "double_pinyin_mspy",
    "double_pinyin_abc",
    "double_pinyin_pyjj",
    "double_pinyin_st",
]


def fail(reason: str) -> None:
    print(f"schemaCatalogSelfCheck=false reason={reason}")
    raise SystemExit(1)


def read(path: Path) -> str:
    if not path.exists():
        fail(f"missing-file:{path}")
    return path.read_text(encoding="utf-8")


settings_text = read(settings_window)
self_check_text = read(settings_window_self_check)
schema_smoke_text = read(schema_smoke)

settings_options = re.findall(
    r'InputiaSchemaOption\(title:\s*"([^"]+)",\s*schemaId:\s*"([^"]+)"\)',
    settings_text,
)
if not settings_options:
    fail("settings-window-schema-options-missing")

self_check_expected = re.findall(r'\(\s*"([^"]+)",\s*"([^"]+)"\s*\)', self_check_text)
schema_smoke_ids = sorted(set(re.findall(r'schema:\s*"([^"]+)"', schema_smoke_text)))

settings_schema_ids = [schema_id for _, schema_id in settings_options]
self_check_schema_ids = [schema_id for _, schema_id in self_check_expected]

if settings_schema_ids != required_schema_ids:
    fail("settings-window-schema-list-mismatch")
if self_check_schema_ids != required_schema_ids:
    fail("settings-window-self-check-schema-list-mismatch")
if len(set(settings_schema_ids)) != len(settings_schema_ids):
    fail("duplicate-settings-schema-id")
if len({title for title, _ in settings_options}) != len(settings_options):
    fail("duplicate-settings-schema-title")

missing_from_smoke = [schema_id for schema_id in settings_schema_ids if schema_id not in schema_smoke_ids]
if missing_from_smoke:
    fail("schema-smoke-missing:" + ",".join(missing_from_smoke))

rime_data_dir = next((path for path in rime_data_dirs if path.exists()), None)
if rime_data_dir is None:
    fail("missing-built-rime-data")

missing_schema_files = [
    schema_id
    for schema_id in settings_schema_ids
    if not (rime_data_dir / f"{schema_id}.schema.yaml").exists()
]
if missing_schema_files:
    fail("rime-data-missing:" + ",".join(missing_schema_files))

print("schemaCatalogSelfCheck=true")
print(f"schemaCatalogSettingsCount={len(settings_schema_ids)}")
print(f"schemaCatalogSelfCheckCount={len(self_check_schema_ids)}")
print(f"schemaCatalogSmokeCoveredCount={len([schema_id for schema_id in settings_schema_ids if schema_id in schema_smoke_ids])}")
print(f"schemaCatalogRimeDataDir={rime_data_dir}")
for schema_id in settings_schema_ids:
    print(f"schemaCatalogSchema.{schema_id}=present")
PY
