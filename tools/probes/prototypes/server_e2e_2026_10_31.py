#!/usr/bin/env python3
"""Dated probe (2026-10-31): end-to-end check of a running murmur-server.

Consumer: docs/findings/2026-10-31-server-model-setup.md (verification and
latency tables). Not a permanent tool.

Speaks the murmur.v1 WebSocket protocol (docs/protocol.md) directly: hello,
then one session per WAV (start -> PCM frames at real time or faster ->
commit -> result), then optional D36 `clean` requests. Prints one JSON line
per result with timings, route, language, status and text. The token is read
from a file and only sent in the Authorization header; it is never printed.

  server_e2e_2026_10_31.py --url ws://127.0.0.1:7313 --token-file FILE \
      [--wav PATH[:LANGS]]... [--clean TEXT]... [--clean-prompt v2|v4c] \
      [--realtime] [--health]

LANGS is a comma list for the session's `languages` (e.g. `he`, `en`);
empty (`PATH:`) or omitted = auto-detect. WAVs may be 16-bit PCM or 32-bit
float, any channel count; they must be 16 kHz (no resampler here).
"""
import argparse
import asyncio
import json
import struct
import sys
import time
import urllib.request

import websockets

RATE = 16000
FRAME = 1600  # recommended frame (100 ms)


def read_wav(path):
    """16 kHz mono int16 samples from a PCM16 or float32 WAV."""
    data = open(path, "rb").read()
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise SystemExit(f"{path}: not a WAV")
    pos, fmt, pcm = 12, None, None
    while pos + 8 <= len(data):
        cid, size = data[pos:pos + 4], struct.unpack("<I", data[pos + 4:pos + 8])[0]
        body = data[pos + 8:pos + 8 + size]
        if cid == b"fmt ":
            tag, ch, rate, _, _, bits = struct.unpack("<HHIIHH", body[:16])
            if tag == 0xFFFE:  # extensible: real tag in the subformat GUID
                tag = struct.unpack("<H", body[24:26])[0]
            fmt = (tag, ch, rate, bits)
        elif cid == b"data":
            pcm = body
        pos += 8 + size + (size & 1)
    tag, ch, rate, bits = fmt
    if rate != RATE:
        raise SystemExit(f"{path}: {rate} Hz, need {RATE}")
    if tag == 1 and bits == 16:
        s = struct.unpack(f"<{len(pcm) // 2}h", pcm[: len(pcm) // 2 * 2])
        s = [x / 32768.0 for x in s]
    elif tag == 3 and bits == 32:
        s = struct.unpack(f"<{len(pcm) // 4}f", pcm[: len(pcm) // 4 * 4])
    else:
        raise SystemExit(f"{path}: unsupported format tag={tag} bits={bits}")
    mono = [sum(s[i:i + ch]) / ch for i in range(0, len(s) - ch + 1, ch)]
    return [max(-32768, min(32767, int(round(x * 32767)))) for x in mono]


def health(http_url, token):
    req = urllib.request.Request(http_url + "/v1/health",
                                 headers={"Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read())


async def run(a, token):
    hdr = {"Authorization": f"Bearer {token}"}
    async with websockets.connect(a.url + "/v1/stream", subprotocols=["murmur.v1"],
                                  additional_headers=hdr, max_size=None) as ws:
        await ws.send(json.dumps({"t": "hello", "proto_min": 1, "proto_max": 1,
                                  "client": "server-e2e-probe 2026-10-31"}))
        ok = json.loads(await ws.recv())
        print(json.dumps({"hello_ok": ok}, ensure_ascii=False), flush=True)
        sid = 0
        for spec in a.wav:
            path, _, langs = spec.partition(":")
            languages = [x for x in langs.split(",") if x]
            pcm = read_wav(path)
            sid += 1
            await ws.send(json.dumps({"t": "start", "sid": sid, "config": {
                "languages": languages, "cleanup_level": "medium",
                "llm_timeout_ms": a.llm_timeout_ms, "platform": "windows"}}))
            for i in range(0, len(pcm), FRAME):
                chunk = pcm[i:i + FRAME]
                await ws.send(struct.pack(f"<{len(chunk)}h", *chunk))
                if a.realtime:
                    await asyncio.sleep(len(chunk) / RATE)
            t0 = time.monotonic()
            await ws.send(json.dumps({"t": "commit", "sid": sid}))
            while True:
                m = json.loads(await ws.recv())
                if m.get("t") in ("result", "error") and m.get("sid") in (sid, None):
                    break
            commit_ms = round((time.monotonic() - t0) * 1000)
            r = m.get("result", {})
            print(json.dumps({
                "wav": path.rsplit("/", 1)[-1], "languages": languages,
                "commit_to_result_ms": commit_ms, "status": r.get("status"),
                "language": r.get("language"), "route": r.get("route"),
                "cleanup_route": r.get("cleanup_route"),
                "fallback_reason": r.get("fallback_reason"),
                "timings": r.get("timings"), "asr_text": r.get("asr_text"),
                "final_text": r.get("final_text"),
                "server_timings": m.get("server_timings"),
                "error": m if m.get("t") == "error" else r.get("error"),
            }, ensure_ascii=False), flush=True)
        for i, text in enumerate(a.clean):
            t0 = time.monotonic()
            await ws.send(json.dumps({"t": "clean", "id": 100 + i, "req": {
                "text": text, "level": "medium", "style": "default",
                "ctx": {"app_category": "other"}, "timeout_ms": a.clean_timeout_ms,
                "prompt": a.clean_prompt}}))
            while True:
                m = json.loads(await ws.recv())
                if m.get("t") in ("cleaned", "error"):
                    break
            m["rtt_ms"] = round((time.monotonic() - t0) * 1000)
            m["input"] = text
            print(json.dumps(m, ensure_ascii=False), flush=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--url", default="ws://127.0.0.1:7313")
    p.add_argument("--token-file", required=True)
    p.add_argument("--wav", action="append", default=[])
    p.add_argument("--clean", action="append", default=[])
    p.add_argument("--clean-prompt", default="v2")
    p.add_argument("--clean-timeout-ms", type=int, default=5000)
    p.add_argument("--llm-timeout-ms", type=int, default=3000)
    p.add_argument("--realtime", action="store_true")
    p.add_argument("--health", action="store_true")
    a = p.parse_args()
    token = open(a.token_file).read().strip()
    if a.health:
        print(json.dumps({"health": health(a.url.replace("ws", "http", 1), token)}), flush=True)
    if a.wav or a.clean:
        asyncio.run(run(a, token))


if __name__ == "__main__":
    sys.exit(main())
