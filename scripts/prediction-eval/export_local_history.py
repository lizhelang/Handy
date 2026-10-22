#!/usr/bin/env python3
"""T3：把本机语音与键入历史按时间切分为本机私有评测条目，只打印计数。

所有快照与条目只写入 --out 私有目录（0700），评测结束后整体删除。学习库副本只保留
切分时刻之前的证据，避免测试正文泄漏进个人层。原库只以只读方式打开，不写回。
"""
import argparse
import datetime
import hashlib
import json
import os
import sqlite3
from pathlib import Path


def snapshot(source, target):
    src = sqlite3.connect(f"file:{source}?mode=ro", uri=True)
    dst = sqlite3.connect(target)
    with dst:
        src.backup(dst)
    src.close()
    return dst


def opaque(kind, row_id):
    return hashlib.sha256(f"t3:{kind}:{row_id}".encode()).hexdigest()[:16]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--train-fraction", type=float, default=0.8)
    args = parser.parse_args()
    args.out.mkdir(mode=0o700, parents=True, exist_ok=False)
    snapshots = args.out / "snapshots"
    snapshots.mkdir(mode=0o700)
    items = []
    voice = snapshot(args.profile / "history.db", snapshots / "history.db")
    for row_id, text, seconds, app in voice.execute(
            "SELECT id,COALESCE(NULLIF(post_processed_text,''),transcription_text),timestamp,inputia_source_app "
            "FROM transcription_history"):
        items.append(("voice", opaque("voice", row_id), (text or "").strip(), float(seconds), app or ""))
    voice.close()
    typed = snapshot(args.profile / "knowledge/typed.sqlite", snapshots / "typed.sqlite")
    for row_id, text, millis, app in typed.execute("SELECT id,text,created_at_ms,source_app FROM typed_segments"):
        items.append(("typed", opaque("typed", row_id), (text or "").strip(), millis / 1000.0, app or ""))
    typed.close()
    items = [item for item in items if len(item[2]) >= 4]
    items.sort(key=lambda item: item[3])
    # 两类来源各自按时间取最后一段作测试；个人证据只来自键入上屏，按键入切分点截断。
    splits = {}
    for kind in ("voice", "typed"):
        times = [item[3] for item in items if item[0] == kind]
        splits[kind] = times[int(len(times) * args.train_fraction)]
    split = splits["typed"]
    test = [item for item in items if item[3] >= splits[item[0]]]
    with (args.out / "t3_items.jsonl").open("w") as out:
        for kind, item_id, text, _, app in test:
            out.write(json.dumps({"id": item_id, "set": f"t3-{kind}", "text": text, "app": app},
                                 ensure_ascii=False) + "\n")

    knowledge = args.out / "root/knowledge"
    knowledge.mkdir(mode=0o700, parents=True)
    learned = snapshot(args.profile / "knowledge/personalization.sqlite", knowledge / "personalization.sqlite")
    # 遗忘屏障只存原库密钥摘要；没有遗忘记录时可用新密钥，避免复制真实密钥。
    if learned.execute("SELECT COUNT(*) FROM forgotten").fetchone()[0] != 0:
        raise SystemExit("学习库存在遗忘记录，副本无法在不复制密钥的情况下保持遗忘语义，已中止")
    kept, removed = (learned.execute(f"SELECT COUNT(*) FROM evidence WHERE created {op} ?", (split,)).fetchone()[0]
                     for op in ("<", ">="))
    with learned:
        learned.execute("DELETE FROM evidence WHERE created >= ?", (split,))
        learned.execute("DELETE FROM evidence_anchors WHERE event_id NOT IN (SELECT event_id FROM evidence)")
    learned.close()
    key = knowledge / "personalization-privacy.key"
    descriptor = os.open(key, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.write(descriptor, os.urandom(32))
    os.close(descriptor)

    counts = {}
    for kind, *_ in test:
        counts[kind] = counts.get(kind, 0) + 1
    print(f"items_total={len(items)} test_items={len(test)} by_kind={json.dumps(counts)}")
    for kind, moment in splits.items():
        print(f"{kind}_split_utc={datetime.datetime.fromtimestamp(moment, datetime.UTC):%Y-%m-%d %H:%M}")
    print(f"personal_evidence_kept={kept} personal_evidence_removed={removed}")


if __name__ == "__main__":
    main()
