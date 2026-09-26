#!/usr/bin/env python3
"""Evaluate the checked-in typed-decision fixture against a running worker."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
expected = json.loads((ROOT / "golden_expected.json").read_text())
requests = (ROOT / "golden.jsonl").read_bytes()
worker = [sys.executable, str(ROOT / "worker.py")]
completed = subprocess.run(worker, input=requests, stdout=subprocess.PIPE, check=True)
passed = 0
for raw in completed.stdout.splitlines():
    result = json.loads(raw)
    case = expected[result["request_id"]]
    answer = result.get("answers", {}).get(case["question"], {})
    if answer.get("type") == "choice":
        observed = answer.get("selected") if not answer.get("abstained") else "abstain"
    elif answer.get("type") == "noul":
        observed = answer.get("probability", 0) >= 0.5 if not answer.get("abstained") else "abstain"
    else:
        observed = "invalid"
    ok = observed == case["expected"]
    passed += int(ok)
    print(json.dumps({"id": result["request_id"], "expected": case["expected"], "observed": observed, "pass": ok}, ensure_ascii=False))
print(f"golden_pass={passed}/{len(expected)}")
