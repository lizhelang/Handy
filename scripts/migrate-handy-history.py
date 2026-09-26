#!/usr/bin/env python3
"""把旧 Handy 的历史和附件迁移到 Inputia 候选数据域。

默认只做盘点；传入 --apply 才会创建备份并写入目标数据库。脚本不删除源数据，
删除应用和源数据必须在迁移校验完成后单独执行。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sqlite3
import sys
import time
from pathlib import Path


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def open_db(path: Path, readonly: bool = False) -> sqlite3.Connection:
    if readonly:
        return sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    connection = sqlite3.connect(path)
    connection.execute("PRAGMA busy_timeout=5000")
    connection.execute("PRAGMA foreign_keys=ON")
    return connection


def checkpoint(path: Path) -> None:
    if not path.exists():
        return
    with open_db(path) as connection:
        connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")


def copy_tree(source: Path, destination: Path) -> None:
    if not source.exists():
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, destination, dirs_exist_ok=True, copy_function=shutil.copy2)


def unique_asset(source: Path, destination_dir: Path, preferred: str, namespace: str) -> str | None:
    if not source.is_file():
        return None
    destination_dir.mkdir(parents=True, exist_ok=True)
    safe_name = Path(preferred).name or source.name
    target = destination_dir / safe_name
    if target.exists() and sha256_file(target) != sha256_file(source):
        target = destination_dir / f"migrated-{namespace[:16]}-{safe_name}"
    if not target.exists():
        shutil.copy2(source, target)
    return target.name


def source_counts(root: Path) -> dict[str, int]:
    counts: dict[str, int] = {}
    for name, table in (("history.db", "transcription_history"), ("clipboard.db", "clipboard_history")):
        path = root / name
        if not path.exists():
            counts[table] = 0
            continue
        with open_db(path, readonly=True) as connection:
            counts[table] = int(connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0])
    return counts


def migrate_history(source: Path, target: Path, execute: bool) -> dict[str, int]:
    source_db = source / "history.db"
    target_db = target / "history.db"
    if not source_db.exists():
        return {"seen": 0, "inserted": 0, "skipped": 0, "missing_assets": 0}
    if not execute:
        return {"seen": source_counts(source)["transcription_history"], "inserted": 0, "skipped": 0, "missing_assets": 0}

    target_db.parent.mkdir(parents=True, exist_ok=True)
    with open_db(target_db) as destination, open_db(source_db, readonly=True) as original:
        columns = [row[1] for row in destination.execute("PRAGMA table_info(transcription_history)")]
        if not columns:
            raise RuntimeError(f"目标数据库没有 transcription_history: {target_db}")
        has_prompt = "post_process_prompt" in columns
        has_requested = "post_process_requested" in columns
        has_app = "inputia_source_app" in columns
        has_trust = "inputia_source_trust" in columns
        fingerprints = {
            sha256_bytes("\0".join(map(str, row)).encode())
            for row in destination.execute(
                "SELECT timestamp,title,transcription_text,post_processed_text FROM transcription_history"
            )
        }
        inserted = skipped = missing = 0
        rows = original.execute(
            "SELECT id,file_name,timestamp,saved,title,transcription_text,post_processed_text,"
            "post_process_prompt,post_process_requested FROM transcription_history ORDER BY id"
        )
        for row in rows:
            source_id, file_name, timestamp, saved, title, raw, processed, prompt, requested = row
            fingerprint = sha256_bytes("\0".join(map(str, (timestamp, title, raw, processed))).encode())
            if fingerprint in fingerprints:
                skipped += 1
                continue
            fingerprints.add(fingerprint)
            source_asset = source / "recordings" / (file_name or "")
            asset_name = unique_asset(source_asset, target / "recordings", file_name or "", fingerprint)
            if file_name and asset_name is None:
                missing += 1
            values = [asset_name or file_name or "", timestamp, saved, title or "", raw or "", processed]
            names = ["file_name", "timestamp", "saved", "title", "transcription_text", "post_processed_text"]
            if has_prompt:
                names.append("post_process_prompt"); values.append(prompt)
            if has_requested:
                names.append("post_process_requested"); values.append(requested or 0)
            if has_app:
                names.append("inputia_source_app"); values.append("com.pais.handy")
            if has_trust:
                # 旧 Handy 记录没有 Inputia 的目标字段证明，只能保守标为 unknown。
                names.append("inputia_source_trust"); values.append("unknown")
            placeholders = ",".join("?" for _ in names)
            destination.execute(
                f"INSERT INTO transcription_history ({','.join(names)}) VALUES ({placeholders})", values
            )
            inserted += 1
        destination.commit()
    return {"seen": inserted + skipped, "inserted": inserted, "skipped": skipped, "missing_assets": missing}


def migrate_clipboard(source: Path, target: Path, execute: bool) -> dict[str, int]:
    source_db = source / "clipboard.db"
    target_db = target / "clipboard.db"
    if not source_db.exists():
        return {"seen": 0, "inserted": 0, "skipped": 0, "missing_assets": 0}
    if not execute:
        return {"seen": source_counts(source)["clipboard_history"], "inserted": 0, "skipped": 0, "missing_assets": 0}

    with open_db(target_db) as destination, open_db(source_db, readonly=True) as original:
        hashes = {row[0] for row in destination.execute("SELECT content_hash FROM clipboard_history")}
        inserted = skipped = missing = 0
        rows = original.execute(
            "SELECT content_type,content_preview,content_hash,full_text,image_path,source_app,"
            "is_favorite,is_pinned,created_at,size_bytes,title FROM clipboard_history ORDER BY id"
        )
        for row in rows:
            content_type, preview, content_hash, full_text, image_path, app, favorite, pinned, created, size, title = row
            if content_hash in hashes:
                skipped += 1
                continue
            hashes.add(content_hash)
            asset_name = image_path
            if content_type == "image" and image_path:
                asset_name = unique_asset(source / "clipboard_images" / image_path, target / "clipboard_images", image_path, content_hash)
                if asset_name is None:
                    missing += 1
            destination.execute(
                "INSERT INTO clipboard_history(content_type,content_preview,content_hash,full_text,image_path,source_app,is_favorite,is_pinned,created_at,size_bytes,title) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                (content_type, preview or "", content_hash, full_text, asset_name, app, favorite or 0, pinned or 0, created, size or 0, title),
            )
            inserted += 1
        destination.commit()
    return {"seen": inserted + skipped, "inserted": inserted, "skipped": skipped, "missing_assets": missing}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--backup", type=Path, required=True)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    source = args.source.expanduser().resolve()
    target = args.target.expanduser().resolve()
    backup = args.backup.expanduser().resolve()
    if source == target or not source.is_dir() or not target.is_dir():
        raise SystemExit("source/target must be two existing directories")
    if not args.apply:
        print(json.dumps({"mode": "dry_run", "source": str(source), "target": str(target), "source_counts": source_counts(source)}, ensure_ascii=False))
        return 0
    for name in ("history.db", "clipboard.db"):
        checkpoint(target / name)
    backup.mkdir(parents=True, exist_ok=False)
    copy_tree(source, backup / "handy-source")
    copy_tree(target, backup / "inputia-before")
    summary = {"history": migrate_history(source, target, True), "clipboard": migrate_clipboard(source, target, True)}
    (backup / "migration-summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"mode": "applied", "backup": str(backup), "summary": summary}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"migration failed: {error}", file=sys.stderr)
        raise
