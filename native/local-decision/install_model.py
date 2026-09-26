#!/usr/bin/env python3
"""Install the pinned Laya-MLX checkpoint with resumable range downloads.

The script is intentionally explicit: it never chooses a floating model,
never replaces an existing directory in place, and verifies every file before
publishing the new directory. It is a development/packaging helper until the
Tauri model manager exposes the same contract.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import tempfile
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

REPOSITORY = "aac6fef/laya-multilingual-mlx"
REVISION = "052592a15d198d9ad47da779604259b10b47b7aa"
BASE_URL = f"https://huggingface.co/{REPOSITORY}/resolve/{REVISION}"
MODEL_SHA256 = "7fc5834af4d8fdfb268d272a9d1a66e5819a0daac98241651c4c888cc43adff1"
MODEL_BYTES = 643835426
CHUNK_BYTES = 16 * 1024 * 1024


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def fetch(url: str, destination: Path, expected_bytes: int, expected_sha: str) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists() and destination.stat().st_size == expected_bytes:
        if sha256(destination) == expected_sha:
            return
        destination.unlink()
    request = urllib.request.Request(url, headers={"User-Agent": "Inputia-local-decision/1"})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read()
    if len(data) != expected_bytes or hashlib.sha256(data).hexdigest() != expected_sha:
        raise RuntimeError(f"verification failed for {destination.name}")
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    temporary.write_bytes(data)
    os.replace(temporary, destination)


def fetch_range(url: str, destination: Path, total: int) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    parts = destination.parent / (destination.name + ".parts")
    parts.mkdir(exist_ok=True)
    count = (total + CHUNK_BYTES - 1) // CHUNK_BYTES

    def one(index: int) -> int:
        start = index * CHUNK_BYTES
        end = min(total - 1, start + CHUNK_BYTES - 1)
        part = parts / f"{index:04d}.part"
        expected = end - start + 1
        if not part.exists() or part.stat().st_size != expected:
            request = urllib.request.Request(
                url,
                headers={
                    "User-Agent": "Inputia-local-decision/1",
                    "Range": f"bytes={start}-{end}",
                },
            )
            with urllib.request.urlopen(request, timeout=60) as response:
                data = response.read()
            if len(data) != expected:
                raise RuntimeError(f"range {index} returned {len(data)} bytes, expected {expected}")
            part.write_bytes(data)
        return index

    with ThreadPoolExecutor(max_workers=8) as pool:
        for future in as_completed([pool.submit(one, index) for index in range(count)]):
            print(f"verified range {future.result() + 1}/{count}", flush=True)
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    with temporary.open("wb") as output:
        for index in range(count):
            output.write((parts / f"{index:04d}.part").read_bytes())
    os.replace(temporary, destination)
    if destination.stat().st_size != total or sha256(destination) != MODEL_SHA256:
        raise RuntimeError("model.safetensors SHA-256 verification failed")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if bool(args.output) == bool(args.verify):
        parser.error("choose exactly one of --output or --verify")
    if args.verify:
        model = args.verify.expanduser().resolve() / "model.safetensors"
        if not model.is_file() or model.stat().st_size != MODEL_BYTES:
            raise SystemExit("verification failed: model.safetensors is missing or has the wrong size")
        if sha256(model) != MODEL_SHA256:
            raise SystemExit("verification failed: model.safetensors SHA-256 mismatch")
        print(f"verified={args.verify.expanduser().resolve()}")
        return 0
    destination = args.output.expanduser().resolve()
    parent = destination.parent
    parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix="laya-multilingual-", dir=parent))
    try:
        manifest_url = f"{BASE_URL}/manifest.json"
        request = urllib.request.Request(manifest_url, headers={"User-Agent": "Inputia-local-decision/1"})
        with urllib.request.urlopen(request, timeout=30) as response:
            manifest = json.load(response)
        if manifest.get("source_revision") != REVISION or manifest.get("repository") != REPOSITORY:
            raise RuntimeError("model manifest revision/repository mismatch")
        files = manifest.get("files", {})
        for relative, info in files.items():
            target = staging / relative
            url = f"{BASE_URL}/{relative}"
            if relative == "model.safetensors":
                fetch_range(url, target, MODEL_BYTES)
            else:
                fetch(url, target, int(info["bytes"]), str(info["sha256"]))
        marker = staging / "inputia-model-install.json"
        marker.write_text(json.dumps({"repository": REPOSITORY, "revision": REVISION, "sha256": MODEL_SHA256}, indent=2) + "\n")
        if destination.exists():
            raise RuntimeError("destination exists; choose a fresh install directory")
        os.replace(staging, destination)
        print(f"installed={destination}")
        return 0
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


if __name__ == "__main__":
    raise SystemExit(main())
