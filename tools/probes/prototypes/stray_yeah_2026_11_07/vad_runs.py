"""Probe (stray "yeah", 2026-11-07): replicate murmur-core `vad::frame_flags`
/ `vad::detect` (20 ms frames, p10 floor, quiet-recording bar) in Python on
a slice of a WAV and print every speech run with its length and level, so
the click that ends a recording can be compared with real speech.

Usage: python3 -I vad_runs.py <wav> [start_s [end_s]]
Mirrors crates/murmur-core/src/vad.rs constants as of 2026-11-07.
"""

import math
import struct
import sys
import wave

RATE = 16000
FRAME = RATE // 50
ABS_MIN, ABS_CAP, FLOOR_FACTOR = 0.004, 0.015, 3.0
REL_FACTOR, QUIET_MIN, DIGITAL_SILENCE = 8.0, 0.0005, 1.5e-5
UNVOICED_FACTOR, ZCR_UNVOICED, MIN_RUN = 0.5, 0.3, 3


def p10(v):
    v = sorted(v)
    return v[len(v) // 10]


def main() -> None:
    path = sys.argv[1]
    with wave.open(path, "rb") as w:
        n = w.getnframes()
        pcm = [s / 32768 for s in struct.unpack(f"<{n}h", w.readframes(n))]
    a = int(float(sys.argv[2]) * RATE) if len(sys.argv) > 2 else 0
    b = int(float(sys.argv[3]) * RATE) if len(sys.argv) > 3 else len(pcm)
    pcm = pcm[a:b]
    stats = []
    for i in range(0, len(pcm), FRAME):
        f = pcm[i : i + FRAME]
        rms = math.sqrt(sum(x * x for x in f) / len(f))
        zc = sum(1 for x, y in zip(f, f[1:]) if (x >= 0) != (y >= 0))
        stats.append((rms, zc / max(len(f) - 1, 1)))
    rms = [s[0] for s in stats]
    floor = p10(rms)
    nz = [r for r in rms if r > DIGITAL_SILENCE]
    min_bar = min(max(p10(nz) * REL_FACTOR, QUIET_MIN), ABS_MIN) if nz else ABS_MIN
    voiced = min(max(floor * FLOOR_FACTOR, min_bar), ABS_CAP)
    unvoiced = max(voiced * UNVOICED_FACTOR, min_bar)
    flags = [r >= voiced or (r >= unvoiced and z >= ZCR_UNVOICED) for r, z in stats]
    db = lambda r: 20 * math.log10(max(r, 1e-9))
    print(f"floor {db(floor):.1f} dBFS, voiced bar {db(voiced):.1f}, unvoiced bar {db(unvoiced):.1f}")
    i = 0
    total = 0
    while i < len(flags):
        if not flags[i]:
            i += 1
            continue
        j = i
        while j < len(flags) and flags[j]:
            j += 1
        if j - i >= MIN_RUN:
            peak = max(rms[i:j])
            mean = math.sqrt(sum(r * r for r in rms[i:j]) / (j - i))
            total += j - i
            print(
                f"  run {(a + i * FRAME) / RATE:6.2f}-{(a + j * FRAME) / RATE:6.2f} s"
                f" {(j - i) * 20:5d} ms  rms {db(mean):6.1f}  peak frame {db(peak):6.1f} dBFS"
            )
        i = j
    print(f"speech {total * 20} ms")


if __name__ == "__main__":
    main()
