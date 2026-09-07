"""用新解包树作为基准；不允许缓存自己证明自己。"""
from pathlib import Path
import tempfile
import unittest
from source_snapshot import publish, verify


class SourceSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.artifacts = self.root / "artifacts"
        self.artifacts.mkdir()
        self.lock = self.root / "sources.lock.json"
        self.lock.write_text("locked fixture source")
        self.count = 0

    def fresh(self):
        self.count += 1
        tree = self.root / ("fresh-" + str(self.count))
        (tree / "core/plugins/lua").mkdir(parents=True)
        (tree / "core/code.cpp").write_text("verified archive bytes")
        (tree / "core/plugins/lua/code.cpp").write_text("verified plugin bytes")
        return tree

    def publish(self):
        return publish(self.fresh(), self.artifacts, self.lock, {"source_root": "core"})

    def test_same_fresh_tree_reuses_only_verified_generation(self):
        first, reused = self.publish()
        self.assertFalse(reused)
        second, reused = self.publish()
        self.assertTrue(reused)
        self.assertEqual(first, second)

    def test_changed_core_or_plugin_is_preserved_but_not_reused(self):
        for relative in ["core/code.cpp", "core/plugins/lua/code.cpp"]:
            first, _ = self.publish()
            corrupted = Path(first["tree_root"]) / relative
            corrupted.write_text("untrusted local modification")
            with self.assertRaisesRegex(RuntimeError, "changed since extraction"):
                verify(self.artifacts / "source-snapshot.json", self.lock)
            second, reused = self.publish()
            self.assertFalse(reused)
            self.assertNotEqual(first["generation_id"], second["generation_id"])
            self.assertTrue(corrupted.exists())
            self.assertEqual(corrupted.read_text(), "untrusted local modification")

    def test_added_file_and_changed_lock_are_rejected(self):
        snapshot, _ = self.publish()
        (Path(snapshot["tree_root"]) / "injected.cpp").write_text("extra")
        with self.assertRaises(RuntimeError):
            verify(self.artifacts / "source-snapshot.json", self.lock)
        self.publish()
        self.lock.write_text("different source lock")
        with self.assertRaisesRegex(RuntimeError, "lock/schema"):
            verify(self.artifacts / "source-snapshot.json", self.lock)

    def test_escape_link_and_hardlink_are_rejected(self):
        tree = self.fresh()
        (tree / "escape").symlink_to(self.lock)
        with self.assertRaisesRegex(RuntimeError, "escapes tree"):
            publish(tree, self.artifacts, self.lock, {"source_root": "core"})
        tree = self.fresh()
        import os
        os.link(tree / "core/code.cpp", tree / "hardlink.cpp")
        with self.assertRaisesRegex(RuntimeError, "hardlinked"):
            publish(tree, self.artifacts, self.lock, {"source_root": "core"})


if __name__ == "__main__":
    unittest.main()
