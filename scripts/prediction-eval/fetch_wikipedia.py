#!/usr/bin/env python3
"""从中文维基随机条目的导语抽取简体句子作为 T2 测试集；正文只写入缓存目录，不入库。

维基正文按 CC BY-SA 4.0 授权。入库的只有页面与修订 ID 清单及文件摘要。每次请求批量
取 20 个条目导语，串行低频访问，遇限流按提示退避。
"""
import argparse
import hashlib
import json
import random
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

API = "https://zh.wikipedia.org/w/api.php"
USER_AGENT = "InputiaPredictionEval/0.1 (offline input-method evaluation; low-rate script)"
SENTENCE_END = re.compile(r"[。！？\n]")
PER_PAGE = 4
BATCH = 20


def call(params):
    query = urllib.parse.urlencode({**params, "format": "json", "formatversion": "2", "maxlag": "5"})
    request = urllib.request.Request(f"{API}?{query}", headers={"User-Agent": USER_AGENT})
    for attempt in range(6):
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            if error.code not in (429, 503) or attempt == 5:
                raise
            # 服从限流提示，串行退避，不并发重试。
            time.sleep(max(int(error.headers.get("Retry-After") or 0), 5 * 2 ** attempt))


def cjk_ratio(text):
    return sum(0x3400 <= ord(c) <= 0x9FFF for c in text) / max(1, len(text))


def sentences(extract):
    result = []
    for raw in SENTENCE_END.split(extract):
        text = raw.strip()
        if 10 <= len(text) <= 60 and cjk_ratio(text) >= 0.8 and not re.search(r"[=|{}<>\[\]]", text):
            result.append(text + "。")
    return result


def batch(params):
    reply = call({"prop": "extracts|revisions", "exintro": "1", "explaintext": "1", "exlimit": str(BATCH),
                  "rvprop": "ids", "variant": "zh-cn", "action": "query", **params})
    pages = reply.get("query", {}).get("pages", [])
    time.sleep(2.0)
    return [page for page in pages if not page.get("missing") and page.get("revisions")]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--sources", type=Path, required=True, help="页面与修订 ID 清单（复现时作为输入）")
    parser.add_argument("--cache", type=Path, required=True, help="原始导语缓存，逐批追加，可断点续抓")
    parser.add_argument("--pages", type=int, default=2600)
    parser.add_argument("--count", type=int, default=2500)
    parser.add_argument("--dev", type=int, default=500)
    parser.add_argument("--seed", type=int, default=20261022)
    args = parser.parse_args()
    fetched = []
    if args.sources.exists():
        ids = [page["page_id"] for page in json.loads(args.sources.read_text())["pages"]]
        for start in range(0, len(ids), BATCH):
            fetched += batch({"pageids": "|".join(map(str, ids[start:start + BATCH]))})
    else:
        if args.cache.exists():
            fetched = [json.loads(line) for line in args.cache.read_text().splitlines() if line.strip()]
        seen = {page["pageid"] for page in fetched}
        with args.cache.open("a") as cache:
            while len(fetched) < args.pages:
                for page in batch({"generator": "random", "grnnamespace": "0", "grnfilterredir": "nonredirects",
                                   "grnlimit": str(BATCH)}):
                    title = page["title"]
                    if page["pageid"] not in seen and "列表" not in title and "消歧义" not in title:
                        seen.add(page["pageid"])
                        fetched.append(page)
                        cache.write(json.dumps(page, ensure_ascii=False) + "\n")
                cache.flush()
        fetched = fetched[:args.pages]
    fetched.sort(key=lambda page: page["pageid"])
    pages, pool = [], []
    for page in fetched:
        revision = page["revisions"][0]["revid"]
        pages.append({"page_id": page["pageid"], "rev_id": revision})
        found = sentences(page.get("extract", ""))
        random.Random(f"{args.seed}:{page['pageid']}").shuffle(found)
        pool.extend({"page_id": page["pageid"], "rev_id": revision, "text": text} for text in found[:PER_PAGE])
    if len(pool) < args.count:
        raise SystemExit(f"可用句子不足：{len(pool)} < {args.count}")
    random.Random(args.seed).shuffle(pool)
    chosen = pool[:args.count]
    with args.out.open("w") as out:
        for index, row in enumerate(chosen):
            item = {"id": f"t2-{index + 1:04d}", "set": "t2-dev" if index < args.dev else "t2-test", **row}
            out.write(json.dumps(item, ensure_ascii=False) + "\n")
    args.sources.write_text(json.dumps({"api": API, "variant": "zh-cn", "section": "intro", "seed": args.seed,
                                        "count": args.count, "dev": args.dev, "per_page": PER_PAGE,
                                        "pages": pages}, ensure_ascii=False, indent=1) + "\n")
    print(f"pages={len(pages)} pool={len(pool)} items={len(chosen)} dev={args.dev}")
    print(f"items_sha256={hashlib.sha256(args.out.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
