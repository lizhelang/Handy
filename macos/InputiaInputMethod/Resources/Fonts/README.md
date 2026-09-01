# Inputia candidate fallback fonts

Inputia bundles dictionary-scoped subsets of Jigmo 2025-09-12 so valid rare
CJK candidates remain visible when macOS system fonts do not contain a glyph.

- Upstream: https://kamichikoichi.github.io/jigmo/
- Upstream archive: `Jigmo-20250912.zip`
- License: CC0 1.0 Universal
- Coverage: Unicode 17.0 CJK Unified Ideographs through Extensions A-J
- Modification: the BMP font is subset to characters present in Inputia's
  bundled dictionaries. Jigmo2 and Jigmo3 retain complete supplementary-plane
  coverage because Rime/OpenCC can emit converted characters that are not
  literally present in the source dictionaries. All three fonts are renamed
  with an `Inputia` prefix to avoid conflicts with user-installed Jigmo fonts.
- Generator: `Tools/generate_candidate_fallback_fonts.py`

The complete upstream CC0 text is included as `Jigmo-LICENSE.txt`.
