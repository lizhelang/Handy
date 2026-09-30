import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from build_trust import create, create_release, load, source, write_new, release_context


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


class ReleaseBuildTrustTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.context_path = self.root / "build-context.json"
        self.key = bytes([4]) + bytes(range(64))
        self.context = {"schema_version": 1, "product_id": "com.inputia",
                        "release_id": "inputia-1.1.0-84-" + "a" * 12 + "-" + "b" * 32,
                        "version": "1.1.0", "build": 84, "source_commit": "a" * 40,
                        "working_tree_clean": False, "mode": "local", "phase": "prepared",
                        "created_at": "2026-09-30T00:00:00Z", "target": {}, "python": "3.9",
                        "product_digest": "c" * 64, "public_release_eligible": False}
        write_new(self.context_path, json.dumps(self.context))
        self.metadata = create_release(self.key, self.context_path)
        self.metadata_path = self.root / "trust.json"
        write_new(self.metadata_path, json.dumps(self.metadata))

    def test_release_sources_share_static_binding_and_exclude_installation(self):
        metadata, key = load(self.metadata_path, context_path=self.context_path)
        for language in ["rust", "swift"]:
            generated = source(metadata, key, language)
            self.assertIn(metadata["release_id"], generated)
            self.assertIn(metadata["product_id"], generated)
            for forbidden in ["unified-candidate", "channel", "installation", "getenv", "runID", "profileID"]:
                self.assertNotIn(forbidden, generated)
        self.assertIn("LEGACY_PROFILE: Option<(&str, &str)> = None", source(metadata, key, "rust"))
        self.assertIn("trust: PairReleaseTrust", source(metadata, key, "swift"))

    def test_context_rebinding_and_byte_changes_rejected(self):
        self.context_path.write_text(json.dumps(self.context) + "\n")
        with self.assertRaises(ValueError):
            load(self.metadata_path, context_path=self.context_path)
        for field, value in [("product_id", "other"), ("source_commit", "d" * 40),
                             ("schema_version", True), ("phase", "signed"), ("build", True),
                             ("release_id", "inputia-"), ("public_release_eligible", True)]:
            self.context_path.write_text(json.dumps({**self.context, field: value}))
            with self.assertRaises(ValueError):
                release_context(self.context_path)

    def test_v2_metadata_unknown_duplicate_and_wrong_binding_rejected(self):
        for patch in [{"product_id": "other"}, {"release_id": "inputia-other"}, {"protocol_major": True},
                      {"source_commit": "d" * 40}, {"context_sha256": "d" * 64}, {"key_id": "other"},
                      {"schema_version": 1}, {"channel": "candidate"}, {"profile_id": "daily"}]:
            self.metadata_path.write_text(json.dumps({**self.metadata, **patch}))
            with self.assertRaises(ValueError):
                load(self.metadata_path, context_path=self.context_path)
        self.metadata_path.write_text('{"schema_version":2,' + json.dumps(self.metadata)[1:])
        with self.assertRaises(ValueError):
            load(self.metadata_path, context_path=self.context_path)

    def test_v2_never_falls_back_and_legacy_does_not_accept_v2(self):
        with self.assertRaises(ValueError):
            load(self.metadata_path, "synthetic-run")
        self.metadata_path.write_text(json.dumps(create(self.key, "synthetic-run")))
        with self.assertRaises(ValueError):
            load(self.metadata_path, context_path=self.context_path)
        with self.assertRaises(ValueError):
            load(self.metadata_path, "synthetic-run", self.context_path)

    def test_duplicate_context_and_nonfinite_rejected(self):
        for raw in ['{"schema_version":1,' + json.dumps(self.context)[1:],
                    json.dumps(self.context).replace('"build": 84', '"build": NaN')]:
            self.context_path.write_text(raw)
            with self.assertRaises(ValueError):
                release_context(self.context_path)

    def test_cli_rejects_changed_git_commit_before_emission(self):
        result = subprocess.run([sys.executable, str(Path(__file__).with_name("build_trust.py")),
                                 "--release-context", str(self.context_path), "--metadata",
                                 str(self.metadata_path), "--emit", "rust"], text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("source commit changed", result.stderr)
        self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
