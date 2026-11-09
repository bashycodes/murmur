#!/usr/bin/env python3
"""Generate the app/tray icons (icons/icon.png, icons/32x32.png, icons/icon.ico).

Pure stdlib (zlib/struct) so no image library is needed. Draws a filled
violet circle with three white rounded "level bars". Re-run after changing
the design; outputs are committed so the Rust build needs no Python.

    python3 tools/gen_icons.py
"""

import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "icons")

BG = (124, 92, 255)
FG = (255, 255, 255)


def render(size):
    px = []
    c = (size - 1) / 2.0
    r = size * 0.48
    bars = [(-0.22, 0.20), (0.0, 0.34), (0.22, 0.26)]  # (x offset, half-height) in units of size
    bw = size * 0.065
    for y in range(size):
        row = []
        for x in range(size):
            # 4x supersampling for smooth edges
            cov_bg = 0
            cov_fg = 0
            for sy in range(4):
                for sx in range(4):
                    fx = x + (sx + 0.5) / 4
                    fy = y + (sy + 0.5) / 4
                    dx, dy = fx - c - 0.5, fy - c - 0.5
                    if dx * dx + dy * dy <= r * r:
                        cov_bg += 1
                        for bx, bh in bars:
                            cx = c + 0.5 + bx * size
                            hh = bh * size
                            # rounded bar = capsule
                            qy = max(abs(fy - c - 0.5) - (hh - bw), 0.0)
                            qx = fx - cx
                            if qx * qx + qy * qy <= bw * bw:
                                cov_fg += 1
                                break
            a = cov_bg / 16.0
            t = cov_fg / max(cov_bg, 1)
            col = tuple(int(BG[i] * (1 - t) + FG[i] * t) for i in range(3))
            row.append(col + (int(a * 255),))
        px.append(row)
    return px


def png(px):
    h = len(px)
    w = len(px[0])
    raw = b"".join(b"\x00" + bytes(v for p in row for v in p) for row in px)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def ico(pngs):
    n = len(pngs)
    head = struct.pack("<HHH", 0, 1, n)
    offset = 6 + 16 * n
    entries = b""
    data = b""
    for size, blob in pngs:
        s = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", s, s, 0, 0, 1, 32, len(blob), offset + len(data))
        data += blob
    return head + entries + data


def main():
    os.makedirs(OUT, exist_ok=True)
    big = png(render(256))
    small = png(render(32))
    with open(os.path.join(OUT, "icon.png"), "wb") as f:
        f.write(big)
    with open(os.path.join(OUT, "32x32.png"), "wb") as f:
        f.write(small)
    with open(os.path.join(OUT, "icon.ico"), "wb") as f:
        f.write(ico([(32, small), (256, big)]))
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
