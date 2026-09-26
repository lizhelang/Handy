#!/usr/bin/env python3
"""BGE-M3 MLX embedding sidecar.

This worker is separate from the typed-decision Laya worker. It emits dense
1024-dimensional vectors only; it never performs retrieval, writes SQLite, or
opens a network listener. Missing or invalid model files fail closed.
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path
from typing import Any

BACKEND = os.environ.get("INPUTIA_EMBEDDING_BACKEND", "laya_encoder")
MODEL_ID = os.environ.get("INPUTIA_EMBEDDING_MODEL", "laya-multilingual-encoder-mean")
MODEL_REVISION = os.environ.get(
    "INPUTIA_EMBEDDING_REVISION",
    "052592a15d198d9ad47da779604259b10b47b7aa",
)
MODEL_PATH = os.environ.get("INPUTIA_EMBEDDING_MODEL_PATH", "")
MAX_LINE_BYTES = 4 * 1024 * 1024
_model: Any = None
_tokenizer: Any = None
_embed_fn: Any = None


def error(request_id: str, code: str) -> dict[str, Any]:
    return {
        "protocol_version": 1,
        "request_id": request_id,
        "model_id": MODEL_ID,
        "model_revision": MODEL_REVISION,
        "error": code,
    }


def load_model() -> tuple[Any, Any]:
    global _model, _tokenizer, _embed_fn
    if _model is None:
        if not MODEL_PATH or not Path(MODEL_PATH).is_dir():
            raise RuntimeError("embedding_model_missing")
        if BACKEND == "laya_encoder":
            import laya_mlx  # type: ignore

            _model = laya_mlx.load(MODEL_PATH, dtype="float16")
            _embed_fn = laya_mlx.embed_fn_from_agent(_model)
            _tokenizer = None
        else:
            from mlx_embeddings import load  # type: ignore

            _model, _tokenizer = load(MODEL_PATH)
    return _model, _tokenizer


def handle(request: dict[str, Any]) -> dict[str, Any]:
    request_id = str(request.get("request_id", ""))
    if request.get("protocol_version") != 1 or not request_id or len(request_id) > 128:
        return error(request_id, "embedding_request_invalid")
    if request.get("kind") == "health":
        ready = bool(MODEL_PATH and Path(MODEL_PATH).is_dir())
        return {
            "protocol_version": 1,
            "request_id": request_id,
            "model_id": MODEL_ID,
            "model_revision": MODEL_REVISION,
            "ready": ready,
        }
    if request.get("kind") != "embed":
        return error(request_id, "embedding_kind")
    texts = request.get("texts")
    if not isinstance(texts, list) or not texts or len(texts) > 32:
        return error(request_id, "embedding_batch")
    if any(not isinstance(text, str) or not text or len(text) > 8192 for text in texts):
        return error(request_id, "embedding_text_limit")
    try:
        import mlx.core as mx  # type: ignore
        from mlx_embeddings import generate  # type: ignore

        model, tokenizer = load_model()
        started = time.monotonic()
        if BACKEND == "laya_encoder":
            import numpy as np  # type: ignore

            raw = _embed_fn(texts)
            norms = np.linalg.norm(raw, axis=1, keepdims=True)
            vectors = (raw / np.maximum(norms, 1e-12)).tolist()
            dimensions = int(raw.shape[1])
        else:
            output = generate(model, tokenizer, texts)
            mx.eval(output.text_embeds)
            vectors = output.text_embeds.tolist()
            dimensions = 1024
        if len(vectors) != len(texts) or any(len(vector) != dimensions for vector in vectors):
            return error(request_id, "embedding_dimensions")
        if any(not all(isinstance(value, (int, float)) and value == value for value in vector) for vector in vectors):
            return error(request_id, "embedding_values")
        return {
            "protocol_version": 1,
            "request_id": request_id,
            "model_id": MODEL_ID,
            "model_revision": MODEL_REVISION,
            "dimensions": dimensions,
            "vectors": vectors,
            "elapsed_ms": int((time.monotonic() - started) * 1000),
        }
    except ModuleNotFoundError:
        return error(request_id, "embedding_dependency_unavailable")
    except Exception:
        return error(request_id, "embedding_inference_failed")


def main() -> int:
    for raw in sys.stdin.buffer:
        if len(raw) > MAX_LINE_BYTES:
            sys.stdout.write(json.dumps(error("", "embedding_request_too_large")) + "\n")
            sys.stdout.flush()
            continue
        try:
            value = json.loads(raw)
            response = handle(value) if isinstance(value, dict) else error("", "embedding_request_invalid")
        except Exception:
            response = error("", "embedding_json_invalid")
        sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
