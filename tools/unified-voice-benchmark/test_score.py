"""仅合成识别文本单测，不能作为真实 ASR 验收证据。"""
import copy
import json
import unittest
import tempfile
from pathlib import Path
from score import characters, compare, distance, has_term, score, validate_manifest, words, sha256, verified_run, validate_run_contract, validate_run_pair
from reconcile_format import semantic_bytes


class ScoringTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = json.loads(Path(__file__).with_name("manifest.json").read_text())

    def predictions(self):
        return [{"id": row["id"], "text": row["text"]} for row in self.manifest["samples"]]

    def test_frozen_corpus_counts_and_three_contexts_per_term(self):
        validate_manifest(self.manifest)
        for term in self.manifest["terms"]:
            examples = [row for row in self.manifest["samples"] if term["text"] in row["expected_terms"]]
            self.assertEqual(len(examples), 3)
            self.assertEqual(len({row["text"] for row in examples}), 3)
        self.assertEqual(sum(row["group"] == "ordinary_near_sound" for row in self.manifest["samples"]), 10)

    def test_perfect_synthetic_transcripts(self):
        result = score(self.manifest, self.predictions())
        self.assertEqual(result["overall"]["cer"], 0)
        self.assertEqual(result["ordinary"]["mixed_token_wer"], 0)
        self.assertEqual(result["terms"]["term_recall"], 1)
        self.assertEqual(result["overall"]["unspoken_term_sample_count"], 0)

    def test_edit_distance_and_normalization_contract(self):
        self.assertEqual(distance(list("abc"), list("adc")), 1)
        self.assertEqual(distance(list("abc"), []), 3)
        self.assertEqual(distance([], list("abc")), 3)
        self.assertEqual(characters("ＡB，中文！"), list("ab中文"))
        self.assertEqual(words("Hello，中文 test"), ["hello", "中", "文", "test"])
        self.assertFalse(has_term("rustic", "Rust"))
        self.assertTrue(has_term("使用 RUST", "Rust"))

    def test_unspoken_term_negative_is_detected(self):
        predictions = self.predictions()
        predictions[70]["text"] += " 星澜计划"
        result = score(self.manifest, predictions)
        self.assertEqual(result["ordinary"]["unspoken_term_sample_count"], 1)
        self.assertEqual(result["per_sample"][70]["unspoken_term_insertions"], ["星澜计划"])

    def test_missing_term_reduces_recall_without_homophone_forgiveness(self):
        predictions = self.predictions()
        predictions[0]["text"] = predictions[0]["text"].replace("星澜计划", "新蓝计划")
        result = score(self.manifest, predictions)
        self.assertAlmostEqual(result["terms"]["term_recall"], 59 / 60)
        self.assertGreater(result["overall"]["cer"], 0)

    def test_missing_duplicate_and_foreign_predictions_rejected(self):
        for predictions in [self.predictions()[:-1], self.predictions() + self.predictions()[:1],
                            self.predictions()[:-1] + [{"id": "unknown", "text": ""}]]:
            with self.assertRaises(ValueError):
                score(self.manifest, predictions)

    def test_comparison_does_not_claim_improvement_for_equal_perfect_results(self):
        baseline = score(self.manifest, self.predictions())
        self.assertFalse(compare(baseline, baseline)["term_recall_improved"])
        modified = copy.deepcopy(baseline)
        modified["ordinary"]["cer"] = 0.0101
        modified["overall"]["unspoken_term_sample_count"] = 1
        comparison = compare(baseline, modified)
        self.assertFalse(comparison["ordinary_cer_within_one_percentage_point"])
        self.assertFalse(comparison["no_unspoken_term_insertion"])

    def test_format_reconciliation_preserves_values_and_json_types(self):
        self.assertEqual(semantic_bytes({"a": 1, "b": "句子"}), semantic_bytes({"b": "句子", "a": 1}))
        self.assertNotEqual(semantic_bytes({"a": True}), semantic_bytes({"a": 1}))
        self.assertNotEqual(semantic_bytes({"a": 1.0}), semantic_bytes({"a": 1}))
        self.assertNotEqual(semantic_bytes({"a": "句子"}), semantic_bytes({"a": "改写"}))

    def test_reader_rejects_missing_run_contract(self):
        manifest_path = Path(__file__).with_name("manifest.json")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic-predictions.json"
            path.write_text(json.dumps({"manifest_sha256": sha256(manifest_path), "predictions": []}))
            with self.assertRaises(ValueError):
                verified_run(path, manifest_path, {"audio": []}, "baseline")

    def run_contract(self, condition="baseline"):
        return {"condition": condition, "model": {"id": "synthetic-model", "revision": "fixture-v1", "backend": "synthetic"},
                "run_parameters": {"binary_sha256": "a" * 64, "model_weights_sha256": "b" * 64,
                                   "decoding": {"beam_size": 1, "language": "zh"},
                                   "post_processing": {"enabled": False}},
                "terms_prompt": [] if condition == "baseline" else ["星澜计划"]}

    def test_model_condition_and_parameters_are_required(self):
        for key in ("condition", "model", "run_parameters", "terms_prompt"):
            data = self.run_contract()
            del data[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_run_contract(data, "baseline", self.manifest)
        for model in ({}, {"id": "x", "revision": " "}, {"id": 3, "revision": "v1"}):
            data = self.run_contract()
            data["model"] = model
            with self.assertRaises(ValueError):
                validate_run_contract(data, "baseline", self.manifest)
        for key in ("binary_sha256", "model_weights_sha256", "decoding", "post_processing"):
            data = self.run_contract()
            del data["run_parameters"][key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                validate_run_contract(data, "baseline", self.manifest)

    def test_pair_rejects_different_model_backend_decoding_and_postprocessing(self):
        baseline = self.run_contract()
        valid = self.run_contract("hotwords")
        validate_run_pair(baseline, valid, self.manifest)
        changes = [("model", "id", "other"), ("model", "revision", "v2"), ("model", "backend", "other"),
                   ("run_parameters", "binary_sha256", "c" * 64), ("run_parameters", "model_weights_sha256", "d" * 64),
                   ("run_parameters", "decoding", {"beam_size": 2}), ("run_parameters", "post_processing", {"enabled": True})]
        for section, key, value in changes:
            data = copy.deepcopy(valid)
            data[section][key] = value
            with self.subTest(section=section, key=key), self.assertRaises(ValueError):
                validate_run_pair(baseline, data, self.manifest)

    def test_swapped_conditions_and_hidden_or_unconfirmed_prompts_rejected(self):
        with self.assertRaises(ValueError):
            validate_run_pair(self.run_contract("hotwords"), self.run_contract(), self.manifest)
        for prompt in (None, ["未确认词"], ["星澜计划", "星澜计划"], []):
            data = self.run_contract("hotwords")
            data["terms_prompt"] = prompt
            with self.assertRaises(ValueError):
                validate_run_contract(data, "hotwords", self.manifest)
        data = self.run_contract()
        data["terms_prompt"] = ["星澜计划"]
        with self.assertRaises(ValueError):
            validate_run_contract(data, "baseline", self.manifest)
        data = self.run_contract()
        data["run_parameters"]["decoding"]["hotwords"] = ["星澜计划"]
        with self.assertRaises(ValueError):
            validate_run_contract(data, "baseline", self.manifest)

    def test_empty_configs_bad_hashes_and_nonfinite_parameters_rejected(self):
        for key, value in (("binary_sha256", "not-a-hash"), ("model_weights_sha256", "B" * 64),
                           ("decoding", {}), ("post_processing", None), ("decoding", {"temperature": float("nan")})):
            data = self.run_contract()
            data["run_parameters"][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                validate_run_contract(data, "baseline", self.manifest)

    def test_common_parameter_json_types_must_match(self):
        baseline, hotwords = self.run_contract(), self.run_contract("hotwords")
        hotwords["run_parameters"]["decoding"]["beam_size"] = True
        with self.assertRaises(ValueError):
            validate_run_pair(baseline, hotwords, self.manifest)


if __name__ == "__main__":
    unittest.main(verbosity=2)
