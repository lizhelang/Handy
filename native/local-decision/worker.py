#!/usr/bin/env python3
"""Bounded MLX typed-decision sidecar for Inputia.

The worker is intentionally a thin transport adapter. It never writes user
data, opens a network listener, or performs an action. If laya_mlx is absent,
requests fail visibly with a stable error and the Rust caller must fall back.
"""

from __future__ import annotations

import json
import os
import sys
import time
import traceback
from typing import Any

MAX_LINE_BYTES = 4 * 1024 * 1024
MODEL_ID = os.environ.get("INPUTIA_DECISION_MODEL", "laya-multilingual-mlx")
MODEL_REVISION = os.environ.get(
    "INPUTIA_DECISION_REVISION",
    "052592a15d198d9ad47da779604259b10b47b7aa",
)
_agent: Any = None


def _error(request_id: str, code: str) -> dict[str, Any]:
    return {
        "protocol_version": 1,
        "request_id": request_id,
        "model_id": MODEL_ID,
        "model_revision": MODEL_REVISION,
        "error": code,
    }


def _load() -> Any:
    global _agent
    if _agent is None:
        import laya_mlx  # type: ignore

        model = os.environ.get("INPUTIA_DECISION_MODEL_PATH", "aac6fef/laya-multilingual-mlx")
        _agent = laya_mlx.load(model, dtype="float16")
    return _agent


def _normalize_answers(raw: dict[str, Any], questions: dict[str, Any]) -> dict[str, Any]:
    """Convert Laya's upstream response names to Inputia's stable contract."""
    normalized: dict[str, Any] = {}
    for name, question in questions.items():
        answer = raw.get(name)
        if not isinstance(answer, dict):
            raise ValueError("answer_shape")
        kind = question.get("type")
        confidence = float(answer.get("confidence", 0.0))
        abstained = confidence < 0.5
        probabilities = answer.get("probabilities", {})
        if kind == "choice":
            selected = answer.get("choice")
            if not isinstance(selected, str) or not isinstance(probabilities, dict):
                raise ValueError("choice_shape")
            normalized[name] = {
                "type": "choice",
                "selected": selected,
                "probabilities": {str(k): float(v) for k, v in probabilities.items()},
                "confidence": confidence,
                "abstained": abstained,
            }
        elif kind == "noul":
            normalized[name] = {
                "type": "noul",
                "probability": float(answer.get("noul", 0.0)),
                "confidence": confidence,
                "abstained": abstained,
            }
        elif kind == "score":
            if not isinstance(probabilities, dict) or not probabilities:
                raise ValueError("score_shape")
            ordered = [float(probabilities[str(i)]) for i in range(len(probabilities))]
            selected = max(range(len(ordered)), key=lambda i: ordered[i])
            normalized[name] = {
                "type": "score",
                "selected": selected,
                "probabilities": ordered,
                "confidence": confidence,
                "abstained": abstained,
            }
        else:
            raise ValueError("question_type")
    return normalized


def handle(request: dict[str, Any]) -> dict[str, Any]:
    request_id = str(request.get("request_id", ""))
    if request.get("protocol_version") != 1 or not request_id or len(request_id) > 128:
        return _error(request_id, "decision_request_invalid")
    if request.get("kind") == "health":
        return {"protocol_version": 1, "request_id": request_id, "model_id": MODEL_ID,
                "model_revision": MODEL_REVISION, "ready": True}
    started = time.monotonic()
    if request.get("kind") != "decide":
        return _error(request_id, "decision_kind")
    try:
        agent = _load()
        result = agent.predict(request["state"], request["questions"])
        answers = result.get("answers") if isinstance(result, dict) else None
        questions = request.get("questions")
        if not isinstance(answers, dict) or not isinstance(questions, dict):
            return _error(request_id, "decision_response_invalid")
        return {
            "protocol_version": 1,
            "request_id": request_id,
            "model_id": MODEL_ID,
            "model_revision": MODEL_REVISION,
            "answers": _normalize_answers(answers, questions),
            "elapsed_ms": int((time.monotonic() - started) * 1000),
        }
    except ModuleNotFoundError:
        return _error(request_id, "decision_dependency_unavailable")
    except Exception:
        if os.environ.get("INPUTIA_DEBUG_MODEL") == "1":
            traceback.print_exc(file=sys.stderr)
        return _error(request_id, "decision_inference_failed")


def main() -> int:
    for raw in sys.stdin.buffer:
        if len(raw) > MAX_LINE_BYTES:
            sys.stdout.write(json.dumps(_error("", "decision_request_too_large")) + "\n")
            sys.stdout.flush()
            continue
        try:
            request = json.loads(raw)
            response = handle(request) if isinstance(request, dict) else _error("", "decision_request_invalid")
        except Exception:
            response = _error("", "decision_json_invalid")
        sys.stdout.write(json.dumps(response, ensure_ascii=False, separators=(",", ":")) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
