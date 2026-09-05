#!/usr/bin/env python3
"""离线文本评分；不加载模型、不访问真实录音或用户词库。"""
import argparse
import hashlib
import json
import re
import unicodedata
from pathlib import Path


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def normalized(text):
    # 不合并同音字、不替换术语、不把数字改写成另一种读法。
    return unicodedata.normalize("NFKC", text).casefold()


def characters(text):
    return [ch for ch in normalized(text) if not ch.isspace()
            and unicodedata.category(ch)[0] not in ("P", "S")]


def words(text):
    # 明确定义 mixed-token WER：连续英文/数字为词，汉字逐字计词。
    return re.findall(r"[a-z0-9]+(?:['’-][a-z0-9]+)*|[^\W_]", normalized(text))


def distance(reference, hypothesis):
    previous = list(range(len(hypothesis) + 1))
    for index, token in enumerate(reference, 1):
        current = [index]
        for other_index, other in enumerate(hypothesis, 1):
            current.append(min(current[-1] + 1, previous[other_index] + 1,
                               previous[other_index - 1] + (token != other)))
        previous = current
    return previous[-1]


def has_term(text, term):
    text, term = normalized(text), normalized(term)
    if term.isascii():
        return re.search(r"(?<![a-z0-9_])" + re.escape(term) + r"(?![a-z0-9_])", text) is not None
    return term in text


def validate_manifest(manifest):
    samples = manifest["samples"]
    terms = [entry["text"] for entry in manifest["terms"]]
    if len(samples) != 100 or len(terms) != 20 or len(set(terms)) != 20:
        raise ValueError("必须冻结 100 段和 20 个唯一确认术语")
    if len({sample["id"] for sample in samples}) != len(samples):
        raise ValueError("音频 ID 重复")
    if sum(sample["group"] == "terms" for sample in samples) != 60:
        raise ValueError("含术语组必须为 60 段")
    for sample in samples:
        if not re.fullmatch(r"[a-z0-9-]+", sample["id"]):
            raise ValueError("音频 ID 不安全")
        expected, forbidden = sample["expected_terms"], sample["forbidden_terms"]
        if set(expected) & set(forbidden) or set(expected) | set(forbidden) != set(terms):
            raise ValueError("每段必须声明所有术语的预期/禁止关系")
        if any(not has_term(sample["text"], term) for term in expected):
            raise ValueError("gold 未包含预期术语")
        if any(has_term(sample["text"], term) for term in forbidden):
            raise ValueError("gold 包含指定未说术语")
    return manifest


def score(manifest, predictions):
    validate_manifest(manifest)
    ids = [item["id"] for item in predictions]
    if len(ids) != len(set(ids)) or set(ids) != {item["id"] for item in manifest["samples"]}:
        raise ValueError("预测必须覆盖每个音频且不能重复或额外添加 ID")
    by_id = {item["id"]: item for item in predictions}
    rows = []
    for sample in manifest["samples"]:
        text = by_id[sample["id"]]["text"]
        if not isinstance(text, str):
            raise ValueError("识别文本必须为字符串；空结果使用空字符串")
        reference_chars, hypothesis_chars = characters(sample["text"]), characters(text)
        reference_words, hypothesis_words = words(sample["text"]), words(text)
        rows.append({"id": sample["id"], "group": sample["group"],
                     "char_errors": distance(reference_chars, hypothesis_chars),
                     "reference_chars": len(reference_chars),
                     "word_errors": distance(reference_words, hypothesis_words),
                     "reference_words": len(reference_words),
                     "expected_term_count": len(sample["expected_terms"]),
                     "recalled_terms": [term for term in sample["expected_terms"] if has_term(text, term)],
                     "unspoken_term_insertions": [term for term in sample["forbidden_terms"] if has_term(text, term)]})

    def summarize(selected):
        chars = sum(row["reference_chars"] for row in selected)
        tokens = sum(row["reference_words"] for row in selected)
        expected = sum(row["expected_term_count"] for row in selected)
        return {"samples": len(selected),
                "cer": sum(row["char_errors"] for row in selected) / chars if chars else None,
                "mixed_token_wer": sum(row["word_errors"] for row in selected) / tokens if tokens else None,
                "term_recall": sum(len(row["recalled_terms"]) for row in selected) / expected if expected else None,
                "unspoken_term_sample_count": sum(bool(row["unspoken_term_insertions"]) for row in selected),
                "unspoken_term_insertion_count": sum(len(row["unspoken_term_insertions"]) for row in selected)}
    return {"normalization": "NFKC+casefold; CER ignores punctuation/symbol/space; mixed-token WER uses Latin words and Chinese characters",
            "overall": summarize(rows),
            "terms": summarize([row for row in rows if row["group"] == "terms"]),
            "ordinary": summarize([row for row in rows if row["group"] != "terms"]),
            "per_sample": rows}


def compare(baseline, hotwords):
    cer_delta = hotwords["ordinary"]["cer"] - baseline["ordinary"]["cer"]
    wer_delta = hotwords["ordinary"]["mixed_token_wer"] - baseline["ordinary"]["mixed_token_wer"]
    return {"term_recall_improved": hotwords["terms"]["term_recall"] > baseline["terms"]["term_recall"],
            "ordinary_cer_delta_percentage_points": cer_delta * 100,
            "ordinary_wer_delta_percentage_points": wer_delta * 100,
            "ordinary_cer_within_one_percentage_point": cer_delta <= 0.01 + 1e-12,
            "ordinary_wer_within_one_percentage_point": wer_delta <= 0.01 + 1e-12,
            "no_unspoken_term_insertion": hotwords["overall"]["unspoken_term_sample_count"] == 0,
            "notice": "仅文本门槛计算，不替代模型运行、音频听感、原生链路或完整 A11 验收。"}


def canonical_json(value):
    try:
        return json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    except (TypeError, ValueError) as error:
        raise ValueError("运行参数必须为有效且有限的 JSON 值") from error


def validate_run_contract(data, expected_condition, manifest):
    if expected_condition not in ("baseline", "hotwords") or data.get("condition") != expected_condition:
        raise ValueError("预测运行条件与 baseline/hotwords 角色不匹配")
    model = data.get("model")
    if not isinstance(model, dict) or any(not isinstance(model.get(key), str) or not model[key].strip() for key in ("id", "revision")):
        raise ValueError("model 必须记录非空 id 和 revision")
    parameters = data.get("run_parameters")
    if not isinstance(parameters, dict) or not parameters:
        raise ValueError("缺少共同 run_parameters")
    for key in ("binary_sha256", "model_weights_sha256"):
        if not isinstance(parameters.get(key), str) or not re.fullmatch(r"[a-f0-9]{64}", parameters[key]):
            raise ValueError(f"{key} 必须是 64 位小写十六进制 SHA-256")
    for key in ("decoding", "post_processing"):
        if not isinstance(parameters.get(key), dict) or not parameters[key]:
            raise ValueError(f"run_parameters 必须明确记录非空 {key} 配置")

    def reject_embedded_terms(value):
        if isinstance(value, dict):
            if any(key.casefold() in ("terms_prompt", "terms", "hotwords") for key in value):
                raise ValueError("术语提示必须独立放在 terms_prompt，不能混入共同运行参数")
            for item in value.values():
                reject_embedded_terms(item)
        elif isinstance(value, list):
            for item in value:
                reject_embedded_terms(item)
    reject_embedded_terms(parameters)
    prompt = data.get("terms_prompt")
    allowed = {entry["text"] for entry in manifest["terms"]}
    if not isinstance(prompt, list) or any(not isinstance(term, str) or term not in allowed for term in prompt) or len(set(prompt)) != len(prompt):
        raise ValueError("terms_prompt 必须是 manifest 已确认术语的去重列表")
    if (expected_condition == "baseline" and prompt) or (expected_condition == "hotwords" and not prompt):
        raise ValueError("baseline 术语提示必须为空，hotwords 术语提示必须非空")
    canonical_json(model)
    canonical_json(parameters)


def validate_run_pair(baseline, hotwords, manifest):
    validate_run_contract(baseline, "baseline", manifest)
    validate_run_contract(hotwords, "hotwords", manifest)
    for key in ("model", "run_parameters"):
        if canonical_json(baseline[key]) != canonical_json(hotwords[key]):
            raise ValueError(f"基线和热词组的完整 {key} 不一致，禁止质量比较")


def verified_run(path, manifest_path, evidence, expected_condition):
    data = json.loads(Path(path).read_text())
    manifest = validate_manifest(json.loads(Path(manifest_path).read_text()))
    validate_run_contract(data, expected_condition, manifest)
    if data.get("manifest_sha256") != sha256(manifest_path):
        raise ValueError("预测的 manifest hash 不匹配")
    hashes = {row["id"]: row["sha256"] for row in evidence["audio"]}
    for row in data["predictions"]:
        if row.get("audio_sha256") != hashes.get(row["id"]):
            raise ValueError("预测音频 hash 不匹配")
    return data


def validate_audio_evidence(manifest, manifest_path, evidence_path, evidence):
    if evidence["manifest_sha256"] != sha256(manifest_path):
        raise ValueError("生成证据不对应当前 manifest")
    expected_ids = {sample["id"] for sample in manifest["samples"]}
    actual_ids = [row["id"] for row in evidence["audio"]]
    if len(actual_ids) != len(set(actual_ids)) or set(actual_ids) != expected_ids:
        raise ValueError("生成证据必须完整且唯一覆盖音频集")
    for row in evidence["audio"]:
        if row["file"] != row["id"] + ".wav":
            raise ValueError("音频证据包含意外路径")
        path = evidence_path.parent / row["file"]
        if path.is_symlink() or sha256(path) != row["sha256"]:
            raise ValueError("固定音频丢失、被链接或 hash 不匹配")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).with_name("manifest.json"))
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--hotwords", type=Path)
    args = parser.parse_args()
    manifest = validate_manifest(json.loads(args.manifest.read_text()))
    evidence = json.loads(args.evidence.read_text())
    validate_audio_evidence(manifest, args.manifest, args.evidence, evidence)
    baseline_run = verified_run(args.baseline, args.manifest, evidence, "baseline")
    baseline = score(manifest, baseline_run["predictions"])
    report = {"manifest_sha256": sha256(args.manifest), "baseline": baseline,
              "model": baseline_run["model"], "run_parameters": baseline_run["run_parameters"],
              "baseline_terms_prompt": baseline_run["terms_prompt"]}
    if args.hotwords:
        hotwords_run = verified_run(args.hotwords, args.manifest, evidence, "hotwords")
        validate_run_pair(baseline_run, hotwords_run, manifest)
        hotwords = score(manifest, hotwords_run["predictions"])
        report.update(hotwords=hotwords, comparison=compare(baseline, hotwords),
                      hotwords_terms_prompt=hotwords_run["terms_prompt"])
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
