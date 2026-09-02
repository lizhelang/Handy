#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_RESOURCES_DIR = ROOT / "Resources"
DEFAULT_SETTINGS_PATH = (
    Path.home() / "Library/Application Support/Inputia/settings.json"
)

TEAL = "#2F6F73"
DARK = "#24575A"
BACKGROUND = "#F8F4EC"
VARIANTS = [
    ("pearl_12", "连珠 12 颗", 12),
    ("pearl_14", "连珠 14 颗", 14),
    ("pearl_16", "连珠 16 颗", 16),
    ("pearl_18", "连珠 18 颗", 18),
]
DEFAULT_VARIANT = "pearl_16"
LOGO_OUTER_RADIUS = 500.0
MENU_ICON_POINTS = 16


def circle_path(cx: float, cy: float, radius: float) -> str:
    return (
        f"M {cx + radius:.3f} {cy:.3f} "
        f"A {radius:.3f} {radius:.3f} 0 1 0 {cx - radius:.3f} {cy:.3f} "
        f"A {radius:.3f} {radius:.3f} 0 1 0 {cx + radius:.3f} {cy:.3f} Z"
    )


def variant_geometry(pearl_count: int) -> tuple[float, float, float, list[tuple[float, float]]]:
    center = 512.0
    outer = LOGO_OUTER_RADIUS
    radial_ratio = math.sin(math.pi / pearl_count)
    pearl_center_radius = outer / (1.0 + radial_ratio)
    pearl_radius = pearl_center_radius * radial_ratio
    inner = pearl_center_radius - pearl_radius
    centers = []
    for index in range(pearl_count):
        angle = math.radians(-90.0 + index * 360.0 / pearl_count)
        centers.append(
            (
                center + pearl_center_radius * math.cos(angle),
                center + pearl_center_radius * math.sin(angle),
            )
        )
    return outer, inner, pearl_radius, centers


def compound_path(pearl_count: int) -> str:
    outer, inner, pearl_radius, centers = variant_geometry(pearl_count)
    paths = [circle_path(512.0, 512.0, outer), circle_path(512.0, 512.0, inner)]
    paths.extend(circle_path(x, y, pearl_radius) for x, y in centers)
    return "\n    ".join(paths)


def logo_svg(pearl_count: int) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" role="img" aria-label="Inputia {pearl_count}-pearl logo">
  <title>Inputia {pearl_count}-pearl logo</title>
  <desc>A calm Fo Qing roundel with {pearl_count} pearl holes sharing tangent boundaries with the inner and outer ring.</desc>
  <path fill="{TEAL}" fill-rule="evenodd" d="
    {compound_path(pearl_count)}
  "/>
</svg>
"""


def app_icon_svg(pearl_count: int) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024" role="img" aria-label="Inputia app icon">
  <title>Inputia app icon</title>
  <rect x="64" y="64" width="896" height="896" rx="202" fill="{BACKGROUND}"/>
  <circle cx="512" cy="512" r="455" fill="none" stroke="{DARK}" stroke-width="10" opacity="0.16"/>
  <path fill="{TEAL}" fill-rule="evenodd" d="
    {compound_path(pearl_count)}
  "/>
</svg>
"""


def known_variant_ids() -> set[str]:
    return {variant_id for variant_id, _, _ in VARIANTS}


def variant_count(variant_id: str) -> int:
    for known_id, _title, count in VARIANTS:
        if known_id == variant_id:
            return count
    raise ValueError(f"unknown Inputia icon variant: {variant_id}")


def selected_variant(argument_variant: str | None, settings_path: Path) -> str:
    candidates = [
        argument_variant,
        os.environ.get("INPUTIA_MENU_ICON_VARIANT"),
        settings_variant(settings_path),
        DEFAULT_VARIANT,
    ]
    known = known_variant_ids()
    for candidate in candidates:
        if candidate in known:
            return candidate
    return DEFAULT_VARIANT


def settings_variant(settings_path: Path) -> str | None:
    try:
        data = json.loads(settings_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    value = data.get("menu_icon_variant")
    return value if isinstance(value, str) else None


def run(command: list[str]) -> None:
    subprocess.run(command, check=True)


def require_tool(name: str) -> str:
    path = shutil.which(name)
    if not path:
        raise SystemExit(f"missing required tool: {name}")
    return path


def write_variants(resources_dir: Path) -> None:
    variants_dir = resources_dir / "InputiaIconVariants"
    variants_dir.mkdir(parents=True, exist_ok=True)
    manifest = []
    for variant_id, title, count in VARIANTS:
        svg_path = variants_dir / f"{variant_id}.svg"
        svg_path.write_text(logo_svg(count), encoding="utf-8")
        manifest.append({"id": variant_id, "title": title, "pearls": count})
    (variants_dir / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def write_selected(resources_dir: Path, variant_id: str) -> None:
    rsvg = require_tool("rsvg-convert")
    iconutil = require_tool("iconutil")
    count = variant_count(variant_id)
    logo_path = resources_dir / "InputiaLogo.svg"
    app_icon_path = resources_dir / "InputiaAppIcon.svg"
    logo_path.write_text(logo_svg(count), encoding="utf-8")
    app_icon_path.write_text(app_icon_svg(count), encoding="utf-8")
    legacy_menu_icon_path = resources_dir / "inputia.pdf"
    menu_icon_path = resources_dir / "inputia-menu.pdf"
    run(
        [
            rsvg,
            "-f",
            "pdf",
            "--dpi-x",
            "72",
            "--dpi-y",
            "72",
            "-w",
            str(MENU_ICON_POINTS),
            "-h",
            str(MENU_ICON_POINTS),
            str(logo_path),
            "-o",
            str(legacy_menu_icon_path),
        ]
    )
    shutil.copy2(legacy_menu_icon_path, menu_icon_path)
    with tempfile.TemporaryDirectory(prefix="inputia-iconset-") as temp_dir:
        iconset_dir = Path(temp_dir) / "Inputia.iconset"
        iconset_dir.mkdir()
        specs = [
            (16, "icon_16x16.png"),
            (32, "icon_16x16@2x.png"),
            (32, "icon_32x32.png"),
            (64, "icon_32x32@2x.png"),
            (128, "icon_128x128.png"),
            (256, "icon_128x128@2x.png"),
            (256, "icon_256x256.png"),
            (512, "icon_256x256@2x.png"),
            (512, "icon_512x512.png"),
            (1024, "icon_512x512@2x.png"),
        ]
        for size, name in specs:
            run([rsvg, "-w", str(size), "-h", str(size), str(app_icon_path), "-o", str(iconset_dir / name)])
        run([iconutil, "-c", "icns", str(iconset_dir), "-o", str(resources_dir / "Inputia.icns")])


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Inputia icon resources.")
    parser.add_argument("--resources-dir", type=Path, default=DEFAULT_RESOURCES_DIR)
    parser.add_argument("--settings", type=Path, default=DEFAULT_SETTINGS_PATH)
    parser.add_argument("--variant", choices=sorted(known_variant_ids()))
    args = parser.parse_args()

    resources_dir = args.resources_dir
    resources_dir.mkdir(parents=True, exist_ok=True)
    variant = selected_variant(args.variant, args.settings)
    write_variants(resources_dir)
    write_selected(resources_dir, variant)
    print(f"inputiaIconVariant={variant}")


if __name__ == "__main__":
    main()
