#!/usr/bin/env python3
"""把测试句转换为联想回放用例；只在两侧都是汉字的词边界出题，只打印计数。

`--ends-out` 另把"汉字后紧跟标点"的位置写成句末用例，答案为该标点，供问号与感叹号联想评测；
`--emoji` 把表情用例（正例带可接受表情，反例为空）转换为以该词为前文的用例。
"""
import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path

import jieba

MAX_CONTEXT = 512
MAX_REMAINING = 32
MAX_POSITIONS = 40
SPLIT_SALT = "prediction-eval-split-v1"
# 句末用例的答案统一为全角标点；ASCII 句点只在汉字之后才算句末。
END_MARKS = {"？": "？", "?": "？", "！": "！", "!": "！", "。": "。", ".": "。",
             "，": "，", ",": "，", "；": "；", ";": "；", "：": "：", ":": "："}


def cjk(char):
    code = ord(char)
    return 0x3400 <= code <= 0x9FFF or 0xF900 <= code <= 0xFAFF


def boundaries(text):
    tokens = jieba.lcut(text, HMM=True)
    if "".join(tokens) != text:
        raise SystemExit("分词结果与原文不一致")
    offset, result = 0, []
    for token in tokens[:-1]:
        offset += len(token)
        if cjk(text[offset - 1]) and cjk(text[offset]):
            result.append(offset)
    if len(result) > MAX_POSITIONS:
        # 长文本均匀取点，避免少数长记录主导指标。
        step = len(result) / MAX_POSITIONS
        result = [result[int(i * step)] for i in range(MAX_POSITIONS)]
    return result


def remaining(text, start):
    end = start
    while end < len(text) and end - start < MAX_REMAINING and cjk(text[end]):
        end += 1
    return text[start:end]


def clause_ends(text):
    for index in range(1, len(text)):
        mark = END_MARKS.get(text[index])
        if mark and cjk(text[index - 1]):
            yield index, mark


def assign_sets(items, prefix, dev_count, stratify=None):
    """按固定哈希切分；给定 stratify 时各类按同一比例切分，避免小类整类落入一侧。"""
    groups = {}
    for item in items:
        groups.setdefault(item.get(stratify) if stratify else None, []).append(item)
    dev = set()
    for group in groups.values():
        ranked = sorted(group, key=lambda item: hashlib.sha256(f"{SPLIT_SALT}:{item['id']}".encode()).hexdigest())
        count = dev_count if not stratify else round(len(group) * dev_count / len(items))
        dev.update(item["id"] for item in ranked[:count])
    for item in items:
        item["set"] = f"{prefix}-{'dev' if item['id'] in dev else 'test'}"


def emoji_items(path):
    data = json.loads(path.read_text())
    items = []
    for polarity in ("positive", "negative"):
        for case in data[polarity]:
            items.append({"id": case["id"], "word": case["word"], "accept": case.get("accept", []),
                          "polarity": polarity})
    return items


def write_emoji(items, prefix, out):
    counts = Counter()
    with out.open("w") as handle:
        for item in items:
            name = item["set"].replace(prefix, f"{prefix}-{'pos' if item['polarity'] == 'positive' else 'neg'}", 1)
            handle.write(json.dumps({"set": name, "item": item["id"], "context": item["word"], "remaining": "",
                                     "accept": item["accept"]}, ensure_ascii=False) + "\n")
            counts[name] += 1
    for name in sorted(counts):
        print(f"{name}: items={counts[name]}")
    print(f"cases_sha256={hashlib.sha256(out.read_bytes()).hexdigest()}")


def write_ends(items, path):
    counts = Counter()
    with path.open("w") as out:
        for item in items:
            text = item["text"].strip()
            for index, mark in clause_ends(text):
                case = {"set": f"{item['set']}:end", "item": item["id"],
                        "context": text[max(0, index - MAX_CONTEXT):index], "remaining": "", "mark": mark}
                if item.get("form"):
                    case["group"] = item["form"]
                if item.get("app"):
                    case["app"] = item["app"]
                out.write(json.dumps(case, ensure_ascii=False) + "\n")
                counts[(case["set"], mark)] += 1
    for (name, mark), count in sorted(counts.items()):
        print(f"{name} {mark}: positions={count}")
    print(f"ends_sha256={hashlib.sha256(path.read_bytes()).hexdigest()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--items", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--set-prefix", help="条目未带 set 时按固定哈希切分开发集与测试集")
    parser.add_argument("--dev", type=int, default=0)
    parser.add_argument("--stratify", help="按该字段分层切分，例如 form 或 polarity")
    parser.add_argument("--ends-out", type=Path, help="另写句末标点用例")
    parser.add_argument("--emoji", action="store_true", help="输入为表情用例 JSON")
    args = parser.parse_args()
    if args.emoji:
        items = emoji_items(args.items)
        assign_sets(items, args.set_prefix, args.dev, args.stratify)
        write_emoji(items, args.set_prefix, args.out)
        return
    jieba.setLogLevel(60)
    items = [json.loads(line) for line in args.items.read_text().splitlines() if line.strip()]
    if args.set_prefix:
        assign_sets(items, args.set_prefix, args.dev, args.stratify)
    if args.ends_out:
        write_ends(items, args.ends_out)
    counts, item_counts = Counter(), Counter()
    with args.out.open("w") as out:
        for item in items:
            text = item["text"].strip()
            item_counts[item["set"]] += 1
            for start in boundaries(text):
                case = {
                    "set": item["set"],
                    "item": item["id"],
                    "context": text[max(0, start - MAX_CONTEXT):start],
                    "remaining": remaining(text, start),
                }
                if item.get("app"):
                    case["app"] = item["app"]
                out.write(json.dumps(case, ensure_ascii=False) + "\n")
                counts[item["set"]] += 1
    for name in sorted(item_counts):
        print(f"{name}: items={item_counts[name]} positions={counts[name]}")
    print(f"cases_sha256={hashlib.sha256(args.out.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
