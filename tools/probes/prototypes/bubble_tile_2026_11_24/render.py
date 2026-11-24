#!/usr/bin/env python3
"""Render the resting dictation bubble with the generated tile to bubble_tile.png.

Probe for the 2026-11-24 bubble tile change, not a permanent tool. It reads the
pathData and fill of the generated VectorDrawable
(apps/murmur-android/app/src/main/res/drawable/ic_bubble_tile.xml, written by
tools/brand/gen-icons.py; VectorDrawable path syntax is SVG's), and draws it the
way BubbleView does: on the vast-950 pill, a 40 dp tile in the 56 dp bubble and a
26 dp tile in the 36 dp minimised circle, at 4 px per dp, over a light and a dark
app background. Also prints the WCAG contrast ratios of the chosen colours.

Needs Playwright's headless Chromium (as gen-icons.py does).
"""
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
DRAWABLE = ROOT / "apps/murmur-android/app/src/main/res/drawable/ic_bubble_tile.xml"
OUT = Path(__file__).resolve().parent / "bubble_tile.png"
A = "{http://schemas.android.com/apk/res/android}"
PILL = "#1A1A1A"  # BubbleColors.surface (vast-950)
PX = 4  # px per dp


def luminance(hex_colour):
    v = [int(hex_colour[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    v = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in v]
    return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]


def contrast(a, b):
    lo, hi = sorted((luminance(a), luminance(b)))
    return (hi + 0.05) / (lo + 0.05)


def tile_svg(path_data, fill, x, y, edge):
    s = edge / 256
    return (f'<path transform="translate({x} {y}) scale({s})" fill="{fill}" '
            f'fill-rule="evenodd" d="{path_data}"/>')


def bubbles(path_data, fill, bg, ox, oy):
    """The full 56 dp bubble and the 36 dp minimised circle side by side on bg, at (ox, oy) dp."""
    parts = [f'<rect x="{ox}" y="{oy}" width="150" height="80" fill="{bg}"/>']
    fx, fy = ox + 16, oy + 12  # full bubble: 56 dp circle
    parts.append(f'<circle cx="{fx + 28}" cy="{fy + 28}" r="28" fill="{PILL}"/>')
    parts.append(tile_svg(path_data, fill, fx + 8, fy + 8, 40))
    mx, my = ox + 96, oy + 22  # minimised: 36 dp circle
    parts.append(f'<circle cx="{mx + 18}" cy="{my + 18}" r="18" fill="{PILL}"/>')
    parts.append(tile_svg(path_data, fill, mx + 5, my + 5, 26))
    return "".join(parts)


def main():
    path = ET.parse(DRAWABLE).getroot().find("path")
    path_data = path.get(f"{A}pathData")
    fill = "#" + path.get(f"{A}fillColor")[3:]
    print(f"tile {fill} vs pill {PILL}: {contrast(fill, PILL):.2f}:1 "
          f"(the knocked-out mark shows the pill, so mark vs tile is the same ratio)")
    w, h = 300, 80
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w * PX}" height="{h * PX}" '
           f'viewBox="0 0 {w} {h}">{bubbles(path_data, fill, "#F5F5F5", 0, 0)}'
           f'{bubbles(path_data, fill, "#2B2B2B", 150, 0)}</svg>')
    from playwright.sync_api import sync_playwright

    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        page = browser.new_page(viewport={"width": w * PX, "height": h * PX})
        page.set_content(f'<html><body style="margin:0">{svg}</body></html>')
        OUT.write_bytes(page.screenshot())
        browser.close()
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
