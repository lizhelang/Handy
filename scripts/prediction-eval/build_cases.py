#!/usr/bin/env python3
"""把测试句转换为联想回放用例；只在两侧都是汉字的词边界出题，只打印计数。"""
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


def assign_sets(items, prefix, dev_count):
    ranked = sorted(items, key=lambda item: hashlib.sha256(f"{SPLIT_SALT}:{item['id']}".encode()).hexdigest())
    dev = {item["id"] for item in ranked[:dev_count]}
    for item in items:
        item["set"] = f"{prefix}-{'dev' if item['id'] in dev else 'test'}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--items", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--set-prefix", help="条目未带 set 时按固定哈希切分开发集与测试集")
    parser.add_argument("--dev", type=int, default=0)
    args = parser.parse_args()
    jieba.setLogLevel(60)
    items = [json.loads(line) for line in args.items.read_text().splitlines() if line.strip()]
    if args.set_prefix:
        assign_sets(items, args.set_prefix, args.dev)
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
