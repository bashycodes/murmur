#!/usr/bin/env python3
"""Dated probe (2026-10-31): the AI-cleanup suite through a server's D36 `clean`.

Consumer: docs/findings/2026-10-31-server-model-setup.md (cleanup row).
Not a permanent tool.

Sends each case of the 2026-10-26 bench (the formatting-eval cases,
the 4 `regression` rows skipped like `v4c_cases.jsonl`) as one `clean`
request with the case's raw `input`, app category and style, and writes rows
in the bench runner's shape (`id`, `output`, `llm_ms`, `fallback`) so
`ai_cleanup_bench_2026_10_26/score.py LABEL=OUT.jsonl` scores them. The
server returns the model's answer only (no guard, no fallback rules, no
post-processing: the client does those under D36), so this is the
"raw-in, no guard" configuration, not the shipped session pipeline.

  server_clean_bench_2026_10_31.py --token-file FILE --out OUT.jsonl \
      [--url ws://127.0.0.1:7313] [--prompt v4c|v2] [--timeout-ms 8000]
"""
import argparse
import asyncio
import json
import os
import time

import websockets

here = os.path.dirname(os.path.abspath(__file__))
CASES = [os.path.join(here, "formatting_eval_2026_10_24", "cases.jsonl")]
STYLE = {"neutral": "default"}


async def run(a, token):
    rows = [json.loads(l) for f in CASES for l in open(f) if l.strip()]
    rows = [r for r in rows if r["category"] != "regression"]
    async with websockets.connect(a.url + "/v1/stream", subprotocols=["flow.v1"],
                                  additional_headers={"Authorization": f"Bearer {token}"},
                                  max_size=None) as ws:
        out = open(a.out, "w")
        await ws.send(json.dumps({"t": "hello", "proto_min": 1, "proto_max": 1,
                                  "client": "server-clean-bench 2026-10-31"}))
        ok = json.loads(await ws.recv())
        assert "clean" in ok.get("features", []), ok
        for i, c in enumerate(rows):
            t0 = time.monotonic()
            await ws.send(json.dumps({"t": "clean", "id": i, "req": {
                "text": c["input"], "level": "medium",
                "style": STYLE.get(c["style"], c["style"]),
                "ctx": {"app_id": c["app_id"], "app_category": c["app_category"],
                        "context_known": True},
                "timeout_ms": a.timeout_ms, "prompt": a.prompt}}))
            while True:
                m = json.loads(await ws.recv())
                if m.get("t") == "cleaned" and m.get("id") == i:
                    break
            out.write(json.dumps({
                "id": c["id"], "output": m.get("text") or c["input"],
                "llm_ms": m.get("llm_ms", 0), "queue_ms": m.get("queue_ms", 0),
                "rtt_ms": round((time.monotonic() - t0) * 1000),
                "fallback": m.get("error"), "model": m.get("model"),
            }, ensure_ascii=False) + "\n")
            out.flush()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--url", default="ws://127.0.0.1:7313")
    p.add_argument("--token-file", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--prompt", default="v4c")
    p.add_argument("--timeout-ms", type=int, default=8000)
    a = p.parse_args()
    token = open(a.token_file).read().strip()
    asyncio.run(run(a, token))


if __name__ == "__main__":
    main()
