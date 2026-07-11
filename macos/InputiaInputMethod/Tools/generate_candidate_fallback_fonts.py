#!/usr/bin/env python3
"""Build deterministic Inputia-only Jigmo subsets for bundled Rime dictionaries."""

from __future__ import annotations

import argparse
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont


FONT_PARTS = (
    ("Jigmo.ttf", "InputiaJigmo.ttf", "Inputia Jigmo", "InputiaJigmo", True),
    ("Jigmo2.ttf", "InputiaJigmo2.ttf", "Inputia Jigmo 2", "InputiaJigmo2", False),
    ("Jigmo3.ttf", "InputiaJigmo3.ttf", "Inputia Jigmo 3", "InputiaJigmo3", False),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--dictionary-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    return parser.parse_args()


def dictionary_characters(dictionary_dir: Path) -> str:
    paths = [dictionary_dir / "luna_pinyin.dict.yaml"]
    paths.extend(sorted(dictionary_dir.glob("inputia_*.dict.yaml")))
    characters: set[str] = set()
    for path in paths:
        if not path.is_file():
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line or line.startswith("#") or line.startswith("---"):
                continue
            term = line.split("\t", 1)[0]
            characters.update(term)
    return "".join(sorted(characters, key=ord))


def rename_font(font: TTFont, family: str, postscript_name: str) -> None:
    replacements = {
        1: family,
        2: "Regular",
        3: f"{postscript_name};Inputia;20250912",
        4: family,
        6: postscript_name,
        16: family,
        17: "Regular",
    }
    for record in font["name"].names:
        replacement = replacements.get(record.nameID)
        if replacement is None:
            continue
        record.string = replacement.encode(record.getEncoding(), errors="replace")


def subset_font(source: Path, destination: Path, text: str, family: str, postscript_name: str) -> None:
    options = subset.Options()
    options.glyph_names = True
    options.layout_features = ["*"]
    options.name_IDs = list(range(0, 26))
    options.name_legacy = True
    options.notdef_glyph = True
    options.notdef_outline = True
    options.recommended_glyphs = True
    options.recalc_timestamp = False

    font = subset.load_font(str(source), options)
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text=text)
    subsetter.subset(font)
    rename_font(font, family, postscript_name)
    destination.parent.mkdir(parents=True, exist_ok=True)
    font.save(destination, reorderTables=True)


def copy_and_rename_font(source: Path, destination: Path, family: str, postscript_name: str) -> None:
    font = TTFont(source, recalcTimestamp=False)
    rename_font(font, family, postscript_name)
    destination.parent.mkdir(parents=True, exist_ok=True)
    font.save(destination, reorderTables=True)


def main() -> None:
    args = parse_args()
    text = dictionary_characters(args.dictionary_dir)
    if not text:
        raise SystemExit("no dictionary characters found")

    for source_name, output_name, family, postscript_name, should_subset in FONT_PARTS:
        source = args.source_dir / source_name
        if not source.is_file():
            raise SystemExit(f"missing source font: {source}")
        destination = args.output_dir / output_name
        if should_subset:
            subset_font(source, destination, text, family, postscript_name)
        else:
            copy_and_rename_font(source, destination, family, postscript_name)
        print(args.output_dir / output_name)


if __name__ == "__main__":
    main()
