"""合成字节验证证据绑定，不冒充Mach-O/Rime运行。"""
import json
from pathlib import Path
import tempfile
import unittest
from artifact_binding import (
    begin,
    manifest_changed_fields,
    resolve_manifest_artifact,
    seal,
    verify,
)


class BindingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.output = self.root / "output"
        for relative, content in {
            "sources.lock.json": "source",
            "probe/main.cpp": "probe source",
            "output/include/rime_api.h": "header",
            "output/lib/libinputia_rime_static.a": "archive",
            "output/static-rime-probe": "executable",
            "output/source-snapshot.json": "verified extracted sources",
        }.items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)

    def receipt(self):
        begin(self.root, self.output)
        seal(self.root, self.output)
        return json.loads((self.output / "probe-link.json").read_text())

    def test_bound_inputs_verify_without_rewriting_metadata(self):
        receipt = self.receipt()
        before = (self.output / "probe-link.json").read_bytes()
        self.assertEqual(verify(self.root, self.output, receipt), receipt)
        self.assertEqual((self.output / "probe-link.json").read_bytes(), before)

    def test_replaced_archive_cannot_reuse_old_probe(self):
        receipt = self.receipt()
        (self.output / "lib/libinputia_rime_static.a").write_text("different archive")
        with self.assertRaisesRegex(RuntimeError, "library_sha256"):
            verify(self.root, self.output, receipt)

    def test_replaced_probe_cannot_reuse_old_manifest(self):
        receipt = self.receipt()
        (self.output / "static-rime-probe").write_text("different executable")
        with self.assertRaisesRegex(RuntimeError, "probe_sha256"):
            verify(self.root, self.output, receipt)

    def test_mutation_during_link_rejects_seal(self):
        begin(self.root, self.output)
        (self.output / "include/rime_api.h").write_text("changed header")
        with self.assertRaisesRegex(RuntimeError, "changed while linking"):
            seal(self.root, self.output)
        self.assertFalse((self.output / "probe-link.json").exists())

    def test_new_attempt_invalidates_old_link_receipt(self):
        self.receipt()
        begin(self.root, self.output)
        self.assertFalse((self.output / "probe-link.json").exists())

    def test_old_unbound_manifest_cannot_be_upgraded_by_verification(self):
        with self.assertRaisesRegex(RuntimeError, "binding mismatch"):
            verify(self.root, self.output, {"library_sha256": "old"})

    def test_relative_manifest_artifact_rebinds_to_copied_output(self):
        metadata = {
            "schema_version": 1,
            "library": "lib/libinputia_rime_static.a",
        }
        self.assertEqual(
            resolve_manifest_artifact(
                self.output,
                metadata,
                "library",
                "lib/libinputia_rime_static.a",
                "arm64",
            ),
            self.output / "lib/libinputia_rime_static.a",
        )

    def test_manifest_rejects_traversal_and_future_schema(self):
        with self.assertRaisesRegex(RuntimeError, "relative path mismatch"):
            resolve_manifest_artifact(
                self.output,
                {"schema_version": 1, "library": "../libinputia_rime_static.a"},
                "library",
                "lib/libinputia_rime_static.a",
                "arm64",
            )
        with self.assertRaisesRegex(RuntimeError, "unsupported"):
            resolve_manifest_artifact(
                self.output,
                {"schema_version": 2, "library": "lib/libinputia_rime_static.a"},
                "library",
                "lib/libinputia_rime_static.a",
                "arm64",
            )

    def test_legacy_absolute_reference_only_accepts_old_layout(self):
        expected = Path(
            "/tmp/source/native/static-rime/artifacts/output/arm64/lib/libinputia_rime_static.a"
        )
        self.assertEqual(
            resolve_manifest_artifact(
                self.output,
                {"library": str(expected)},
                "library",
                "lib/libinputia_rime_static.a",
                "arm64",
            ),
            self.output / "lib/libinputia_rime_static.a",
        )
        with self.assertRaisesRegex(RuntimeError, "legacy manifest path mismatch"):
            resolve_manifest_artifact(
                self.output,
                {"library": "/tmp/libinputia_rime_static.a"},
                "library",
                "lib/libinputia_rime_static.a",
                "arm64",
            )

    def test_complete_manifest_comparison_detects_changed_and_extra_evidence(self):
        expected = {
            "architecture": "arm64",
            "minimum_macos": "13.0",
            "signature": "verified",
        }
        observed = {
            **expected,
            "signature": "tampered",
            "extra": True,
            "unexpected_none": None,
        }
        self.assertEqual(
            manifest_changed_fields(observed, expected),
            ["extra", "signature", "unexpected_none"],
        )
        self.assertEqual(
            manifest_changed_fields({}, {"expected_none": None}),
            ["expected_none"],
        )


if __name__ == "__main__":
    unittest.main()
