import tempfile
import unittest
from pathlib import Path

from generate_inputia_lexicons import add_poetry_layers, write_dict


class PoetryQualityTests(unittest.TestCase):
    def test_preserves_source_clauses_without_inventing_substrings(self):
        pronunciations = dict(zip("轻舟已过万重山", "qing zhou yi guo wan chong shan".split()))
        bucket = {}
        add_poetry_layers(bucket, "轻舟已过，万重山。", pronunciations, 5000)
        self.assertEqual(set(bucket), {"轻舟已过万重山", "轻舟已过", "万重山"})
        self.assertNotIn("轻舟已", bucket)
        self.assertNotIn("舟已过", bucket)

    def test_keeps_short_source_words_and_writes_deterministically(self):
        bucket = {}
        pronunciations = {"秋": "qiu", "风": "feng"}
        add_poetry_layers(bucket, "秋风", pronunciations, 5000)
        self.assertIn("秋风", bucket)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "poetry.dict.yaml"
            write_dict(path, "inputia_poetry", bucket)
            first = path.read_bytes()
            write_dict(path, "inputia_poetry", bucket)
            self.assertEqual(path.read_bytes(), first)

    def test_packaged_dictionary_retains_phrases_without_old_fragments(self):
        path = Path(__file__).resolve().parents[1] / "Resources/RimeData/inputia_poetry.dict.yaml"
        entries = {line.split("\t")[0] for line in path.read_text().splitlines() if "\t" in line}
        self.assertIn("长风破浪会有时", entries)
        self.assertIn("长风破浪", entries)
        self.assertNotIn("风来满", entries)
        self.assertNotIn("轻舟已", entries)


if __name__ == "__main__":
    unittest.main()
