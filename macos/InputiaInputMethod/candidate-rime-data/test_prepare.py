import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import prepare


class CandidateResourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.base = self.root / "base"
        self.extensions = self.root / "extensions"
        self.base.mkdir()
        self.extensions.mkdir()
        for schema in set(prepare.SCHEMAS + prepare.EXTENDED_SCHEMAS) - {"double_pinyin_sogou"}:
            (self.base / (schema + ".schema.yaml")).write_text(
                "schema:\n  schema_id: " + schema + "\ntranslator:\n  dictionary: luna_pinyin # comment\n")
        (self.base / "default.yaml").write_text("config_version: '1'\nschema_list:\n  - schema: old\n\nmenu:\n  page_size: 5\n")
        (self.base / "untouched.dict.yaml").write_text("完整源词典\n")
        (self.base / "opencc").mkdir()
        (self.base / "opencc/example.ocd2").write_bytes(bytes(range(256)))
        for name in prepare.EXTENSIONS:
            (self.extensions / name).write_text("扩展词典-" + name + "\n")
        (self.extensions / "new-local-resource.txt").write_text("未来新增也保留")
        self.legacy = self.root / "legacy.sh"
        self.template = "schema:\n  schema_id: double_pinyin_sogou\ntranslator:\n  dictionary: luna_pinyin\n"
        self.legacy.write_text('/bin/cat >"$BUILD_DIR/double_pinyin_sogou.schema.yaml" <<\'YAML\'\n' + self.template + "YAML\n")
        self.output = self.root / "output"

    def assemble(self):
        return prepare.assemble(self.base, [], self.extensions, self.legacy, self.output)

    def test_full_base_and_all_repository_resources_preserved(self):
        base, local = self.assemble()
        result = prepare.inventory(self.output)
        self.assertEqual(result["untouched.dict.yaml"], base["untouched.dict.yaml"])
        self.assertEqual(result["opencc/example.ocd2"], base["opencc/example.ocd2"])
        for name, entry in local.items():
            self.assertEqual(result[name], entry)
        self.assertTrue(set(base).issubset(result))
        self.assertIn("new-local-resource.txt", result)

    def test_existing_schema_order_sogou_and_extension_contract(self):
        self.assemble()
        default = (self.output / "default.yaml").read_text()
        self.assertEqual(default.split("schema_list:\n")[1].split("menu:")[0],
                         "".join("  - schema: " + schema + "\n" for schema in prepare.SCHEMAS))
        self.assertIn("menu:\n  page_size: 5", default)
        self.assertEqual((self.output / "double_pinyin_sogou.schema.yaml").read_text(),
                         self.template.replace("dictionary: luna_pinyin", "dictionary: inputia_luna_pinyin"))
        for schema in prepare.EXTENDED_SCHEMAS:
            self.assertIn("dictionary: inputia_luna_pinyin\n", (self.output / (schema + ".schema.yaml")).read_text())

    def test_missing_extension_fails_instead_of_smaller_dictionary(self):
        (self.extensions / prepare.EXTENSIONS[-1]).unlink()
        with self.assertRaisesRegex(RuntimeError, "incomplete"):
            self.assemble()

    def test_ambiguous_legacy_template_fails(self):
        self.legacy.write_text(self.legacy.read_text() * 2)
        with self.assertRaisesRegex(RuntimeError, "contract changed"):
            self.assemble()

    def test_links_in_source_tree_are_rejected(self):
        (self.base / "alias").symlink_to(self.extensions, target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, "symbolic link"):
            self.assemble()

    def test_archive_duplicate_and_symlink_members_rejected(self):
        self.output.mkdir()
        resource = {"prefix": "locked", "files": {"schema.yaml": "schema.yaml"}}
        for kind in ["duplicate", "symlink"]:
            archive = self.root / (kind + ".tar.gz")
            with tarfile.open(archive, "w:gz") as bundle:
                member = tarfile.TarInfo("locked/schema.yaml")
                if kind == "symlink":
                    member.type = tarfile.SYMTYPE
                    member.linkname = "/outside"
                    bundle.addfile(member)
                else:
                    member.size = 1
                    bundle.addfile(member, io.BytesIO(b"x"))
                    bundle.addfile(member, io.BytesIO(b"x"))
            with self.assertRaisesRegex(RuntimeError, "unsafe"):
                prepare.copy_schema_archive(archive, resource, self.output)

    def test_corrupt_cache_rejected_without_download_or_fallback(self):
        archive = self.root / "locked.tar.gz"
        archive.write_bytes(b"corrupt")
        resource = {"file": archive.name, "sha256": hashlib.sha256(b"good").hexdigest()}
        with patch.object(prepare.subprocess, "run", side_effect=AssertionError("network must not run")):
            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                prepare.fetch(resource, self.root, False)

    def test_offline_missing_cache_fails(self):
        with self.assertRaisesRegex(RuntimeError, "offline"):
            prepare.fetch({"file": "missing.tar.gz"}, self.root, True)

    def test_run_and_output_do_not_escape_candidate_roots(self):
        for run in ["", "..", "a.b", "a/b", "a:b", " " , "a" * 65]:
            with self.assertRaisesRegex(RuntimeError, "run ID"):
                prepare.validate_output(Path("/tmp/RimeData"), run)
        with self.assertRaisesRegex(RuntimeError, "exact resource"):
            prepare.validate_output(Path("/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData"), "trial")
        prepare.validate_output(prepare.ROOT / "artifacts/outputs/trial/RimeData", "trial")

    def test_resource_tamper_does_not_refresh_manifest(self):
        self.assemble()
        evidence = {"schema_version": 1, "files": prepare.inventory(self.output)}
        manifest = self.output / prepare.MANIFEST
        manifest.write_text(json.dumps(evidence))
        before = manifest.read_bytes()
        (self.output / "untouched.dict.yaml").write_text("changed")
        with self.assertRaisesRegex(RuntimeError, "inventory mismatch"):
            prepare.verify_output(self.output)
        self.assertEqual(manifest.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
