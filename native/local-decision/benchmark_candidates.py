#!/usr/bin/env python3
"""只用固定合成语境评价已有本地模型，不下载模型或访问个人输入。"""

import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--resources", type=Path, required=True)
    args = parser.parse_args()
    worker = args.resources / "inputia-decision-worker/inputia-decision-worker"
    model = args.resources / "laya-multilingual-mlx"
    if not worker.is_file() or not model.is_dir():
        parser.error("必须提供已存在的本地worker与模型目录")
    cases = [
        ("heli", "经过讨论大家觉得这个方案很", ["合理", "合力", "河里", "盒里"], "合理"),
        ("heli", "小鱼从岸边跳回了", ["合理", "合力", "河里", "盒里"], "河里"),
        ("gongshi", "请推导这个数学", ["共识", "公式", "工事", "工时"], "公式"),
        ("gongshi", "经过反复讨论协商终于达成", ["共识", "公式", "工事", "工时"], "共识"),
        ("quanli", "每个公民依法享有的", ["权力", "权利", "全力"], "权利"),
        ("quanli", "为了比赛胜利我们拼尽", ["权力", "权利", "全力"], "全力"),
    ]
    payloads = []
    # 首项只预热，其余为不训练模型的独立评估。
    for i, (code, context, candidates, _) in enumerate([cases[0], *cases]):
        payloads.append(json.dumps({
            "kind": "decide", "protocol_version": 1, "request_id": f"candidate-benchmark-{i}",
            "model_id": "laya-multilingual-mlx", "state": {"context": context, "input_code": code},
            "questions": {"candidate": {
                "type": "choice",
                "instructions": "Choose the most likely candidate for this context and input code. Use only the supplied candidates.",
                "criteria": {str(index): word for index, word in enumerate(candidates)},
            }},
            "limits": {"deadline_ms": 150, "max_input_chars": 4000},
        }, ensure_ascii=False))
    env = dict(os.environ, INPUTIA_DECISION_MODEL_PATH=str(model.resolve()), HF_HUB_OFFLINE="1", TRANSFORMERS_OFFLINE="1")
    started = time.monotonic()
    result = subprocess.run([str(worker.resolve())], input="\n".join(payloads) + "\n", text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, timeout=60, check=True)
    rows = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
    if len(rows) != len(payloads):
        raise RuntimeError("worker没有返回完整的逐请求结果")
    observations = []
    for case, row in zip(cases, rows[1:]):
        answer = row.get("answers", {}).get("candidate", {})
        selected = answer.get("selected", "")
        text = case[2][int(selected)] if str(selected).isdigit() and int(selected) < len(case[2]) else None
        observations.append({"input_code": case[0], "context": case[1], "expected": case[3],
                             "selected": text, "correct": text == case[3],
                             "elapsed_ms": row.get("elapsed_ms"), "confidence": answer.get("confidence"),
                             "abstained": answer.get("abstained"), "error": row.get("error")})
    timings = sorted(o["elapsed_ms"] for o in observations if o["elapsed_ms"] is not None)
    print(json.dumps({"scope": "fixed synthetic contexts, existing local model, no training; excludes app IPC",
                      "correct": sum(o["correct"] for o in observations), "count": len(observations),
                      "usable_within_150ms": sum(o["elapsed_ms"] is not None and o["elapsed_ms"] <= 150 and not o["abstained"] and not o["error"] for o in observations),
                      "cold_ms": rows[0].get("elapsed_ms"), "warm_p50_ms": statistics.median(timings) if timings else None,
                      "warm_max_ms": max(timings) if timings else None,
                      "wall_ms": round((time.monotonic() - started) * 1000, 2), "observations": observations}, ensure_ascii=False))


if __name__ == "__main__":
    main()
