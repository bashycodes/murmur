#!/usr/bin/env python3
"""Write each history_audio recording of a history.sqlite copy as a WAV.

Probe for docs/findings/2026-11-07-random-capitals.md: builds a
murmur-replay corpus (`OUT/audio/<history id>.wav`, 16-bit mono) outside the
repo. Usage: export_wavs.py <history.sqlite> <OUT dir>
"""
import sqlite3
import sys
import wave
from pathlib import Path

con = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
out = Path(sys.argv[2]) / "audio"
out.mkdir(parents=True, exist_ok=True)
n = 0
for hid, rate, pcm in con.execute("select history_id, rate, pcm from history_audio"):
    with wave.open(str(out / f"{hid}.wav"), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm)
    n += 1
print(f"{n} wavs")
