#!/usr/bin/env python3
"""只允许 JSON 完全相等的格式调整，保留生成时证据与当前文件 hash 的关联。"""
import datetime
import hashlib
import json
from pathlib import Path
from generate import write_json
from score import sha256, validate_audio_evidence


def semantic_bytes(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def main():
    root = Path(__file__).resolve().parent
    artifacts = root / "artifacts"
    original = artifacts / "manifest.before-format.json"
    current = root / "manifest.json"
    old_data = json.loads(original.read_text())
    new_data = json.loads(current.read_text())
    if semantic_bytes(old_data) != semantic_bytes(new_data):
        raise ValueError("不是纯格式变化：禁止修改 gold、术语或合成参数")
    freeze_path = artifacts / "freeze.json"
    evidence_path = artifacts / "audio-evidence.json"
    receipt_path = artifacts / "format-only-reconciliation.json"
    if receipt_path.exists():
        receipt = json.loads(receipt_path.read_text())
        if receipt["manifest_file_sha256"] != sha256(current) or receipt["manifest_generation_sha256"] != sha256(original):
            raise ValueError("已有格式映射与文件不一致")
        return
    freeze = json.loads(freeze_path.read_text())
    evidence = json.loads(evidence_path.read_text())
    if freeze["contract"]["manifest_sha256"] != sha256(original):
        raise ValueError("原始 manifest 不匹配生成前冻结证据")
    validate_audio_evidence(old_data, original, evidence_path, evidence)
    # 保存原始冻结和音频账本字节，不覆盖它们。
    for path in (freeze_path, evidence_path):
        archive = path.with_name(path.stem + ".at-generation.json")
        with archive.open("xb") as output:
            output.write(path.read_bytes())
    semantic = hashlib.sha256(semantic_bytes(old_data)).hexdigest()
    receipt = {"reason": "仅项目 Prettier 格式化；JSON 值逐项完全相等，没有改变 gold 或参数。",
               "at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
               "manifest_generation_sha256": sha256(original),
               "manifest_file_sha256": sha256(current), "manifest_semantic_sha256": semantic,
               "freeze_at_generation_sha256": sha256(artifacts / "freeze.at-generation.json"),
               "audio_evidence_at_generation_sha256": sha256(artifacts / "audio-evidence.at-generation.json")}
    freeze["contract"]["manifest_sha256"] = sha256(current)
    freeze["format_only_reconciliation"] = receipt
    evidence.update(manifest_sha256=sha256(current), manifest_generation_sha256=sha256(original), manifest_semantic_sha256=semantic)
    write_json(receipt_path, receipt)
    write_json(freeze_path, freeze)
    write_json(evidence_path, evidence)
    print(json.dumps(receipt, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
