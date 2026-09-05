#!/usr/bin/env python3
"""从已冻结 manifest 离线生成语音，保留音频与工具 hash；从不录制麦克风。"""
import array
import datetime
import fcntl
import json
import math
import os
import platform
import subprocess
import tempfile
import wave
from pathlib import Path
from score import sha256, validate_manifest

ROOT = Path(__file__).resolve().parent


def write_json(path, value):
    data = json.dumps(value, ensure_ascii=False, indent=2) + "\n"
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(data)
    temporary.replace(path)


def inspect_audio(path):
    with wave.open(str(path), "rb") as audio:
        if (audio.getframerate(), audio.getnchannels(), audio.getsampwidth(), audio.getcomptype()) != (16000, 1, 2, "NONE"):
            raise ValueError(f"音频格式错误: {path.name}")
        count = audio.getnframes()
        samples = array.array("h", audio.readframes(count))
    if not samples or max(abs(value) for value in samples) == 0:
        raise ValueError(f"空音频或全静音: {path.name}")
    return {"sha256": sha256(path), "frames": count, "duration_seconds": count / 16000,
            "sample_rate_hz": 16000, "channels": 1, "pcm_bits": 16,
            "peak": max(abs(value) for value in samples) / 32768,
            "rms": math.sqrt(sum(value * value for value in samples) / len(samples)) / 32768}


def main():
    manifest_path = ROOT / "manifest.json"
    manifest = validate_manifest(json.loads(manifest_path.read_text()))
    artifacts = ROOT / "artifacts"
    artifacts.mkdir(exist_ok=True)
    with (artifacts / ".generation.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        voices = subprocess.run(["/usr/bin/say", "-v", "?"], check=True, capture_output=True, text=True).stdout
        for sample in manifest["samples"]:
            if not any(line.startswith(sample["voice"] + " ") for line in voices.splitlines()):
                raise ValueError("manifest 音色不在本机已列出音色中；禁止静默替换或下载")
        frozen = {"manifest_sha256": sha256(manifest_path),
                  "generator_sha256": sha256(__file__),
                  "platform": platform.platform(),
                  "tools": {path: sha256(path) for path in ["/usr/bin/say", "/usr/bin/afconvert"]}}
        freeze_path = artifacts / "freeze.json"
        if freeze_path.exists():
            if json.loads(freeze_path.read_text())["contract"] != frozen:
                raise ValueError("已冻结 manifest/生成器/工具与当前不同，拒绝覆盖音频")
        else:
            write_json(freeze_path, {"frozen_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "contract": frozen})
        evidence_path = artifacts / "audio-evidence.json"
        evidence = json.loads(evidence_path.read_text()) if evidence_path.exists() else {"manifest_sha256": frozen["manifest_sha256"], "audio": []}
        known = {row["id"]: row for row in evidence["audio"]}
        for sample in manifest["samples"]:
            destination = artifacts / (sample["id"] + ".wav")
            if sample["id"] in known:
                if not destination.exists() or sha256(destination) != known[sample["id"]]["sha256"]:
                    raise ValueError("已生成音频丢失或 hash 变化；拒绝静默重建")
                continue
            if destination.exists():
                raise ValueError("存在没有证据的同名音频，需先独立审计")
            with tempfile.TemporaryDirectory(prefix="synthesis-", dir=artifacts) as temporary:
                source = Path(temporary) / "source.aiff"
                wav = Path(temporary) / "converted.wav"
                subprocess.run(["/usr/bin/say", "-v", sample["voice"], "-r", str(sample["rate_wpm"]), "-o", str(source), sample["text"]], check=True, timeout=45)
                subprocess.run(["/usr/bin/afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", str(source), str(wav)], check=True, timeout=45)
                details = inspect_audio(wav)
                os.rename(wav, destination)
            row = {"id": sample["id"], "file": destination.name, "voice": sample["voice"], "rate_wpm": sample["rate_wpm"], **details}
            evidence["audio"].append(row)
            evidence["total_duration_seconds"] = sum(row["duration_seconds"] for row in evidence["audio"])
            evidence["completed_count"] = len(evidence["audio"])
            evidence["recognition_quality"] = "not_evaluated"
            evidence["listening_review"] = "not_evaluated"
            write_json(evidence_path, evidence)
            print(f"{len(evidence['audio'])}/100 {sample['id']} {details['duration_seconds']:.2f}s", flush=True)
        print(json.dumps({"count": len(evidence["audio"]), "duration_seconds": evidence["total_duration_seconds"], "manifest_sha256": frozen["manifest_sha256"]}))


if __name__ == "__main__":
    main()
