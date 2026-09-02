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

swift_settings = root / "Sources/InputiaInputMethod/InputiaSettingsWindow.swift"
rust_settings = repo_root / "crates/inputia-settings/src/lib.rs"


def fail(reason: str) -> None:
    print(f"settingsContractSelfCheck=false reason={reason}")
    raise SystemExit(1)


def read(path: Path) -> str:
    if not path.exists():
        fail(f"missing-file:{path}")
    return path.read_text(encoding="utf-8")


def require(condition: bool, reason: str) -> None:
    if not condition:
        fail(reason)


def extract_swift_string(text: str, label: str) -> str:
    match = re.search(rf'{label}:\s*"([^"]+)"', text)
    if not match:
        fail(f"missing-swift-field:{label}")
    return match.group(1)


def extract_swift_bool(text: str, label: str) -> bool:
    match = re.search(rf'{label}:\s*(true|false)', text)
    if not match:
        fail(f"missing-swift-bool:{label}")
    return match.group(1) == "true"


def extract_rust_string(text: str, field: str) -> str:
    match = re.search(rf'{field}:\s*"([^"]+)"\.to_string\(\)', text)
    if not match:
        fail(f"missing-rust-field:{field}")
    return match.group(1)


def extract_rust_bool(text: str, field: str) -> bool:
    match = re.search(rf'{field}:\s*(true|false)', text)
    if not match:
        fail(f"missing-rust-bool:{field}")
    return match.group(1) == "true"


def extract_sensitive_bundle_ids(text: str) -> list[str]:
    match = re.search(r'default_sensitive_bundle_ids\(\)\s*->\s*Vec<String>\s*\{(?P<body>.*?)\n\}', text, re.S)
    if not match:
        fail("missing-rust-sensitive-bundle-list")
    return re.findall(r'"([^"]+)"\.to_string\(\)', match.group("body"))


swift_text = read(swift_settings)
rust_text = read(rust_settings)

swift_sensitive_match = re.search(r'let inputiaDefaultSensitiveBundleIds = \[(?P<body>.*?)\n\]', swift_text, re.S)
if not swift_sensitive_match:
    fail("missing-swift-sensitive-bundle-list")
swift_sensitive_bundle_ids = re.findall(r'"([^"]+)"', swift_sensitive_match.group("body"))
rust_sensitive_bundle_ids = extract_sensitive_bundle_ids(rust_text)

swift_schema = extract_swift_string(swift_text, "schemaId")
rust_schema = extract_rust_string(rust_text, "schema_id")

rust_candidate_match = re.search(r'candidate_page_size:\s*(\d+)', rust_text)
swift_candidate_match = re.search(r'let inputiaSettingsDefaultCandidatePageSize = (\d+)', swift_text)
swift_min_match = re.search(r'let inputiaSettingsMinCandidatePageSize = (\d+)', swift_text)
swift_max_match = re.search(r'let inputiaSettingsMaxCandidatePageSize = (\d+)', swift_text)
rust_clamp_match = re.search(r'candidate_page_size = self\.candidate_page_size\.clamp\((\d+),\s*(\d+)\)', rust_text)
for label, match in [
    ("rust-candidate-page-size", rust_candidate_match),
    ("swift-candidate-page-size", swift_candidate_match),
    ("swift-candidate-min", swift_min_match),
    ("swift-candidate-max", swift_max_match),
    ("rust-candidate-clamp", rust_clamp_match),
]:
    require(match is not None, f"missing-{label}")

checks = [
    ("settingsContractSchemaDefaultMatches", swift_schema == rust_schema == "luna_pinyin_simp"),
    (
        "settingsContractCandidateDefaultMatches",
        int(swift_candidate_match.group(1)) == int(rust_candidate_match.group(1)) == 7,
    ),
    (
        "settingsContractCandidateRangeMatches",
        (
            int(swift_min_match.group(1)),
            int(swift_max_match.group(1)),
        )
        == (
            int(rust_clamp_match.group(1)),
            int(rust_clamp_match.group(2)),
        )
        == (1, 9),
    ),
    (
        "settingsContractShiftShortcutMatches",
        extract_swift_string(swift_text, "inputModeToggleShortcut") == "shift"
        and "input_mode_toggle_shortcut: InputModeToggleShortcut::Shift" in rust_text,
    ),
    (
        "settingsContractScriptShortcutMatches",
        extract_swift_string(swift_text, "scriptToggleShortcut") == "control_shift_s"
        and "script_toggle_shortcut: ScriptToggleShortcut::ControlShiftS" in rust_text,
    ),
    (
        "settingsContractChineseScriptMatches",
        extract_swift_string(swift_text, "chineseScript") == "simplified"
        and "chinese_script: ChineseScript::Simplified" in rust_text,
    ),
    (
        "settingsContractPunctuationMatches",
        extract_swift_string(swift_text, "punctuationPreference") == "english_in_chinese"
        and "punctuation_preference: PunctuationPreference::EnglishInChinese" in rust_text,
    ),
    (
        "settingsContractCharacterWidthMatches",
        extract_swift_string(swift_text, "characterWidthPreference") == "half_width"
        and "character_width_preference: CharacterWidthPreference::HalfWidth" in rust_text,
    ),
    (
        "settingsContractSpellingCorrectionMatches",
        extract_swift_bool(swift_text, "spellingCorrectionEnabled")
        == extract_rust_bool(rust_text, "spelling_correction_enabled")
        == True,
    ),
    (
        "settingsContractMemoryDefaultMatches",
        extract_swift_bool(swift_text, "memoryEnabled")
        == extract_rust_bool(rust_text, "memory_enabled")
        == True,
    ),
    (
        "settingsContractPrivacyDefaultMatches",
        extract_swift_bool(swift_text, "privacyLearningEnabled")
        == extract_rust_bool(rust_text, "privacy_learning_enabled")
        == True,
    ),
    (
        "settingsContractSensitiveBundleIdsMatch",
        swift_sensitive_bundle_ids == rust_sensitive_bundle_ids,
    ),
    (
        "settingsContractPathsMatch",
        'appendingPathComponent("rime", isDirectory: true)' in swift_text
        and 'base_dir.join("rime")' in rust_text
        and 'appendingPathComponent("inputia_memory.db").path' in swift_text
        and 'base_dir.join("inputia_memory.db")' in rust_text,
    ),
    (
        "settingsContractLegacyShortcutMigrationMatches",
        'inputModeToggleShortcut = shiftToggleEnabled ? "shift" : "none"' in swift_text
        and 'input_mode_toggle_shortcut = InputModeToggleShortcut::None' in rust_text,
    ),
]

ok = all(passed for _, passed in checks)
print(f"settingsContractSelfCheck={str(ok).lower()}")
print(f"settingsContractSchema={swift_schema}")
print(f"settingsContractCandidateDefault={swift_candidate_match.group(1)}")
print(f"settingsContractCandidateRange={swift_min_match.group(1)}..{swift_max_match.group(1)}")
print(f"settingsContractSensitiveBundleCount={len(swift_sensitive_bundle_ids)}")
for name, passed in checks:
    print(f"{name}={str(passed).lower()}")
raise SystemExit(0 if ok else 1)
PY
