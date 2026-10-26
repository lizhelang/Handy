#!/usr/bin/env python3
"""从谷歌图书 Ngram 第三版简体中文生成上屏后联想用的二元表，只在构建时运行。

原始文件边下载边计算 SHA256、边解压边统计，不落盘。首次下载把大小与摘要写入
固定清单，之后每次构建都核对；谷歌未公布摘要，因此以首次核对结果为准。
只保留 1980–2019 年计数、不带词性标记的词，且两端都必须是 Rime 公共词表里能打出的词。
另记录每个词后面紧跟标点的次数（语料中的中文标点已统一为半角符号），供句末标点联想使用。
"""
import argparse
import hashlib
import heapq
import json
import multiprocessing
import sqlite3
import subprocess
import sys
import time
import urllib.request
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE = "https://storage.googleapis.com/books/ngrams/books/20200217/chi_sim"
FILES = ["1-00000-of-00001.gz"] + [f"2-{i:05d}-of-00006.gz" for i in range(6)]
LOCK = ROOT / "scripts/prediction-ngram.lock.json"
DESTINATION = ROOT / "src-tauri/resources/personalization"
FIRST_YEAR, LAST_YEAR = 1980, 2019
MIN_UNIGRAM = 40
MIN_BIGRAM = 20
NEXT_K = 12
COMPLETION_K = 8
CHUNK = 1 << 20
FORMAT = "2"
# 语料中的标点记号与联想展示的全角标点。只用于句末判断与问号、感叹号联想，不作为普通续写。
PUNCTUATION = {b",": "，", b".": "。", b"?": "？", b"!": "！", b";": "；", b":": "："}


def vocabulary(lexicon):
    """Rime 公共词表中的词及其出现过的单字，保证联想结果都能用拼音打出。"""
    words = set()
    for line in lexicon.read_text(encoding="utf-8").splitlines():
        word = line.split("\t", 1)[0].strip()
        if word:
            words.add(word)
            words.update(word)
    return {word.encode() for word in words}


def modern_count(fields):
    total = 0
    for field in fields:
        year = int(field[:4])
        if FIRST_YEAR <= year <= LAST_YEAR:
            total += int(field[5:field.index(b",", 5)])
    return total


def stream_lines(url, digest):
    """逐行产出解压内容，同时累计压缩字节的摘要与长度。"""
    request = urllib.request.Request(url, headers={"User-Agent": "InputiaNgramBuild/1"})
    inflate = zlib.decompressobj(31)
    pending = b""
    size = 0
    with urllib.request.urlopen(request, timeout=120) as response:
        while chunk := response.read(CHUNK):
            size += len(chunk)
            digest.update(chunk)
            pending += inflate.decompress(chunk)
            *lines, pending = pending.split(b"\n")
            yield from lines
    pending += inflate.flush()
    if pending:
        yield pending
    yield size


def process(task):
    name, vocab = task
    for attempt in range(3):
        digest = hashlib.sha256()
        unigrams, nexts, marks, size = {}, {}, {}, 0
        try:
            for line in stream_lines(f"{BASE}/{name}", digest):
                if isinstance(line, int):
                    size = line
                    break
                tab = line.find(b"\t")
                gram = line[:tab]
                if tab <= 0 or b"_" in gram:
                    continue
                if name.startswith("1-"):
                    if gram in vocab:
                        count = modern_count(line[tab + 1:].split(b"\t"))
                        if count:
                            unigrams[gram] = unigrams.get(gram, 0) + count
                    continue
                space = gram.find(b" ")
                head, word = gram[:space], gram[space + 1:]
                if space <= 0 or head not in vocab:
                    continue
                if word in PUNCTUATION:
                    count = modern_count(line[tab + 1:].split(b"\t"))
                    if count >= MIN_BIGRAM:
                        key = (head, PUNCTUATION[word])
                        marks[key] = marks.get(key, 0) + count
                    continue
                if word not in vocab:
                    continue
                count = modern_count(line[tab + 1:].split(b"\t"))
                if count < MIN_BIGRAM:
                    continue
                # 每个分片保留前 NEXT_K 即可得到全局精确前 NEXT_K：同一词对只在一个分片出现。
                heap = nexts.setdefault(head, [])
                entry = (count, word)
                if len(heap) < NEXT_K:
                    heapq.heappush(heap, entry)
                elif entry > heap[0]:
                    heapq.heapreplace(heap, entry)
            return name, size, digest.hexdigest(), unigrams, nexts, marks
        except OSError as error:
            if attempt == 2:
                raise
            print(f"{name}: retry after {error}", flush=True)
            time.sleep(10 * (attempt + 1))


def verify(results):
    lock = json.loads(LOCK.read_text()) if LOCK.exists() else {"files": {}}
    for name, size, sha, *_ in results:
        expected = lock["files"].get(name)
        if expected and (expected["bytes"] != size or expected["sha256"] != sha):
            raise SystemExit(f"{name} 与固定清单不符：先审查来源再更新固定值")
        lock["files"][name] = {"url": f"{BASE}/{name}", "bytes": size, "sha256": sha}
    lock.update({"source": "Google Books Ngram Viewer dataset v3 (20200217), chi_sim",
                 "license": "CC BY 3.0"})
    LOCK.write_text(json.dumps(lock, ensure_ascii=False, indent=2) + "\n")


def build(results, output):
    counts = {}
    nexts = {}
    marks = {}
    for name, _, _, unigrams, shard, shard_marks in results:
        for word, count in unigrams.items():
            counts[word.decode()] = counts.get(word.decode(), 0) + count
        for head, heap in shard.items():
            nexts.setdefault(head.decode(), []).extend((count, word.decode()) for count, word in heap)
        for (head, mark), count in shard_marks.items():
            key = (head.decode(), mark)
            marks[key] = marks.get(key, 0) + count
    words = sorted(word for word, count in counts.items() if count >= MIN_UNIGRAM)
    ids = {word: index + 1 for index, word in enumerate(words)}
    extend = dict.fromkeys(words, 0)
    completions = {}
    for word in words:
        for end in range(1, len(word)):
            prefix = word[:end]
            if prefix in ids:
                extend[prefix] += counts[word]
                completions.setdefault(prefix, []).append((-counts[word], word))
    output.unlink(missing_ok=True)
    db = sqlite3.connect(output)
    db.executescript("""
        PRAGMA page_size=4096; PRAGMA journal_mode=DELETE;
        CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE words(id INTEGER PRIMARY KEY, text TEXT NOT NULL UNIQUE, count INTEGER NOT NULL, extend INTEGER NOT NULL);
        CREATE TABLE next(h INTEGER NOT NULL, rank INTEGER NOT NULL, w INTEGER NOT NULL, count INTEGER NOT NULL, PRIMARY KEY(h, rank)) WITHOUT ROWID;
        CREATE TABLE completion(h INTEGER NOT NULL, rank INTEGER NOT NULL, w INTEGER NOT NULL, PRIMARY KEY(h, rank)) WITHOUT ROWID;
        CREATE TABLE punct(h INTEGER NOT NULL, mark TEXT NOT NULL, count INTEGER NOT NULL, PRIMARY KEY(h, mark)) WITHOUT ROWID;
    """)
    with db:
        db.executemany("INSERT INTO words VALUES(?,?,?,?)",
                       ((ids[word], word, counts[word], extend[word]) for word in words))
        next_rows = 0
        for head in sorted(nexts):
            if head not in ids:
                continue
            ranked = sorted(((-count, word) for count, word in nexts[head] if word in ids))[:NEXT_K]
            db.executemany("INSERT INTO next VALUES(?,?,?,?)",
                           ((ids[head], rank, ids[word], -count) for rank, (count, word) in enumerate(ranked)))
            next_rows += len(ranked)
        completion_rows = 0
        for prefix in sorted(completions):
            ranked = sorted(completions[prefix])[:COMPLETION_K]
            db.executemany("INSERT INTO completion VALUES(?,?,?)",
                           ((ids[prefix], rank, ids[word]) for rank, (_, word) in enumerate(ranked)))
            completion_rows += len(ranked)
        punct_rows = [(ids[head], mark, count) for (head, mark), count in sorted(marks.items()) if head in ids]
        db.executemany("INSERT INTO punct VALUES(?,?,?)", punct_rows)
        db.executemany("INSERT INTO meta VALUES(?,?)", [
            ("format", FORMAT), ("first_year", str(FIRST_YEAR)), ("last_year", str(LAST_YEAR)),
            ("min_unigram", str(MIN_UNIGRAM)), ("min_bigram", str(MIN_BIGRAM)),
            ("next_k", str(NEXT_K)), ("completion_k", str(COMPLETION_K))])
    db.execute("VACUUM")
    db.close()
    return {"words": len(words), "next_rows": next_rows, "completion_rows": completion_rows,
            "punct_rows": len(punct_rows)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lexicon", type=Path, default=DESTINATION / "base-lexicon.tsv")
    parser.add_argument("--jobs", type=int, default=len(FILES))
    args = parser.parse_args()
    vocab = vocabulary(args.lexicon)
    started = time.time()
    with multiprocessing.Pool(args.jobs) as pool:
        results = []
        for result in pool.imap_unordered(process, [(name, vocab) for name in FILES]):
            print(f"{result[0]}: bytes={result[1]} sha256={result[2]} elapsed={time.time() - started:.0f}s",
                  flush=True)
            results.append(result)
    results.sort(key=lambda result: result[0])
    verify(results)
    output = DESTINATION / "prediction-ngram.sqlite"
    stats = build(results, output)
    derived = hashlib.sha256(output.read_bytes()).hexdigest()
    (DESTINATION / "prediction-ngram.provenance.json").write_text(json.dumps({
        "source": "Google Books Ngram Viewer dataset v3 (20200217), Chinese (Simplified)",
        "url": "https://storage.googleapis.com/books/ngrams/books/datasetsv3.html",
        "license": "CC BY 3.0",
        "attribution": "Google Books Ngram Viewer, http://books.google.com/ngrams",
        "files": "scripts/prediction-ngram.lock.json",
        "transform": (f"years {FIRST_YEAR}-{LAST_YEAR}; untagged tokens only; both tokens in the Rime public "
                      f"lexicon vocabulary; unigram >= {MIN_UNIGRAM}; bigram >= {MIN_BIGRAM}; top {NEXT_K} next "
                      f"words and top {COMPLETION_K} completions per word; counts of each word followed by "
                      f"the punctuation tokens {' '.join(t.decode() for t in PUNCTUATION)} (bigram >= {MIN_BIGRAM})"),
        "vocabulary": "src-tauri/resources/personalization/base-lexicon.tsv plus its characters",
        "reproduce": "python3 scripts/prepare-prediction-ngram.py",
        "derived_sha256": derived,
        "bytes": output.stat().st_size,
        **stats,
    }, ensure_ascii=False, indent=2) + "\n")
    # 表情表写入同一文件，并更新来源记录中的最终摘要。
    subprocess.run([sys.executable, "-I", str(ROOT / "scripts/prepare-prediction-emoji.py"), "--table", str(output)],
                   check=True)
    print(json.dumps({**stats, "bytes": output.stat().st_size,
                      "sha256": hashlib.sha256(output.read_bytes()).hexdigest()}))


if __name__ == "__main__":
    main()
