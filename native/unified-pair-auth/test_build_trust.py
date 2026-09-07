import json
from pathlib import Path
import tempfile
import unittest
from build_trust import create, load, source, write_new


class BuildTrustTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name).resolve() / "public-build.json"
        self.key = bytes([4]) + bytes(range(64))
        self.metadata = create(self.key, "synthetic-run")
        write_new(self.path, json.dumps(self.metadata))

    def test_only_public_key_is_accepted(self):
        for invalid in [b"", bytes(65), bytes(97), self.key + b"private"]:
            with self.assertRaises(ValueError):
                create(invalid, "synthetic-run")

    def test_pair_sources_share_key_profile_and_no_runtime_environment_lookup(self):
        metadata, key = load(self.path, "synthetic-run")
        for language in ["rust", "swift"]:
            generated = source(metadata, key, language)
            self.assertIn(metadata["key_id"], generated)
            self.assertIn(metadata["profile_id"], generated)
            self.assertNotIn("environment", generated)
            self.assertNotIn("getenv", generated)

    def test_changed_run_or_key_identity_is_rejected(self):
        with self.assertRaises(ValueError):
            load(self.path, "different-run")
        self.metadata["key_id"] = "another-build"
        self.path.write_text(json.dumps(self.metadata))
        with self.assertRaises(ValueError):
            load(self.path, "synthetic-run")

    def test_unknown_fields_and_boolean_schema_are_rejected(self):
        for patch in [{"private_key": "forbidden"}, {"schema_version": True}]:
            self.path.write_text(json.dumps({**self.metadata, **patch}))
            with self.assertRaises(ValueError):
                load(self.path, "synthetic-run")

    def test_run_injection_and_overwrite_are_rejected(self):
        for run in ["../daily", "bad\nrun", '"; code', "a" * 65]:
            with self.assertRaises(ValueError):
                create(self.key, run)
        with self.assertRaises(FileExistsError):
            write_new(self.path, "changed")

    def test_linked_or_publicly_writable_metadata_is_rejected(self):
        alias = self.path.with_name("alias")
        alias.symlink_to(self.path)
        with self.assertRaises(ValueError):
            load(alias, "synthetic-run")
        self.path.chmod(0o666)
        with self.assertRaises(ValueError):
            load(self.path, "synthetic-run")


if __name__ == "__main__":
    unittest.main()
