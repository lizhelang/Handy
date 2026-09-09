#!/usr/bin/env python3
"""将真实 headless CLI 输出打包给既有评分器；不执行识别或修改配置。"""
import argparse
import json
from pathlib import Path

from score import canonical_json, sha256, validate_manifest, validate_run_pair


def package(root, binary, weights, revision):
    assert not list((weights.parent / "custom-words-qwen3-0.6b").rglob("*.gguf")), \
        "此实验要求隔离模型目录中不存在额外纠错模型"
    manifest_path = Path(__file__).with_name("manifest.json")
    manifest = validate_manifest(json.loads(manifest_path.read_text()))
    artifacts = manifest_path.parent / "artifacts"
    evidence = json.loads((artifacts / "audio-evidence.json").read_text())
    assert evidence["manifest_sha256"] == sha256(manifest_path)
    assert len(evidence["audio"]) == 100
    for row in evidence["audio"]:
        assert sha256(artifacts / row["file"]) == row["sha256"]
    hashes = {row["id"]: row["sha256"] for row in evidence["audio"]}
    results = []
    shared_settings = None
    for condition in ("baseline", "hotwords"):
        directory = root / f"{condition}-raw-v1"
        settings = json.loads((directory / "settings-before.json").read_text())["settings"]
        prompt = settings.pop("custom_words")
        assert settings["word_correction_threshold"] == 0
        assert settings["filler_word_removal_enabled"] is False
        assert settings["post_process_enabled"] is False
        assert settings["selected_custom_words_model"] == ""
        assert not any(settings["post_process_api_keys"].values()), "禁止打包含凭据的配置"
        if shared_settings is None:
            shared_settings = settings
        assert settings == shared_settings, "除术语列表外的设置必须完全相同"
        rows = []
        model_ids, backends = set(), set()
        for sample in manifest["samples"]:
            raw = json.loads((directory / f'{sample["id"]}.json').read_text())
            assert isinstance(raw["text"], str)
            assert len(raw["transcribe_ms"]) == 1
            model_ids.add(raw["model"])
            backends.add(raw["bound_backend"])
            rows.append({"id": sample["id"], "audio_sha256": hashes[sample["id"]],
                         "text": raw["text"], "measurement": raw})
        assert len(model_ids) == len(backends) == 1
        results.append({
            "manifest_sha256": sha256(manifest_path), "condition": condition,
            "model": {"id": next(iter(model_ids)), "revision": revision,
                      "backend": next(iter(backends))},
            "run_parameters": {
                "binary_sha256": sha256(binary), "model_weights_sha256": sha256(weights),
                "decoding": {"language": settings["selected_language"],
                             "translate": settings["translate_to_english"],
                             "repeat": 1, "other_run_options": "compiled RunOptions::default",
                             "qwen_context_prefix": "术语参考（仅作转写提示，未说勿写）：",
                             "qwen_context_separator": "、",
                             "full_settings_without_term_list": canonical_json(settings)},
                "post_processing": {"normalization": "compiled normalize_transcription_output",
                                    "filler_removal": False, "fuzzy_correction_threshold": 0,
                                    "local_correction_model": "absent in isolated profile",
                                    "context_echo_cleanup": "compiled strip_trailing_echo"}},
            "terms_prompt": prompt, "predictions": rows})
    validate_run_pair(*results, manifest)
    for result in results:
        output = root / f'{result["condition"]}-predictions.json'
        with output.open("x") as stream:
            json.dump(result, stream, ensure_ascii=False, indent=2)
            stream.write("\n")
    print("packaged_actual_predictions=200")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--weights", required=True, type=Path)
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    package(args.root, args.binary, args.weights, args.revision)
