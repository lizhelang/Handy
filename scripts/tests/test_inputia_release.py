"""发布合同回归：未验收数据不得升级成可信分发声明。"""
import copy
import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

MODULE = Path(__file__).resolve().parents[1] / "inputia_release.py"
spec = importlib.util.spec_from_file_location("inputia_release", MODULE)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def fixture(product):
    digest = "a" * 64
    stores = [{"id": sid, "readable_schema_range": {"min": 1, "max": 2}, "writable_schema_range": {"min": 2, "max": 2}, "migration_id": "expand-2", "event_formats": {"readable_versions": [1, 2], "writable_version": 2}, "outbox_capabilities": ["idempotent-consumer"], "revision_capabilities": ["monotonic-revision"], "privacy_capabilities": ["deletion-barrier", "forgetting-barrier"]} for sid in product["compatibility"]["required_stores"]]
    components = [{"role": c["role"], "bundle_id": c["bundle_id"], "artifact": f"components/{c['role']}.zip", "sha256": digest, "size": 4, "cdhashes": ["b" * 40], "signing_requirement": {"trust_domain": "developer-id", "team_id": "TESTTEAM01", "bundle_id": c["bundle_id"]}} for c in product["components"]]
    return {"schema_version": 2, "product_id": product["product_id"], "release_id": "inputia-test-current", "version": product["version"], "build": product["build"], "source_commit": "c" * 40, "target": {"platform": "macos", "architecture": "arm64", "min_os": "13.0", "tested_os": ["13", "26"]}, "components": components, "protocol": {"supported_majors": [1, 2], "capabilities": ["pair-auth"]}, "stores": stores, "rollback_targets": [{"release_id": "inputia-test-compatible", "artifact_digests": [digest], "stores": copy.deepcopy(stores)}], "resources": [{"id": "fixture-resource", "version": "1", "digest": digest, "license": "MIT", "required": True, "distribution_mode": "bundled"}], "pair_manifest": {"schema": 2, "artifact": "pair-manifest.json", "sha256": digest, "signer_key_id": "pair-key-1"}, "updater": {"min_version": "1.0.0", "transaction_schema": 1, "migration_requirements": []}, "distribution_artifacts": [{"role": "installer-dmg", "artifact": "Inputia.dmg", "sha256": digest, "size": 4}], "sbom_digest": digest}


class ReleaseContractTests(unittest.TestCase):
    def setUp(self):
        self.product = release.load_product()
        self.manifest = fixture(self.product)
        # 夹具可在非 macOS CI 执行；宿主拒绝行为另行覆盖。
        patcher = mock.patch.object(release, "host_target", return_value={"platform": "macos", "architecture": "arm64"})
        patcher.start()
        self.addCleanup(patcher.stop)

    def rejected(self, value):
        with self.assertRaises(release.ReleaseError):
            release.validate_manifest(value, self.product)

    def test_generated_configuration_has_no_runtime_identity(self):
        self.assertEqual(release.config_drift(self.product), [])
        for raw in release.generated_files(self.product).values():
            self.assertNotIn(b"trial-20260905", raw)
            self.assertNotIn(b"InputiaReleaseChannel", raw)

    def test_valid_contract_is_only_structure(self):
        self.assertEqual(release.validate_manifest(self.manifest, self.product), self.manifest)

    def test_historical_release_remains_valid_after_product_bump(self):
        self.manifest["version"] = "1.0.0"
        self.manifest["build"] = 74
        release.validate_manifest(self.manifest, self.product)
        with self.assertRaises(release.ReleaseError):
            release.validate_manifest(self.manifest, self.product, expect_current_build=True)

    def test_wrong_host_cannot_prepare_arm64_build(self):
        with mock.patch.object(release, "host_target", return_value={"platform": "macos", "architecture": "x86_64"}):
            self.assertIn("unsupported_build_host", release.preflight(self.product, "local")["blockers"])

    def test_binary_architecture_and_minimum_os_are_checked(self):
        with mock.patch.object(release.subprocess, "check_output", side_effect=["arm64", "cmd LC_BUILD_VERSION\nminos 13.0\n"]):
            release.verify_executable(Path("fixture-binary"), self.product)
        for outputs in (["x86_64"], ["arm64", "cmd LC_BUILD_VERSION\nminos 15.0\n"]):
            with mock.patch.object(release.subprocess, "check_output", side_effect=outputs):
                with self.assertRaises(release.ReleaseError):
                    release.verify_executable(Path("fixture-binary"), self.product)

    def test_local_bridge_scope_does_not_claim_complete_release_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            context = {"product_digest": hashlib.sha256(release.canonical_bytes(self.product)).hexdigest(), "release_id": "inputia-test-scope", "source_commit": "a" * 40}
            context_path = root / "context.json"
            context_path.write_text(json.dumps(context))
            for component in self.product["components"]:
                if component["role"] in ("updater", "bootstrap"):
                    continue
                path = root / component["app_name"] / "Contents/Info.plist"
                path.parent.mkdir(parents=True)
                path.write_bytes(plistlib.dumps({"CFBundleIdentifier": component["bundle_id"], "CFBundleShortVersionString": self.product["version"], "CFBundleVersion": str(self.product["build"]), "InputiaReleaseID": context["release_id"], "InputiaSourceCommit": context["source_commit"], "LSMinimumSystemVersion": self.product["target"]["min_os"], "CFBundleExecutable": "fixture"}))
            with mock.patch.object(release, "verify_executable", return_value={}):
                local = release.verify_bundles(root, context_path, self.product, "local-legacy")
                self.assertEqual(len(local["components"]), 3)
                self.assertFalse(local["public_release_eligible"])
                with self.assertRaises(FileNotFoundError):
                    release.verify_bundles(root, context_path, self.product)

    def test_rejects_unknown_identity_and_hash_cycle_fields(self):
        for key in ("channel", "profile_id", "installation_id", "attestation_digest", "manifest_digest"):
            with self.subTest(key=key):
                value = copy.deepcopy(self.manifest)
                value[key] = "forbidden"
                self.rejected(value)

    def test_rejects_bool_numbers_and_invalid_ranges(self):
        for value in (True, 0, 2**62, 1.2):
            manifest = copy.deepcopy(self.manifest)
            manifest["build"] = value
            self.rejected(manifest)
        self.manifest["stores"][0]["writable_schema_range"] = {"min": 3, "max": 1}
        self.rejected(self.manifest)

    def test_rejects_path_traversal_and_aliases(self):
        for path in ("../Inputia.dmg", "/Inputia.dmg", "a/../x", "a\\x", "a//x", "a/./x", "file:x", "a/\x00x"):
            with self.subTest(path=path):
                self.manifest["distribution_artifacts"][0]["artifact"] = path
                self.rejected(self.manifest)

    def test_rejects_duplicate_component_identity_and_bundle_mismatch(self):
        self.manifest["components"].append(copy.deepcopy(self.manifest["components"][0]))
        self.rejected(self.manifest)
        self.manifest = fixture(self.product)
        self.manifest["components"][0]["bundle_id"] = "com.other.control"
        self.rejected(self.manifest)

    def test_rejects_missing_recovery_component(self):
        self.manifest["components"] = [c for c in self.manifest["components"] if c["role"] != "bootstrap"]
        self.rejected(self.manifest)

    def test_recovery_identity_and_new_feed_keyset_contract_are_fixed(self):
        for role in ("updater", "bootstrap"):
            value = copy.deepcopy(self.manifest)
            component = next(c for c in value["components"] if c["role"] == role)
            component["bundle_id"] = "com.other.helper"
            component["signing_requirement"]["bundle_id"] = "com.other.helper"
            self.rejected(value)
        feed = {"schema_version": 2, "product_id": self.product["product_id"], "channel": "candidate", "platform": "macos", "architecture": "arm64", "sequence": 1, "keyset_version": 1, "keyset_digest": "a" * 64, "archive_policy_id": "release-2026", "issued_at": "2026-09-30T00:00:00Z", "expires_at": "2026-10-02T00:00:00Z", "release_id": self.manifest["release_id"], "manifest_digest": "b" * 64, "attestation_digest": "c" * 64, "release_path": "releases/" + self.manifest["release_id"], "rollback": None}
        release.validate_document(feed, "feed", self.product)
        for key, value in (("keyset_version", True), ("rollback", False), ("expires_at", "2026-10-30T00:00:00Z"), ("release_path", "releases/another")):
            changed = copy.deepcopy(feed)
            changed[key] = value
            with self.assertRaises(release.ReleaseError):
                release.validate_document(changed, "feed", self.product)

    def test_rollback_requires_all_stores_write_events_and_privacy(self):
        for mutation in ("store", "write", "event", "privacy", "outbox", "revision"):
            value = copy.deepcopy(self.manifest)
            target = value["rollback_targets"][0]
            if mutation == "store":
                target["stores"].pop()
            elif mutation == "write":
                target["stores"][0]["writable_schema_range"] = {"min": 1, "max": 1}
            elif mutation == "event":
                target["stores"][0]["event_formats"] = {"readable_versions": [1], "writable_version": 1}
            else:
                target["stores"][0][f"{mutation}_capabilities"] = []
            with self.subTest(mutation=mutation):
                self.rejected(value)

    def test_rollback_cannot_drop_new_capability(self):
        self.manifest["stores"][0]["privacy_capabilities"].append("epoch-lease")
        self.rejected(self.manifest)

    def test_json_duplicate_and_nonfinite_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "input.json"
            for raw in ('{"build": 1, "build": 2}', '{"build": NaN}', '{"build": Infinity}'):
                path.write_text(raw)
                with self.assertRaises(release.ReleaseError):
                    release.read_json(path)

    def test_config_check_catches_drift_without_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for relative, raw in release.generated_files(self.product).items():
                release.write_file(root / relative, raw)
            self.assertEqual(release.config_drift(self.product, root), [])
            path = root / "src-tauri/tauri.inputia-release.conf.json"
            value = json.loads(path.read_text())
            value["version"] = "9.0.0"
            path.write_text(json.dumps(value))
            before = path.read_bytes()
            self.assertEqual(release.config_drift(self.product, root), [str(path.relative_to(root))])
            self.assertEqual(path.read_bytes(), before)

    def test_public_preflight_stays_closed_without_real_trust_and_acceptance(self):
        with mock.patch.object(release, "git_state", return_value={"source_commit": "a" * 40, "working_tree_clean": False}):
            result = release.preflight(self.product, "public")
        self.assertIn("dirty_release_checkout", result["blockers"])
        self.assertNotIn("profile_bound_pair_trust_v1", result["blockers"])
        self.assertIn("developer_id_and_notarization_not_verified", result["blockers"])
        self.assertFalse(result["public_release_eligible"])
        self.assertFalse(result["certificate_accessed"])

    def test_prepare_reads_real_commit_and_creates_unique_build_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            first = release.prepare(self.product, Path(directory).resolve() / "first", "local")
            second = release.prepare(self.product, Path(directory).resolve() / "second", "local")
            self.assertNotEqual(first["release_id"], second["release_id"])
            self.assertEqual(first["source_commit"], release.git_state()["source_commit"])
            with self.assertRaises(release.ReleaseError):
                release.prepare(self.product, Path(directory).resolve() / "first", "local")
            self.assertFalse(first["public_release_eligible"])

    def test_public_prepare_leaves_no_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve() / "public"
            with self.assertRaises(release.ReleaseError):
                release.prepare(self.product, output, "public")
            self.assertFalse(output.exists())

    def test_apply_plist_before_signing_and_detects_product_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            release.prepare(self.product, root / "build", "local")
            path = root / "Info.plist"
            path.write_bytes(plistlib.dumps({"CFBundleIdentifier": self.product["components"][0]["bundle_id"], "InputiaReleaseChannel": "stable"}))
            release.apply_plist(path, "control", root / "build/build-context.json", self.product)
            info = plistlib.loads(path.read_bytes())
            self.assertEqual(info["CFBundleVersion"], str(self.product["build"]))
            self.assertNotIn("InputiaReleaseChannel", info)
            changed = copy.deepcopy(self.product)
            changed["build"] += 1
            with self.assertRaises(release.ReleaseError):
                release.apply_plist(path, "control", root / "build/build-context.json", changed)

    def test_frozen_archive_digest_and_symlink_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for item in [*self.manifest["components"], *self.manifest["distribution_artifacts"], self.manifest["pair_manifest"]]:
                path = root / item["artifact"]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"test")
                item["sha256"] = hashlib.sha256(b"test").hexdigest()
            release.verify_artifacts(self.manifest, root)
            path = root / self.manifest["components"][0]["artifact"]
            path.unlink()
            path.symlink_to(root / "Inputia.dmg")
            with self.assertRaises(release.ReleaseError):
                release.verify_artifacts(self.manifest, root)

    def test_bind_manifest_recomputes_frozen_artifact_digests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            artifact_root = root / "artifacts"
            artifact_root.mkdir()
            for item in [*self.manifest["components"], *self.manifest["distribution_artifacts"], self.manifest["pair_manifest"]]:
                path = artifact_root / item["artifact"]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"bound-artifact")
            context = {"schema_version": 1, "phase": "prepared", "product_id": self.product["product_id"], "release_id": self.manifest["release_id"], "version": self.manifest["version"], "build": self.manifest["build"], "source_commit": self.manifest["source_commit"], "target": self.manifest["target"], "product_digest": hashlib.sha256(release.canonical_bytes(self.product)).hexdigest()}
            context_path = root / "context.json"
            context_path.write_text(json.dumps(context))
            template_path = root / "template.json"
            template_path.write_text(json.dumps(self.manifest))
            output = root / "release-manifest.json"
            result = release.bind_manifest(template_path, context_path, artifact_root, output, self.product)
            self.assertTrue(result["manifest_bound"])
            bound = release.read_json(output)
            expected = hashlib.sha256(b"bound-artifact").hexdigest()
            self.assertTrue(all(item["sha256"] == expected for item in bound["components"]))
            self.assertEqual(bound["distribution_artifacts"][0]["size"], len(b"bound-artifact"))
            self.assertEqual(bound["pair_manifest"]["sha256"], expected)

    def test_cli_does_not_claim_signature_verification(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / "manifest.json"
            path.write_text(json.dumps(self.manifest))
            result = subprocess.run([sys.executable, str(MODULE), "validate", "--kind", "manifest", "--document", str(path)], text=True, capture_output=True, check=True)
            report = json.loads(result.stdout)
            self.assertEqual(report["signature_verification"], "NOT_RUN")
            self.assertEqual(report["artifact_verification"], "NOT_RUN")
            self.assertFalse(report["public_release_eligible"])

    def test_cli_verifies_signed_envelope_with_explicit_trusted_key(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            private = root / "private.pem"
            public_der = root / "public.der"
            subprocess.run(["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", str(private)], check=True, capture_output=True)
            subprocess.run(["openssl", "ec", "-in", str(private), "-pubout", "-outform", "DER", "-out", str(public_der)], check=True, capture_output=True)
            raw_public = public_der.read_bytes()[-65:]
            key_id = "sha256-" + hashlib.sha256(raw_public).hexdigest()
            payload = release.canonical_bytes(self.manifest)
            signing = b"Inputia.Release.v1\0manifest\0" + payload
            signing_path = root / "signing.bin"
            signature_path = root / "signature.der"
            signing_path.write_bytes(signing)
            subprocess.run(["openssl", "dgst", "-sha256", "-sign", str(private), "-out", str(signature_path), str(signing_path)], check=True, capture_output=True)
            envelope = {"schema_version": 1, "payload_kind": "manifest", "payload": self.manifest, "signatures": [{"key_id": key_id, "algorithm": "ecdsa-p256-sha256", "signature_der_base64": base64.b64encode(signature_path.read_bytes()).decode()}]}
            document = root / "envelope.json"
            document.write_text(json.dumps(envelope))
            keys = root / "keys.json"
            keys.write_text(json.dumps({"threshold": 1, "keys": [{"key_id": key_id, "public_key_x963_base64": base64.b64encode(raw_public).decode()}]}))
            result = subprocess.run([sys.executable, str(MODULE), "validate", "--kind", "manifest", "--document", str(document), "--trusted-keys", str(keys)], text=True, capture_output=True, check=True)
            report = json.loads(result.stdout)
            self.assertEqual(report["signature_verification"], "PASS")
            self.assertEqual(report["valid_signatures"], 1)
            self.assertFalse(report["public_release_eligible"])

    def test_channels_reference_same_immutable_manifest(self):
        original = release.canonical_bytes(self.manifest)
        manifest_digest = hashlib.sha256(original).hexdigest()
        for channel in ("candidate", "stable"):
            feed = {"schema_version": 1, "product_id": self.product["product_id"], "channel": channel, "platform": "macos", "architecture": "arm64", "sequence": 5, "issued_at": "2026-09-30T01:00:00Z", "expires_at": "2026-10-30T01:00:00Z", "release_id": self.manifest["release_id"], "manifest_digest": manifest_digest, "attestation_digest": "d" * 64, "release_path": "releases/inputia-test-current", "rollback": False}
            release.validate_document(feed, "feed", self.product)
            self.assertEqual(release.canonical_bytes(self.manifest), original)

    def test_unsupported_python_has_actionable_error(self):
        if Path("/usr/bin/python3").exists():
            result = subprocess.run(["/usr/bin/python3", str(MODULE), "validate-product"], text=True, capture_output=True)
            if result.returncode:
                self.assertIn("Python >= 3.11", result.stderr)


if __name__ == "__main__":
    unittest.main()
