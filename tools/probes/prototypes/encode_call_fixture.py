#!/usr/bin/env python3
"""Encode the public-domain JFK speech clip as a call-recording-like m4a (D46 Calls fixture).

The JFK sample (1961 inaugural address, US public domain) ships with whisper.cpp as
samples/jfk.wav (16 kHz mono). Samsung call recordings are AAC-LC in an MP4 container, so the
fixture is 44.1 kHz stereo AAC-LC m4a: the phone has to decode, downmix and resample it.

Needs PyAV (`pip install av`, e.g. in a throwaway venv). No ffmpeg binary needed.

    python encode_call_fixture.py path/to/jfk.wav apps/flow-android/tools/fixtures/calls/jfk-call.m4a

Promoted to: nothing (the fixture it made is committed; rerun only to regenerate it).
"""
import sys
import wave

import av
import numpy as np


def main(src: str, dst: str) -> None:
    with wave.open(src, "rb") as w:
        assert (w.getnchannels(), w.getsampwidth()) == (1, 2), "16-bit mono WAV expected"
        rate = w.getframerate()
        pcm = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")

    out = av.open(dst, "w", format="mp4")
    stream = out.add_stream("aac", rate=44100, layout="stereo")
    stream.bit_rate = 48_000
    resampler = av.AudioResampler(format="fltp", layout="stereo", rate=44100)
    step = 1024
    for i in range(0, len(pcm), step):
        chunk = pcm[i:i + step].reshape(1, -1)
        frame = av.AudioFrame.from_ndarray(chunk, format="s16", layout="mono")
        frame.sample_rate = rate
        frame.pts = i
        frame.time_base = __import__("fractions").Fraction(1, rate)
        for f in resampler.resample(frame):
            for p in stream.encode(f):
                out.mux(p)
    for f in resampler.resample(None):
        for p in stream.encode(f):
            out.mux(p)
    for p in stream.encode(None):
        out.mux(p)
    out.close()

    # Check: decodes back to roughly the same length.
    back = av.open(dst)
    s = back.streams.audio[0]
    n = sum(f.samples for f in back.decode(s))
    print(f"{dst}: {s.codec_context.name} {s.rate} Hz {s.channels} ch, {n / s.rate:.2f} s (source {len(pcm) / rate:.2f} s)")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
