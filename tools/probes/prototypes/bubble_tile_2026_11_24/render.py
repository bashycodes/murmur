#!/usr/bin/env python3
"""Render the resting dictation bubble (the generated tile) to bubble_tile.png.

Probe for the 2026-11-24 bubble tile change, not a permanent tool. It reads the
paths and fills of the generated VectorDrawable
(apps/murmur-android/app/src/main/res/drawable/ic_bubble_tile.xml, written by
tools/brand/gen-icons.py; VectorDrawable path syntax is SVG's) and draws them the
way BubbleView does: the tile over the whole bubble (56 dp, and the 36 dp shrunk
form) with the pill's shadow, the Ready bubble with a copy button (the pill
continues right of the tile), and a recording pill for comparison. One row per
app background: light, dark, brand violet and busy text. 4 px per dp. Also prints
the WCAG contrast ratios of the colours involved.

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
ON_PILL = "#FCFCFB"  # BubbleColors.onSurface (sand-50)
PX = 4  # px per dp
ROW_H, W = 84, 460
BACKGROUNDS = [("light", "#FFFFFF"), ("dark", "#121212"), ("violet", "#7C3AED"), ("text", None)]
SHADOW = 'filter="url(#sh)"'


def luminance(hex_colour):
    v = [int(hex_colour[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    v = [c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in v]
    return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]


def contrast(a, b):
    lo, hi = sorted((luminance(a), luminance(b)))
    return (hi + 0.05) / (lo + 0.05)


def tile(paths, x, y, edge):
    s = edge / 256
    # The pill-coloured shadow caster has the tile's shape, as BubbleView's drawRoundRect: corners of
    # the outer path's rx ("M<rx>,0h…", in the 256-unit viewBox).
    rx = float(paths[0][1][1:].split(",")[0])
    body = "".join(f'<path fill="{f}" d="{d}"/>' for f, d in paths)
    return (f'<rect {SHADOW} x="{x}" y="{y}" width="{edge}" height="{edge}" rx="{edge * rx / 256}" fill="{PILL}"/>'
            f'<g transform="translate({x} {y}) scale({s})">{body}</g>')


def background(colour, y):
    if colour:
        return f'<rect x="0" y="{y}" width="{W}" height="{ROW_H}" fill="{colour}"/>'
    words = ("The quick brown fox jumps over the lazy dog while the meeting notes scroll by, "
             "links, numbers 1234, and bold headings fill the screen. ") * 3
    lines = "".join(f'<text x="4" y="{y + 9 + i * 9}" font-size="8" font-family="sans-serif" '
                    f'fill="{"#1A0DAB" if i % 3 == 1 else "#202124"}">{words[i * 17:i * 17 + 140]}</text>'
                    for i in range(9))
    return f'<rect x="0" y="{y}" width="{W}" height="{ROW_H}" fill="#FAFAF7"/>{lines}'


def row(paths, colour, y):
    parts = [background(colour, y)]
    top = y + 14
    parts.append(tile(paths, 16, top, 56))  # Idle / Ready: the bubble is the tile
    parts.append(tile(paths, 96, top + 10, 36))  # shrunk, "Shrink to a dot" off
    # Ready with the copy button: the pill runs under the tile's right side to the ⧉ slot.
    parts.append(f'<rect {SHADOW} x="156" y="{top}" width="120" height="56" rx="28" fill="{PILL}"/>')
    parts.append(tile(paths, 156, top, 56))
    parts.append(f'<text x="{156 + 64 + 28}" y="{top + 36}" font-size="22" text-anchor="middle" '
                 f'font-weight="bold" fill="{ON_PILL}">⧉</text>')
    # Recording (tap mode), unchanged: the round pill, for the shape change.
    parts.append(f'<rect {SHADOW} x="296" y="{top}" width="150" height="56" rx="28" fill="{PILL}"/>')
    for i, hh in enumerate((10, 22, 30, 18, 26, 14, 8)):
        parts.append(f'<rect x="{350 + i * 7}" y="{top + 28 - hh / 2}" width="3" height="{hh}" rx="1.5" '
                     f'fill="{ON_PILL}"/>')
    return "".join(parts)


def main():
    paths = [("#" + p.get(f"{A}fillColor")[3:], p.get(f"{A}pathData"))
             for p in ET.parse(DRAWABLE).getroot().findall("path")]
    (ink, _), (fill, _), _ = paths
    print(f"mark {ink} on tile {fill}: {contrast(ink, fill):.2f}:1")
    for name, bg in BACKGROUNDS:
        if bg:
            print(f"on {name} {bg}: tile {contrast(fill, bg):.2f}:1, edge {contrast(ink, bg):.2f}:1")
    h = ROW_H * len(BACKGROUNDS)
    rows = "".join(row(paths, c, i * ROW_H) for i, (_, c) in enumerate(BACKGROUNDS))
    svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{W * PX}" height="{h * PX}" viewBox="0 0 {W} {h}">'
           f'<defs><filter id="sh" x="-20%" y="-20%" width="140%" height="150%">'
           f'<feDropShadow dx="0" dy="2" stdDeviation="1.5" flood-color="#1A1A1A" flood-opacity="0.3"/>'
           f'</filter></defs>{rows}</svg>')
    from playwright.sync_api import sync_playwright

    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        page = browser.new_page(viewport={"width": W * PX, "height": h * PX})
        page.set_content(f'<html><body style="margin:0">{svg}</body></html>')
        OUT.write_bytes(page.screenshot())
        browser.close()
    print(f"wrote {OUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
