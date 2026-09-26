#!/usr/bin/env python3
"""Small local worker benchmark; reports model time only, not app IPC latency."""

from __future__ import annotations

import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
request = {
    "protocol_version": 1,
    "request_id": "benchmark",
    "kind": "decide",
    "state": {"text": "如何把语音历史接入知识库？"},
    "questions": {
        "intent": {
            "type": "choice",
            "instructions": "Classify the knowledge query intent.",
            "criteria": {
                "exact_lookup": "Find an exact fact or original text",
                "procedure": "Find steps or how-to instructions",
                "definition": "Find a definition",
                "unknown": "Cannot determine",
            },
        }
    },
}
payload = "".join(json.dumps({**request, "request_id": f"bench-{i}"}, ensure_ascii=False) + "\n" for i in range(11))
started = time.perf_counter()
worker = subprocess.run(
    [sys.executable, str(ROOT / "worker.py")],
    input=payload.encode(),
    stdout=subprocess.PIPE,
    check=True,
)
wall_ms = (time.perf_counter() - started) * 1000
rows = [json.loads(line) for line in worker.stdout.splitlines()]
elapsed = [row["elapsed_ms"] for row in rows[1:] if "elapsed_ms" in row]
if not elapsed:
    raise SystemExit("no valid warm measurements")
print(json.dumps({
    "warm_count": len(elapsed),
    "model_p50_ms": statistics.median(elapsed),
    "model_p95_ms": sorted(elapsed)[max(0, int(len(elapsed) * 0.95) - 1)],
    "wall_ms": round(wall_ms, 2),
    "note": "model elapsed excludes application IPC and first-load time",
}, ensure_ascii=False))
