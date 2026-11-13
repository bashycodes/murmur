#!/usr/bin/env python3
"""Prefill-speed probe: one long system prompt (cached after the first
call), then short user turns, against the llama-server at argv[1]. Prints
the server's timings per call. Separates "batch prefill is slow" from
"cache reuse is slow" on the iGPU."""
import json
import sys
import time
import urllib.request

url = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:18091"
SYS = "You clean up dictated text. " * 300
A = "first message about the meeting on tuesday with the design team and some more words to make it longer, about sixty tokens or so in total for this test"
B = "second different message: please buy milk, eggs, bread and also call the plumber about the kitchen sink before friday afternoon if you can thanks"


def req(u, sys_msg=SYS):
    d = json.dumps({"messages": [{"role": "system", "content": sys_msg}, {"role": "user", "content": u}],
                    "max_tokens": 8, "temperature": 0}).encode()
    t = time.time()
    v = json.load(urllib.request.urlopen(url + "/v1/chat/completions", d))
    tm = v["timings"]
    print(f"{time.time() - t:6.2f}s", {k: tm[k] for k in ("cache_n", "prompt_n", "prompt_ms", "predicted_n", "predicted_ms")})


for u in (A, A, B, A):
    req(u)
req(A, sys_msg="Short system.")
req(B, sys_msg="Short system.")
