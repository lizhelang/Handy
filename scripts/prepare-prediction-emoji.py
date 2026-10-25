#!/usr/bin/env python3
"""把"词→表情"对照写入上屏后联想表 prediction-ngram.sqlite 的 emoji 表，只在构建时运行。

来源（版本与摘要固定在 scripts/prediction-emoji.lock.json）：
- Inputia 自写的口语词表 scripts/prediction-emoji-curated.tsv，排在最前；
- Unicode CLDR 48.2.3 中文表情注释（Unicode License V3）；
- rime-emoji 的词→表情转换表（LGPL-3.0）；
- Unicode 16.0 emoji-test.txt：只收其中完整限定的表情，并以它把各来源的写法统一成完整限定形式。

CLDR 与 rime-emoji 的词只在表情属于表情、人物手势、动植物、吃喝、天气与出行、活动、衣物与音乐时收录，
不收办公、工具、技术与符号类（例如"文件→📄"、"通过→🈴"）；肤色变体归并到基础表情。
数据来源的"词→表情"只在该词是表情的 CLDR 名称，或 rime-emoji 与 CLDR 关键词一致时收录，
单一来源的泛指关联（例如"中国→🐉"、"工作→👔"）不收。
每个词最多保留 3 个表情，按 自写词表 > rime-emoji > CLDR 名称 > CLDR 关键词（越专一越前）排序。
"""
import argparse
import hashlib
import json
import re
import sqlite3
import urllib.request
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "scripts/prediction-emoji.lock.json"
CURATED = ROOT / "scripts/prediction-emoji-curated.tsv"
DESTINATION = ROOT / "src-tauri/resources/personalization"
PER_KEYWORD = 3
MAX_KEYWORD_CHARS = 6
# 数据来源的词只在这些子分组的表情上收录。
ALLOWED = {
    "Smileys & Emotion": None,
    "Animals & Nature": None,
    "Activities": None,
    "Food & Drink": {"food-fruit", "food-vegetable", "food-prepared", "food-asian", "food-sweet", "drink"},
    "People & Body": {"hand-fingers-open", "hand-fingers-partial", "hand-single-finger", "hand-fingers-closed",
                      "hands", "hand-prop", "body-parts", "person-gesture", "person-activity", "person-sport",
                      "person-resting", "person-fantasy"},
    "Travel & Places": {"sky", "transport-air", "transport-water", "place-geographic", "place-map", "hotel"},
    "Objects": {"clothing", "music", "musical-instrument", "money"},
}


def fetch(name, entry):
    request = urllib.request.Request(entry["url"], headers={"User-Agent": "InputiaEmojiBuild/1"})
    with urllib.request.urlopen(request, timeout=120) as response:
        data = response.read()
    if len(data) != entry["bytes"] or hashlib.sha256(data).hexdigest() != entry["sha256"]:
        raise SystemExit(f"{name} 与固定清单不符：先审查来源再更新固定值")
    return data.decode("utf-8")


def cjk(text):
    return all(0x3400 <= ord(c) <= 0x9FFF or 0xF900 <= ord(c) <= 0xFAFF for c in text)


def bare(text):
    """去掉变体选择符与肤色修饰，用于在各来源之间对齐同一个表情。"""
    return "".join(c for c in text if c != "️" and not 0x1F3FB <= ord(c) <= 0x1F3FF)


def emoji_list(text):
    """emoji-test.txt 中完整限定的表情：基础写法 → (完整限定写法, 分组, 子分组, 次序)。"""
    known, group, subgroup = {}, None, None
    for line in text.splitlines():
        if line.startswith("# group:"):
            group = line.split(":", 1)[1].strip()
        elif line.startswith("# subgroup:"):
            subgroup = line.split(":", 1)[1].strip()
        elif line and not line.startswith("#"):
            codes, rest = line.split(";", 1)
            if rest.split("#", 1)[0].strip() != "fully-qualified":
                continue
            emoji = "".join(chr(int(code, 16)) for code in codes.split())
            if any(0x1F3FB <= ord(c) <= 0x1F3FF for c in emoji):
                continue
            known.setdefault(bare(emoji), (emoji, group, subgroup, len(known)))
    return known


def allowed(entry):
    _, group, subgroup, _ = entry
    return group in ALLOWED and (ALLOWED[group] is None or subgroup in ALLOWED[group])


def collect(curated, cldr, rime, known):
    scores = defaultdict(lambda: defaultdict(float))
    # 每个"词→表情"出现在哪些来源：curated、name（CLDR 名称）、rime、keyword（CLDR 关键词）。
    origins = defaultdict(set)
    for line in curated.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        word, emojis = line.split("\t")
        for rank, emoji in enumerate(emojis.split()):
            if bare(emoji) not in known:
                raise SystemExit(f"自写词表中的 {word} → {emoji} 不在 Unicode 16.0 完整限定清单中")
            scores[word][bare(emoji)] += 10 - rank
            origins[(word, bare(emoji))].add("curated")
    keyword_sets = defaultdict(set)
    names = defaultdict(set)
    for emoji, value in json.loads(cldr)["annotations"]["annotations"].items():
        key = bare(emoji)
        if key not in known or not allowed(known[key]):
            continue
        for word in value.get("default", []):
            keyword_sets[word].add(key)
        for word in value.get("tts", []):
            names[word].add(key)
    for word, emojis in keyword_sets.items():
        for key in emojis:
            scores[word][key] += 1 + 1 / len(emojis)
            origins[(word, key)].add("keyword")
    for word, emojis in names.items():
        for key in emojis:
            scores[word][key] += 2
            origins[(word, key)].add("name")
    for line in rime.splitlines():
        word, _, converted = line.partition("\t")
        ranked = []
        for item in converted.split(" "):
            key = bare(item)
            if item != word and key in known and allowed(known[key]) and key not in ranked:
                ranked.append(key)
        for rank, key in enumerate(ranked):
            scores[word][key] += 3 - 0.2 * rank
            origins[(word, key)].add("rime")

    def eligible(word, key):
        found = origins[(word, key)]
        return bool(found & {"curated", "name"}) or {"rime", "keyword"} <= found

    rows = []
    for word in sorted(scores):
        if not word or len(word) > MAX_KEYWORD_CHARS or not cjk(word):
            continue
        ranked = sorted(((key, score) for key, score in scores[word].items() if eligible(word, key)),
                        key=lambda item: (-item[1], known[item[0]][3]))
        for rank, (key, _) in enumerate(ranked[:PER_KEYWORD]):
            rows.append((word, rank, known[key][0]))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--table", type=Path, default=DESTINATION / "prediction-ngram.sqlite")
    args = parser.parse_args()
    lock = json.loads(LOCK.read_text())
    sources = {name: fetch(name, entry) for name, entry in lock["files"].items()}
    known = emoji_list(sources["emoji-test.txt"])
    rows = collect(CURATED.read_text(encoding="utf-8"), sources["cldr-annotations-zh.json"],
                   sources["rime-emoji-emoji_word.txt"], known)
    db = sqlite3.connect(args.table)
    with db:
        db.executescript("""
            DROP TABLE IF EXISTS emoji;
            CREATE TABLE emoji(keyword TEXT NOT NULL, rank INTEGER NOT NULL, emoji TEXT NOT NULL,
                               PRIMARY KEY(keyword, rank)) WITHOUT ROWID;
        """)
        db.executemany("INSERT INTO emoji VALUES(?,?,?)", rows)
        db.execute("INSERT OR REPLACE INTO meta VALUES('emoji_rows', ?)", (str(len(rows)),))
    db.execute("VACUUM")
    db.close()
    keywords = len({row[0] for row in rows})
    curated_sha = hashlib.sha256(CURATED.read_bytes()).hexdigest()
    provenance_path = args.table.with_name("prediction-ngram.provenance.json")
    provenance = json.loads(provenance_path.read_text())
    provenance["emoji"] = {
        "sources": {name: {"url": entry["url"], "sha256": entry["sha256"], "license": entry["license"]}
                    for name, entry in lock["files"].items()},
        "curated": {"file": "scripts/prediction-emoji-curated.tsv", "sha256": curated_sha,
                    "license": "same as Inputia"},
        "transform": (f"keywords of 1-{MAX_KEYWORD_CHARS} CJK characters; curated words first, then rime-emoji, "
                      "CLDR names and CLDR keywords limited to expressive, nature, food, weather, travel, activity, "
                      f"clothing and music subgroups; skin tones folded; top {PER_KEYWORD} per keyword"),
        "reproduce": "python3 scripts/prepare-prediction-emoji.py",
        "keywords": keywords,
        "rows": len(rows),
    }
    provenance["derived_sha256"] = hashlib.sha256(args.table.read_bytes()).hexdigest()
    provenance["bytes"] = args.table.stat().st_size
    provenance_path.write_text(json.dumps(provenance, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"keywords": keywords, "rows": len(rows), "sha256": provenance["derived_sha256"]}))


if __name__ == "__main__":
    main()
