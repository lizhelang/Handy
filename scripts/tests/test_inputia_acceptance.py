"""验收失败语义及证据绑定回归；只使用临时合成数据。"""
import copy
from datetime import datetime, timedelta, timezone
import json
import subprocess
import sys
import tempfile
import tomllib
import unittest
from uuid import uuid4
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import inputia_acceptance as acceptance


class AcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.proof = self.root / "proof.txt"
        self.proof.write_text("synthetic test fixture, not product acceptance\n")
        self.subject = {"product_id": "inputia", "release_id": "fixture-release",
                        "source_commit": "a" * 40, "manifest_sha256": "b" * 64,
                        "target": {"platform":"macos","architecture":"arm64","min_os":"13.0","tested_os":["13.0","26.0"]},
                        "artifacts": [{"role": "installer", "artifact": "Inputia.dmg", "sha256": "c" * 64, "size": 1}]}
        self.report = acceptance.initial_report(self.subject)

    def result(self, identity, report=None):
        return next(c for c in (report or self.report)["cases"] if c["id"] == identity)

    def passing(self, identity, report=None):
        result = self.result(identity, report)
        definition = next(c for c in acceptance.catalog()["cases"] if c["id"] == identity)
        result.update(status="PASS", evidence_level=definition["evidence_levels"][0],
                      reason="合成门禁测试", procedure=["synthetic fixture"], started_at=(datetime.now(timezone.utc)-timedelta(days=10)).isoformat().replace("+00:00","Z"), executed_at=acceptance.now(),
                      machine={"os": "macos", "os_version": "26.0" if identity == "G8.current-os" else "13.0", "architecture": "arm64", "model": "fixture"},
                      evidence=[{"path": "proof.txt", "sha256": acceptance.digest(self.proof)}],
                      metrics={key: next(iter(bounds.values())) for key, bounds in definition["metrics"].items()})
        end_days = 15
        start_days = 20
        if identity == "G10b.candidate-download":
            start_days, end_days = 14, 13
        elif identity == "G10b.observation":
            start_days, end_days = 12, 2
        elif identity == "G10b.stable-download":
            start_days, end_days = 1, 0
        result["started_at"] = (datetime.now(timezone.utc)-timedelta(days=start_days)).isoformat().replace("+00:00","Z")
        result["executed_at"] = (datetime.now(timezone.utc)-timedelta(days=end_days)).isoformat().replace("+00:00","Z")
        self.bind_execution(result)
        return result

    def bind_execution(self, result):
        path = self.root / (uuid4().hex + ".json")
        record = acceptance.execution_payload(result, self.subject,
            {"kind":"reviewed", "identity":"synthetic-fixture", "reviewer":"fixture-reviewer"})
        acceptance.write_report(path, record)
        result["execution_record"] = {"path":path.name,"sha256":acceptance.digest(path)}

    def valid(self, report=None):
        return acceptance.validate_report(report or self.report, self.root, self.subject)

    def test_initial_matrix_is_complete_and_not_passed(self):
        self.valid()
        result = acceptance.summarize(self.report, "pre-public")
        self.assertFalse(result["acceptance_passed"])
        self.assertFalse(result["publication_authorized"])
        self.assertGreater(result["required_cases"], 20)

    def test_partial_report_cannot_pass(self):
        self.passing("G0.source")
        self.report["cases"] = [self.result("G0.source")]
        with self.assertRaisesRegex(acceptance.AcceptanceError, "缺少必需"):
            self.valid()

    def test_duplicate_unknown_and_na_cases_rejected(self):
        for mutation in ("duplicate", "unknown", "na"):
            with self.subTest(mutation=mutation):
                report = copy.deepcopy(self.report)
                if mutation == "duplicate":
                    report["cases"].append(copy.deepcopy(report["cases"][0]))
                elif mutation == "unknown":
                    report["cases"][0]["id"] = "extra"
                else:
                    report["cases"][0]["status"] = "NOT_APPLICABLE"
                with self.assertRaises(acceptance.AcceptanceError):
                    self.valid(report)

    def test_mock_cannot_prove_physical_input(self):
        self.passing("G4.electron")["evidence_level"] = "ui_mock"
        with self.assertRaisesRegex(acceptance.AcceptanceError, "等级"):
            self.valid()

    def test_short_samples_and_bad_latency_do_not_pass(self):
        self.passing("G7.keys")["metrics"]["ordinary_added_p99_ms"] = 16
        with self.assertRaisesRegex(acceptance.AcceptanceError, "未达标"):
            self.valid()
        self.passing("G7.keys")["metrics"]["key_events"] = 20
        with self.assertRaisesRegex(acceptance.AcceptanceError, "未达标"):
            self.valid()

    def test_missing_metrics_and_boolean_count_rejected(self):
        self.passing("G2.migration")["metrics"].pop("records")
        with self.assertRaisesRegex(acceptance.AcceptanceError, "缺少验收指标"):
            self.valid()
        self.passing("G2.migration")["metrics"]["records"] = True
        with self.assertRaisesRegex(acceptance.AcceptanceError, "无效指标"):
            self.valid()

    def test_missing_or_changed_evidence_rejected(self):
        self.passing("G0.source")
        self.proof.write_text("changed")
        with self.assertRaisesRegex(acceptance.AcceptanceError, "摘要"):
            self.valid()
        self.proof.unlink()
        with self.assertRaisesRegex(acceptance.AcceptanceError, "文件缺失"):
            self.valid()

    def test_evidence_path_traversal_and_symlink_rejected(self):
        evidence = self.passing("G0.source")["evidence"][0]
        for path in ("../proof.txt", "/etc/passwd", "./proof.txt", "a//proof.txt", "a\\proof.txt", "https:proof.txt"):
            with self.subTest(path=path):
                evidence["path"] = path
                with self.assertRaises(acceptance.AcceptanceError):
                    self.valid()
        (self.root / "link.txt").symlink_to(self.proof)
        evidence["path"] = "link.txt"
        with self.assertRaisesRegex(acceptance.AcceptanceError, "符号链接"):
            self.valid()

    def test_other_artifact_or_source_report_rejected(self):
        for key in ("source_commit", "release_id", "manifest_sha256"):
            report = copy.deepcopy(self.report)
            report["subject"][key] = "d" * len(report["subject"][key])
            with self.assertRaises(acceptance.AcceptanceError):
                self.valid(report)

    def test_catalog_drift_is_rejected(self):
        self.report["catalog_sha256"] = "0" * 64
        with self.assertRaisesRegex(acceptance.AcceptanceError, "案例目录"):
            self.valid()

    def test_no_evidence_or_future_execution_is_rejected(self):
        self.passing("G0.source")["evidence"] = []
        with self.assertRaisesRegex(acceptance.AcceptanceError, "缺少执行证据"):
            self.valid()
        self.passing("G0.source")["executed_at"] = "2999-01-01T00:00:00Z"
        with self.assertRaisesRegex(acceptance.AcceptanceError, "未来"):
            self.valid()

    def test_stage_order_requires_candidate_and_observation(self):
        for case in acceptance.catalog()["cases"]:
            if case["stage"] == "pre-public":
                self.passing(case["id"])
        self.valid()
        self.assertTrue(acceptance.summarize(self.report, "pre-public")["acceptance_passed"])
        self.assertFalse(acceptance.summarize(self.report, "candidate")["acceptance_passed"])
        self.passing("G10b.candidate-download")
        self.assertTrue(acceptance.summarize(self.report, "candidate")["acceptance_passed"])
        self.assertFalse(acceptance.summarize(self.report, "stable")["acceptance_passed"])
        self.passing("G10b.observation")["metrics"]["duration_days"] = 6
        with self.assertRaisesRegex(acceptance.AcceptanceError, "未达标"):
            self.valid()

    def test_merge_combines_without_overwriting_failure(self):
        first = copy.deepcopy(self.report)
        second = copy.deepcopy(self.report)
        self.passing("G0.source", first)
        self.passing("G1.review", second)
        merged = acceptance.merge_reports([first, second], self.root, self.subject)
        self.assertEqual(self.result("G0.source", merged)["status"], "PASS")
        self.assertEqual(self.result("G1.review", merged)["status"], "PASS")
        third = copy.deepcopy(first)
        self.result("G0.source", third)["status"] = "FAIL"
        with self.assertRaisesRegex(acceptance.AcceptanceError, "冲突结果"):
            acceptance.merge_reports([first, third], self.root, self.subject)

    def test_json_duplicate_nan_unknown_keys_rejected(self):
        for text in ('{"a":1,"a":2}', '{"a":NaN}'):
            path = self.root / "invalid.json"
            path.write_text(text)
            with self.assertRaises(acceptance.AcceptanceError):
                acceptance.read_json(path)
        self.report["acceptance_passed"] = True
        with self.assertRaises(acceptance.AcceptanceError):
            self.valid()

    def test_write_never_overwrites_prior_evidence(self):
        path = self.root / "report.json"
        acceptance.write_report(path, self.report)
        with self.assertRaises(FileExistsError):
            acceptance.write_report(path, self.report)

    def test_cli_not_run_exit_is_nonzero(self):
        report_path = self.root / "report.json"
        acceptance.write_report(report_path, self.report)
        with patch.object(acceptance, "manifest_subject", return_value=self.subject), \
             patch.object(acceptance, "verify_source_catalog"), \
             patch("builtins.print"):
            code = acceptance.main(["verify", "--manifest", "unused", "--report", str(report_path),
                                    "--evidence-root", str(self.root)])
        self.assertEqual(code, 2)

    def test_runner_refuses_dirty_or_wrong_source(self):
        with patch.object(acceptance.subprocess, "check_output", side_effect=["a" * 40, " M source.rs"]):
            with self.assertRaisesRegex(acceptance.AcceptanceError, "干净工作区"):
                acceptance.run_rust(self.subject, self.root)

    def test_runner_includes_release_updater_and_cannot_hide_return_based_skips(self):
        called = []
        def execute(command, **kwargs):
            called.append(command)
            output = "test result: ok. 3 passed; 0 failed; 0 ignored\n"
            if "crates/inputia-capi/Cargo.toml" in command:
                output += "skip: required Rime runtime missing\n"
            return subprocess.CompletedProcess(command, 0, output)
        with patch.object(acceptance.subprocess, "check_output", side_effect=["a" * 40, "", "", "a" * 40]), \
             patch.object(acceptance.subprocess, "run", side_effect=execute):
            report = acceptance.run_rust(self.subject, self.root)
        self.assertEqual(len(called), 7)
        manifests = [command[command.index("--manifest-path") + 1] for command in called]
        self.assertIn("crates/inputia-release/Cargo.toml", manifests)
        updater = next(command for command in called if "crates/inputia-updater/Cargo.toml" in command)
        self.assertIn("native-code-verification", updater)
        self.assertTrue(all(command[-2:] == ["--", "--nocapture"] for command in called))
        for command in called:
            manifest = acceptance.ROOT / command[command.index("--manifest-path") + 1]
            metadata = tomllib.loads(manifest.read_text())
            if "--features" in command:
                self.assertIn(command[command.index("--features") + 1], metadata["features"])
        result = self.result("G1.rust", report)
        self.assertEqual(result["status"], "FAIL")
        self.assertEqual(result["metrics"]["skipped_required"], 1)
        self.assertFalse(acceptance.summarize(report, "pre-public")["acceptance_passed"])

    def test_negative_latency_and_fractional_counts_are_not_evidence(self):
        self.passing("G7.keys")["metrics"]["ordinary_added_p95_ms"] = -1
        with self.assertRaisesRegex(acceptance.AcceptanceError, "合法下界"):
            self.valid()
        self.passing("G7.keys")["metrics"]["key_events"] = 10000.5
        with self.assertRaisesRegex(acceptance.AcceptanceError, "整数"):
            self.valid()

    def test_observation_cannot_predate_candidate_availability(self):
        candidate = self.passing("G10b.candidate-download")
        candidate["started_at"] = (datetime.now(timezone.utc)-timedelta(hours=1)).isoformat().replace("+00:00","Z")
        candidate["executed_at"] = acceptance.now()
        self.bind_execution(candidate)
        self.passing("G10b.observation")
        with self.assertRaisesRegex(acceptance.AcceptanceError, "观察期早于"):
            self.valid()

    def test_duration_is_elapsed_time_not_only_reported_count(self):
        result = self.passing("G10b.observation")
        result["started_at"] = (datetime.now(timezone.utc)-timedelta(days=3)).isoformat().replace("+00:00","Z")
        self.bind_execution(result)
        with self.assertRaisesRegex(acceptance.AcceptanceError, "实际观察时间不足"):
            self.valid()

    def test_current_os_cannot_be_downgraded_by_manifest(self):
        subject = copy.deepcopy(self.subject)
        subject["target"]["tested_os"] = ["13.0"]
        with self.assertRaisesRegex(acceptance.AcceptanceError, "矩阵被降低"):
            acceptance.initial_report(subject)

    def test_unbound_text_or_another_case_record_cannot_prove_pass(self):
        source = self.passing("G0.source")
        source["execution_record"] = {"path":self.proof.name,"sha256":acceptance.digest(self.proof)}
        with self.assertRaises(ValueError):
            self.valid()
        source = self.passing("G0.source")
        other = self.passing("G1.review")
        other["execution_record"] = source["execution_record"]
        with self.assertRaisesRegex(acceptance.AcceptanceError, "未绑定同一案例"):
            self.valid()

    def test_shell_skipped_or_bypassed_ui_is_not_pass(self):
        helper = acceptance.ROOT / "macos/InputiaInputMethod/Tools/post-install-result.sh"
        for arguments, expected in ((["0", "0", "0"], 8), (["1", "0", "0"], 8),
                                     (["1", "1", "1"], 8), (["1", "1", "0"], 0)):
            with self.subTest(arguments=arguments):
                result = subprocess.run(["/bin/sh", "-c", '. "$1"; shift; inputia_post_install_result "$@"',
                                         "sh", str(helper), *arguments], capture_output=True, text=True)
                self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                if expected:
                    self.assertNotIn("postInstallRegressionPassed=true", result.stdout)
                else:
                    self.assertIn("evidenceLevel=native_api", result.stdout)
                    self.assertIn("releaseAcceptancePassed=false", result.stdout)


if __name__ == "__main__":
    unittest.main()
