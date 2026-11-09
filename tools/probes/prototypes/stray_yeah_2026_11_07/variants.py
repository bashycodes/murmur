"""Probe (stray "yeah", 2026-11-07): make WAV variants of one recording to
find which audio makes the ASR emit the extra word.

Usage: python3 -I variants.py <in.wav> <out prefix> <split_s>

Writes:
  <prefix>.head.wav   audio before split_s (the speech)
  <prefix>.tail.wav   audio from split_s to the end (the trailing pause +
                      whatever ends the recording)
  <prefix>.muted.wav  the whole recording with everything after split_s
                      replaced by digital silence
"""

import sys
import wave


def main() -> None:
    src, prefix, split_s = sys.argv[1], sys.argv[2], float(sys.argv[3])
    with wave.open(src, "rb") as w:
        rate = w.getframerate()
        pcm = w.readframes(w.getnframes())
    cut = int(split_s * rate) * 2
    for name, data in (
        ("head", pcm[:cut]),
        ("tail", pcm[cut:]),
        ("muted", pcm[:cut] + bytes(len(pcm) - cut)),
    ):
        with wave.open(f"{prefix}.{name}.wav", "wb") as o:
            o.setnchannels(1)
            o.setsampwidth(2)
            o.setframerate(rate)
            o.writeframes(data)
        print(f"{prefix}.{name}.wav {len(data) / 2 / rate:.2f} s")


if __name__ == "__main__":
    main()
