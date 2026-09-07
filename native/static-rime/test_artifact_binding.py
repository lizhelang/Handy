"""合成字节验证证据绑定，不冒充Mach-O/Rime运行。"""
import json
from pathlib import Path
import tempfile
import unittest
from artifact_binding import begin, seal, verify


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


if __name__ == "__main__":
    unittest.main()
