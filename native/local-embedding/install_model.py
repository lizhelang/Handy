#!/usr/bin/env python3
"""Install/verify the pinned BGE-M3 MLX embedding checkpoint."""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import tempfile
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

REPOSITORY = "TyKaoz/bge-m3-4bit"
REVISION = "21c7550f1154689f10f3eca2dcb4265c612950c4"
HF_ENDPOINT = os.environ.get("INPUTIA_HF_ENDPOINT", "https://huggingface.co").rstrip("/")
BASE_URL = f"{HF_ENDPOINT}/{REPOSITORY}/resolve/{REVISION}"
MODEL_BYTES = 319903668
MODEL_SHA256 = "c2ba80aaf1bbcd20cf286a91b00ec7313ef20fcd1597b9101bbaf114dbedebd"
CHUNK_BYTES = 16 * 1024 * 1024


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def fetch_range(destination: Path) -> None:
    parts = destination.parent / (destination.name + ".parts")
    parts.mkdir(exist_ok=True)
    count = (MODEL_BYTES + CHUNK_BYTES - 1) // CHUNK_BYTES

    def fetch(index: int) -> int:
        start = index * CHUNK_BYTES
        end = min(MODEL_BYTES - 1, start + CHUNK_BYTES - 1)
        expected = end - start + 1
        part = parts / f"{index:04d}.part"
        if not part.exists() or part.stat().st_size != expected:
            request = urllib.request.Request(
                f"{BASE_URL}/model.safetensors",
                headers={"Range": f"bytes={start}-{end}", "User-Agent": "Inputia-embedding/1"},
            )
            with urllib.request.urlopen(request, timeout=120) as response:
                data = response.read()
            if len(data) != expected:
                raise RuntimeError(f"range {index} returned {len(data)} bytes, expected {expected}")
            part.write_bytes(data)
        return index

    with ThreadPoolExecutor(max_workers=8) as pool:
        for future in as_completed([pool.submit(fetch, index) for index in range(count)]):
            print(f"verified range {future.result() + 1}/{count}", flush=True)
    temporary = destination.with_suffix(".tmp")
    with temporary.open("wb") as output:
        for index in range(count):
            output.write((parts / f"{index:04d}.part").read_bytes())
    os.replace(temporary, destination)
    if destination.stat().st_size != MODEL_BYTES or digest(destination) != MODEL_SHA256:
        raise RuntimeError("BGE-M3 model SHA-256 verification failed")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify", type=Path)
    args = parser.parse_args()
    if bool(args.output) == bool(args.verify):
        parser.error("choose exactly one of --output or --verify")
    if args.verify:
        model = args.verify.expanduser().resolve() / "model.safetensors"
        if not model.is_file() or model.stat().st_size != MODEL_BYTES or digest(model) != MODEL_SHA256:
            raise SystemExit("BGE-M3 verification failed")
        print(f"verified={args.verify.expanduser().resolve()}")
        return 0
    destination = args.output.expanduser().resolve()
    if destination.exists():
        raise SystemExit("destination exists; choose a fresh directory")
    staging = Path(tempfile.mkdtemp(prefix="bge-m3-", dir=destination.parent))
    try:
        fetch_range(staging / "model.safetensors")
        for name in ["config.json", "model.safetensors.index.json", "tokenizer.json", "tokenizer_config.json", "README.md"]:
            request = urllib.request.Request(f"{BASE_URL}/{name}", headers={"User-Agent": "Inputia-embedding/1"})
            with urllib.request.urlopen(request, timeout=60) as response:
                (staging / name).write_bytes(response.read())
        os.replace(staging, destination)
        print(f"installed={destination}")
        return 0
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


if __name__ == "__main__":
    raise SystemExit(main())
