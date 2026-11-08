"""Probe (stray "yeah", 2026-11-07): dump retry audio rows from a COPY of
the Windows `history.sqlite` to 16 kHz mono WAV files and print a coarse
energy profile of each recording's last seconds.

Usage: python3 -I dump_audio.py <history.sqlite copy> <out dir> <row id>...

Evidence stays in the gitignored/out-of-repo eval-private folder; this
script never prints transcript text.
"""

import math
import sqlite3
import struct
import sys
import wave


def main() -> None:
    db, out, ids = sys.argv[1], sys.argv[2], [int(i) for i in sys.argv[3:]]
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    for hid in ids:
        rate, pcm = con.execute(
            "select rate, pcm from history_audio where history_id = ?", (hid,)
        ).fetchone()
        path = f"{out}/row{hid}.wav"
        with wave.open(path, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(pcm)
        n = len(pcm) // 2
        samples = struct.unpack(f"<{n}h", pcm)
        print(f"row {hid}: {n / rate:.2f} s -> {path}")
        win = rate // 10  # 100 ms windows
        tail_s = 6
        start = max(0, n - tail_s * rate)
        for i in range(start, n, win):
            chunk = samples[i : i + win]
            rms = math.sqrt(sum(s * s for s in chunk) / max(1, len(chunk)))
            db_fs = 20 * math.log10(max(rms, 1) / 32768)
            bar = "#" * max(0, int((db_fs + 70) / 2))
            print(f"  t={i / rate:6.1f}s {db_fs:6.1f} dBFS {bar}")


if __name__ == "__main__":
    main()
