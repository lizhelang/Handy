#!/usr/bin/env python3
"""从当前已固定的公共Rime词频资料生成简体联想底库，不读取个人userdb。"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PINNED_ESSAY = "aee0abc0e4f09c23e9f2f5645791a29f8479d7e4e4be2723f5a9405cb565c885"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rime-data", type=Path, required=True)
    args = parser.parse_args()
    source = args.rime_data / "essay.txt"
    raw = source.read_bytes()
    if hashlib.sha256(raw).hexdigest() != PINNED_ESSAY:
        raise SystemExit("公共词频源与固定Squirrel 1.1.2资料不符；先审查来源再更新固定值")
    runtime = ROOT / "native/static-rime/artifacts/deps"
    converter = runtime / "bin/opencc"
    destination = ROOT / "src-tauri/resources/personalization"
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="inputia-base-lexicon-") as work:
        converted = Path(work) / "simplified.txt"
        subprocess.run([str(converter), "-c", str(runtime / "share/opencc/t2s.json"),
                        "-i", str(source), "-o", str(converted)], check=True)
        entries = {}
        for line in converted.read_text().splitlines():
            fields = line.rsplit("\t", 1)
            if len(fields) != 2 or not fields[1].isdigit():
                continue
            text, weight = fields[0].strip(), int(fields[1])
            if 2 <= len(text) <= 32 and weight > 0 and not any(ord(c) < 32 for c in text):
                entries[text] = min(2**63 - 1, entries.get(text, 0) + weight)
        result = "".join(f"{word}\t{entries[word]}\n" for word in sorted(entries)).encode()
        (destination / "base-lexicon.tsv").write_bytes(result)
    shutil.copyfile(source, destination / "essay.original.txt")
    shutil.copyfile(args.rime_data / "Squirrel-release.LICENSE.txt", destination / "COPYING.GPL-3.0")
    (destination / "provenance.json").write_text(json.dumps({
        "source": "https://github.com/rime/rime-essay", "license": "LGPL-3.0",
        "distribution": "https://github.com/rime/squirrel/releases/tag/1.1.2",
        "original_sha256": PINNED_ESSAY, "derived_sha256": hashlib.sha256(result).hexdigest(),
        "transform": "OpenCC t2s; retain 2..32-character phrases; sum simplified duplicates; lexicographic order",
        "reproduce": "python3 scripts/prepare-personalization-lexicon.py --rime-data <fixed RimeData directory>",
        "entries": len(entries),
    }, ensure_ascii=False, indent=2) + "\n")
    print(f"public_lexicon_entries={len(entries)} bytes={len(result)} user_data_read=false")


if __name__ == "__main__":
    main()
