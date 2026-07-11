#!/usr/bin/env python3
"""Generate Inputia's first-stage Rime extension dictionaries.

The generator only consumes source snapshots with MIT licenses.  It is kept
deterministic so the generated dictionaries can be reviewed and committed.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import time
import unicodedata
import urllib.parse
import urllib.request
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = ROOT.parents[1]
VERSION = "2026.07.09"

SOURCES = {
    "thuocl": {
        "repo": "thunlp/THUOCL",
        "revision": "a30ce79d895d01ab5132a5c74c29703ff7efb4cc",
        "license": "MIT",
    },
    "chinese_poetry": {
        "repo": "chinese-poetry/chinese-poetry",
        "revision": "b8594f81a89752241442f2ce267d6f66f96704ee",
        "license": "MIT",
    },
    "chinese_xinhua": {
        "repo": "pwxcoo/chinese-xinhua",
        "revision": "fe6d6c2e8baa82187f4c96bbe042e43f96c05666",
        "license": "MIT",
    },
}

THUOCL_CHENGYU = ("thuocl_chengyu.txt", "thuocl", "data/THUOCL_chengyu.txt")
THUOCL_POEM = ("thuocl_poem.txt", "thuocl", "data/THUOCL_poem.txt")
XINHUA_IDIOM = ("xinhua_idiom.json", "chinese_xinhua", "data/idiom.json")
XINHUA_WORD = ("xinhua_word.json", "chinese_xinhua", "data/word.json")
POETRY_FILES = [
    ("poetry_tang_0.json", "chinese_poetry", "全唐诗/poet.tang.0.json"),
    ("poetry_tang_1000.json", "chinese_poetry", "全唐诗/poet.tang.1000.json"),
    ("poetry_song_0.json", "chinese_poetry", "宋词/ci.song.0.json"),
]

PINYIN_OVERRIDES = {
    "长风破浪": "chang feng po lang",
    "会有时": "hui you shi",
    "长风破浪会有时": "chang feng po lang hui you shi",
    "直挂云帆济沧海": "zhi gua yun fan ji cang hai",
    "沉舟侧畔": "chen zhou ce pan",
    "千帆过": "qian fan guo",
    "沉舟侧畔千帆过": "chen zhou ce pan qian fan guo",
    "病树前头万木春": "bing shu qian tou wan mu chun",
    "烂柯人": "lan ke ren",
    "到乡翻似烂柯人": "dao xiang fan si lan ke ren",
    "巴山楚水凄凉地": "ba shan chu shui qi liang di",
    "怀旧空吟闻笛赋": "huai jiu kong yin wen di fu",
    "因地制宜": "yin di zhi yi",
    "坚定不移": "jian ding bu yi",
    "全力以赴": "quan li yi fu",
    "龘": "da",
    "䶮": "yan",
    "罍": "lei",
    "𰻞": "biang",
}

MANUAL_IDIOMS = [
    ("因地制宜", 58_000),
    ("坚定不移", 54_000),
    ("全力以赴", 52_000),
]

MANUAL_POETRY = [
    ("长风破浪会有时", 62_000),
    ("长风破浪", 58_000),
    ("会有时", 55_000),
    ("直挂云帆济沧海", 50_000),
    ("沉舟侧畔千帆过", 50_000),
    ("千帆过", 46_000),
    ("病树前头万木春", 44_000),
]

MANUAL_CLASSICAL = [
    ("烂柯人", 42_000),
    ("到乡翻似烂柯人", 36_000),
    ("巴山楚水凄凉地", 32_000),
    ("怀旧空吟闻笛赋", 32_000),
]

MANUAL_EXT_CHARS = [
    ("𰻞", 28_000),
    ("龘", 1_600),
    ("䶮", 1_400),
    ("罍", 1_200),
]

PUNCTUATION_RE = re.compile(r"[，。！？、；：,.!?;:（）()《》〈〉“”\"'‘’\[\]【】{}<>·—…\s]+")
PINYIN_SPLIT_RE = re.compile(r"[\s,;/|]+")
VALID_PINYIN_RE = re.compile(r"^[a-zv]+$")
TONE_TRANSLATION = str.maketrans(
    {
        "ā": "a",
        "á": "a",
        "ǎ": "a",
        "à": "a",
        "ē": "e",
        "é": "e",
        "ě": "e",
        "è": "e",
        "ī": "i",
        "í": "i",
        "ǐ": "i",
        "ì": "i",
        "ō": "o",
        "ó": "o",
        "ǒ": "o",
        "ò": "o",
        "ū": "u",
        "ú": "u",
        "ǔ": "u",
        "ù": "u",
        "ǖ": "v",
        "ǘ": "v",
        "ǚ": "v",
        "ǜ": "v",
        "ü": "v",
        "ń": "n",
        "ň": "n",
        "ǹ": "n",
        "ḿ": "m",
    }
)


@dataclass(frozen=True)
class Entry:
    text: str
    pinyin: str
    weight: int


def raw_url(source_key: str, path: str) -> str:
    source = SOURCES[source_key]
    encoded = urllib.parse.quote(path)
    return f"https://raw.githubusercontent.com/{source['repo']}/{source['revision']}/{encoded}"


def fetch_text(cache_dir: Path, cache_name: str, source_key: str, path: str) -> str:
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_path = cache_dir / cache_name
    if cache_path.exists():
        return cache_path.read_text(encoding="utf-8")

    url = raw_url(source_key, path)
    last_error: Exception | None = None
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=45) as response:
                data = response.read()
            text = data.decode("utf-8")
            cache_path.write_text(text, encoding="utf-8")
            return text
        except Exception as error:  # noqa: BLE001 - report the original fetch failure.
            last_error = error
            time.sleep(1 + attempt)
    raise RuntimeError(f"failed to fetch {url}: {last_error}")


def is_cjk(char: str) -> bool:
    code = ord(char)
    return (
        0x3400 <= code <= 0x9FFF
        or 0xF900 <= code <= 0xFAFF
        or 0x20000 <= code <= 0x2EBEF
        or 0x30000 <= code <= 0x3134F
    )


def clean_text(value: str) -> str:
    normalized = unicodedata.normalize("NFKC", value.strip())
    return PUNCTUATION_RE.sub("", normalized)


def split_poetry_line(value: str) -> list[str]:
    normalized = unicodedata.normalize("NFKC", value.strip())
    return [clean_text(part) for part in PUNCTUATION_RE.split(normalized) if clean_text(part)]


def normalize_pinyin(value: str) -> str | None:
    normalized = unicodedata.normalize("NFKC", value).lower().translate(TONE_TRANSLATION)
    normalized = normalized.replace("u:", "v").replace("ü", "v")
    syllables: list[str] = []
    for raw_token in PINYIN_SPLIT_RE.split(normalized):
        token = re.sub(r"[^a-zv]", "", raw_token)
        if not token:
            continue
        token = re.sub(r"[1-5]$", "", token)
        if not VALID_PINYIN_RE.match(token):
            return None
        syllables.append(token)
    return " ".join(syllables) if syllables else None


def parse_base_char_pinyin(base_dict: Path) -> dict[str, str]:
    variants: dict[str, set[str]] = defaultdict(set)
    in_entries = False
    for raw_line in base_dict.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line == "...":
            in_entries = True
            continue
        if not in_entries or not line or line.startswith("#"):
            continue
        parts = raw_line.split("\t")
        if len(parts) < 2:
            continue
        text = parts[0].strip()
        if len(text) != 1 or not is_cjk(text):
            continue
        pinyin = normalize_pinyin(parts[1])
        if pinyin and " " not in pinyin:
            variants[text].add(pinyin)
    return {char: next(iter(values)) for char, values in variants.items() if len(values) == 1}


def phrase_pinyin(text: str, char_pinyin: dict[str, str], explicit: str | None = None) -> str | None:
    if explicit:
        return normalize_pinyin(explicit)
    if text in PINYIN_OVERRIDES:
        return normalize_pinyin(PINYIN_OVERRIDES[text])
    if not text or not all(is_cjk(char) for char in text):
        return None
    syllables: list[str] = []
    for char in text:
        syllable = char_pinyin.get(char)
        if not syllable:
            return None
        syllables.append(syllable)
    return " ".join(syllables)


def add_entry(bucket: dict[str, Entry], text: str, pinyin: str | None, weight: int) -> None:
    text = clean_text(text)
    if not text or "\t" in text:
        return
    if not pinyin:
        return
    pinyin = normalize_pinyin(pinyin)
    if not pinyin:
        return
    existing = bucket.get(text)
    if existing is None or weight > existing.weight:
        bucket[text] = Entry(text=text, pinyin=pinyin, weight=weight)


def add_with_generated_pinyin(
    bucket: dict[str, Entry],
    text: str,
    char_pinyin: dict[str, str],
    weight: int,
    explicit: str | None = None,
) -> None:
    cleaned = clean_text(text)
    add_entry(bucket, cleaned, phrase_pinyin(cleaned, char_pinyin, explicit), weight)


def parse_thuocl_lines(text: str, limit: int) -> list[tuple[str, int]]:
    rows: list[tuple[str, int]] = []
    for line in text.splitlines():
        parts = line.strip().split()
        if not parts:
            continue
        word = clean_text(parts[0])
        try:
            freq = int(parts[1]) if len(parts) > 1 else 1
        except ValueError:
            freq = 1
        if word:
            rows.append((word, freq))
        if len(rows) >= limit:
            break
    return rows


def load_json(text: str) -> object:
    return json.loads(text.lstrip("\ufeff"))


def add_poetry_layers(bucket: dict[str, Entry], line: str, char_pinyin: dict[str, str], base_weight: int) -> None:
    full = clean_text(line)
    if 2 <= len(full) <= 16:
        add_with_generated_pinyin(bucket, full, char_pinyin, base_weight)
    for segment in split_poetry_line(line):
        if len(segment) < 2:
            continue
        add_with_generated_pinyin(bucket, segment, char_pinyin, base_weight + min(len(segment), 8) * 45)
        max_len = min(8, len(segment))
        for length in range(max_len, 1, -1):
            fragment_weight = max(800, base_weight - 1_000 + length * 80)
            for start in range(0, len(segment) - length + 1):
                fragment = segment[start : start + length]
                add_with_generated_pinyin(bucket, fragment, char_pinyin, fragment_weight)


def generate_idioms(cache_dir: Path, char_pinyin: dict[str, str], limit: int) -> dict[str, Entry]:
    bucket: dict[str, Entry] = {}
    thuocl_text = fetch_text(cache_dir, *THUOCL_CHENGYU)
    for word, freq in parse_thuocl_lines(thuocl_text, limit):
        add_with_generated_pinyin(bucket, word, char_pinyin, 12_000 + min(freq // 90, 12_000))

    idiom_data = load_json(fetch_text(cache_dir, *XINHUA_IDIOM))
    if isinstance(idiom_data, list):
        for index, item in enumerate(idiom_data[:limit]):
            if not isinstance(item, dict):
                continue
            word = str(item.get("word") or "")
            pinyin = str(item.get("pinyin") or "")
            add_entry(bucket, word, pinyin, max(6_000, 18_000 - min(index // 8, 12_000)))

    for word, weight in MANUAL_IDIOMS:
        add_with_generated_pinyin(bucket, word, char_pinyin, weight)
    return bucket


def generate_poetry(cache_dir: Path, char_pinyin: dict[str, str], thuocl_limit: int) -> dict[str, Entry]:
    bucket: dict[str, Entry] = {}
    thuocl_text = fetch_text(cache_dir, *THUOCL_POEM)
    for word, freq in parse_thuocl_lines(thuocl_text, thuocl_limit):
        add_poetry_layers(bucket, word, char_pinyin, 9_000 + min(freq // 100, 9_000))

    for cache_name, source_key, path in POETRY_FILES:
        data = load_json(fetch_text(cache_dir, cache_name, source_key, path))
        if not isinstance(data, list):
            continue
        for poem in data:
            if not isinstance(poem, dict):
                continue
            for paragraph in poem.get("paragraphs") or []:
                if isinstance(paragraph, str):
                    add_poetry_layers(bucket, paragraph, char_pinyin, 5_000)

    for word, weight in MANUAL_POETRY:
        add_with_generated_pinyin(bucket, word, char_pinyin, weight)
    return bucket


def generate_classical(cache_dir: Path, char_pinyin: dict[str, str], limit: int) -> dict[str, Entry]:
    bucket: dict[str, Entry] = {}
    # chinese-xinhua/ci.json is a large explanatory lexicon and is not required
    # for the first-stage classical fixed-phrase coverage.  Keep this bucket
    # lightweight and deterministic by deriving conservative fixed phrases from
    # the already-used THUOCL poetry snapshot plus explicit multi-pronunciation
    # corrections below.
    thuocl_text = fetch_text(cache_dir, *THUOCL_POEM)
    for index, (word, freq) in enumerate(parse_thuocl_lines(thuocl_text, limit)):
        if 2 <= len(word) <= 8:
            add_with_generated_pinyin(
                bucket,
                word,
                char_pinyin,
                max(2_000, 7_000 + min(freq // 120, 6_000) - min(index // 20, 3_000)),
            )

    for word, weight in MANUAL_CLASSICAL:
        add_with_generated_pinyin(bucket, word, char_pinyin, weight)
    return bucket


def generate_ext_chars(
    cache_dir: Path,
    char_pinyin: dict[str, str],
    limit: int,
    include_xinhua_word: bool,
) -> dict[str, Entry]:
    bucket: dict[str, Entry] = {}
    if include_xinhua_word:
        try:
            word_data = load_json(fetch_text(cache_dir, *XINHUA_WORD))
        except Exception as error:  # noqa: BLE001 - the manual rare-char seed still keeps this dict useful.
            print(f"warning: skip chinese-xinhua word.json: {error}", file=sys.stderr)
            word_data = []
        if isinstance(word_data, list):
            added = 0
            for item in word_data:
                if not isinstance(item, dict):
                    continue
                word = clean_text(str(item.get("word") or ""))
                pinyin = str(item.get("pinyin") or "")
                if len(word) != 1 or not is_cjk(word):
                    continue
                normalized = normalize_pinyin(pinyin)
                if not normalized or " " in normalized:
                    continue
                add_entry(bucket, word, normalized, 900)
                added += 1
                if added >= limit:
                    break

    for word, weight in MANUAL_EXT_CHARS:
        add_with_generated_pinyin(bucket, word, char_pinyin, weight)
    return bucket


def write_dict(path: Path, name: str, entries: dict[str, Entry]) -> None:
    ordered = sorted(entries.values(), key=lambda entry: (-entry.weight, entry.text, entry.pinyin))
    lines = [
        "# Rime dictionary",
        "# encoding: utf-8",
        "#",
        "# Generated by Tools/generate_inputia_lexicons.py from MIT-licensed sources.",
        "---",
        f"name: {name}",
        f'version: "{VERSION}"',
        "sort: by_weight",
        "use_preset_vocabulary: false",
        "...",
    ]
    lines.extend(f"{entry.text}\t{entry.pinyin}\t{entry.weight}" for entry in ordered)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_wrapper(path: Path) -> None:
    lines = [
        "# Rime dictionary",
        "# encoding: utf-8",
        "#",
        "# Inputia wrapper dictionary.  It imports the upstream luna_pinyin table",
        "# and appends Inputia's local extension dictionaries without modifying",
        "# luna_pinyin.dict.yaml.",
        "---",
        "name: inputia_luna_pinyin",
        f'version: "{VERSION}"',
        "sort: by_weight",
        "use_preset_vocabulary: true",
        "import_tables:",
        "  - luna_pinyin",
        "  - inputia_idiom",
        "  - inputia_poetry",
        "  - inputia_classical",
        "  - inputia_ext_chars",
        "...",
    ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--base-dict",
        type=Path,
        default=ROOT / "build/RimeData/luna_pinyin.dict.yaml",
        help="Existing luna_pinyin.dict.yaml used only as a conservative char-pinyin map.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=ROOT / "Resources/RimeData",
        help="Directory for generated Inputia Rime dictionaries.",
    )
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=ROOT / "build/inputia-lexicon-sources",
        help="Source snapshot cache.  Existing files make generation offline/repeatable.",
    )
    parser.add_argument("--idiom-limit", type=int, default=5_000)
    parser.add_argument("--poetry-limit", type=int, default=2_000)
    parser.add_argument("--classical-limit", type=int, default=3_000)
    parser.add_argument("--ext-char-limit", type=int, default=6_000)
    parser.add_argument(
        "--include-xinhua-word",
        action="store_true",
        help="Optionally import chinese-xinhua/data/word.json.  It is large, so the first-stage default uses a curated rare-char seed.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.base_dict.exists():
        print(f"missing base dict: {args.base_dict}", file=sys.stderr)
        return 2

    char_pinyin = parse_base_char_pinyin(args.base_dict)
    args.output_dir.mkdir(parents=True, exist_ok=True)

    dictionaries = {
        "inputia_idiom": generate_idioms(args.cache_dir, char_pinyin, args.idiom_limit),
        "inputia_poetry": generate_poetry(args.cache_dir, char_pinyin, args.poetry_limit),
        "inputia_classical": generate_classical(args.cache_dir, char_pinyin, args.classical_limit),
        "inputia_ext_chars": generate_ext_chars(
            args.cache_dir,
            char_pinyin,
            args.ext_char_limit,
            args.include_xinhua_word,
        ),
    }

    write_wrapper(args.output_dir / "inputia_luna_pinyin.dict.yaml")
    for name, entries in dictionaries.items():
        write_dict(args.output_dir / f"{name}.dict.yaml", name, entries)
        print(f"{name}: {len(entries)} entries")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
