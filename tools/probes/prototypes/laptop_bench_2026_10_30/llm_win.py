#!/usr/bin/env python3
"""AI-cleanup (prompt v4c, raw transcript) latency + answers on the Windows PC via llama-server.exe.
Dated probe, 2026-10-30; runs ON the PC.

For each --run BACKEND:MODEL[:THREADS]: start the official llama.cpp release llama-server.exe
(bin/llama-<backend>/, -c 4096, -np 1, -ngl 99 unless backend is cpu) at below-normal priority,
prefill the static v4c prefix once (the app does this at model load), then send each case of
cases.jsonl exactly as murmur-core renders it (build_prompt_versioned(..., PromptScope::FULL, V4c):
system + 24 few-shot turns + "APP: <app_id> | CATEGORY: <name>\n[STYLE: <casual>\n]TRANSCRIPT: «…»"),
greedy, max_tokens = 2 x transcript tokens + 32 (murmur-llm's rule), cache_prompt on.
Writes one row per case in the ai-cleanup-bench `--from` shape ({id, llm_raw, error, llm_ms} + llama
timings) so `ai-cleanup-bench --mode regard` applies the product guards + fallback rules on Linux and
score.py scores it. llm_ms = request wall time on the PC (no network).

usage: python llm_win.py --bin BIN --models MODELS --prompt prompt_v4c.json --cases cases.jsonl \
          --run vulkan:qwen2.5-1.5b-instruct-q4_k_m.gguf --run cpu:...:8 --outdir results
"""
import argparse, json, pathlib, subprocess, time, urllib.request

BELOW_NORMAL = 0x00004000
CASUAL = ('casual: relaxed chat punctuation, no final period (keep "?" and "!"); keep sentence capitals '
          'and every word except fillers')
CAT = {"email": "email", "work_chat": "work chat", "personal_chat": "personal chat", "ai_prompt": "AI prompt",
       "terminal": "terminal", "document": "document", "other": "other"}
# sessions/prod.json of the 2026-10-26 bench: personal/work casual, email/other formal
CASUAL_CATS = {"personal_chat", "work_chat"}


def post(port, path, body, timeout=300):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def wait_ready(port, proc, timeout=300):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if proc.poll() is not None:
            raise RuntimeError("llama-server exited")
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                if r.status == 200:
                    return time.time() - t0
        except Exception:
            pass
        time.sleep(0.25)
    raise TimeoutError


def user(app, cat, casual, transcript):
    return f"APP: {app} | CATEGORY: {cat}\n" + (f"STYLE: {CASUAL}\n" if casual else "") + f"TRANSCRIPT: «{transcript}»"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", type=pathlib.Path, required=True)
    ap.add_argument("--models", type=pathlib.Path, required=True)
    ap.add_argument("--prompt", required=True)
    ap.add_argument("--cases", required=True)
    ap.add_argument("--run", action="append", required=True)
    ap.add_argument("--outdir", type=pathlib.Path, required=True)
    ap.add_argument("--port", type=int, default=8093)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--server-args", default="", help="extra llama-server args, e.g. --swa-full (Gemma 3 cache reuse)")
    ap.add_argument("--tag", default="")
    a = ap.parse_args()
    pf = json.load(open(a.prompt, encoding="utf-8"))
    prefix = [{"role": "system", "content": pf["system"]}]
    for e in pf["examples"]:
        prefix += [{"role": "user", "content": user(e[0], e[1], len(e) > 4 and e[4] == "casual", e[2])},
                   {"role": "assistant", "content": e[3]}]
    cases = [json.loads(l) for l in open(a.cases, encoding="utf-8") if l.strip()]
    if a.limit:
        cases = cases[: a.limit]
    a.outdir.mkdir(exist_ok=True)
    for spec in a.run:
        backend, model, *th = spec.split(":")
        th = int(th[0]) if th else 4
        cmd = [str(a.bin / f"llama-{backend}" / "llama-server.exe"), "-m", str(a.models / model), "-c", "4096",
               "-np", "1", "--port", str(a.port), "-t", str(th), "--no-webui"]
        cmd += ["-ngl", "0"] if backend == "cpu" else ["-ngl", "99"]
        cmd += a.server_args.split()
        log = open(a.outdir / f"server-{backend}-{model}-t{th}.log", "w")
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, creationflags=BELOW_NORMAL)
        label = f"{backend}-{pathlib.Path(model).stem}-t{th}{a.tag}"
        out = a.outdir / f"v4c-{label}-{time.strftime('%Y%m%d-%H%M%S')}.jsonl"
        try:
            load_s = wait_ready(a.port, proc)
            t0 = time.perf_counter()
            post(a.port, "/v1/chat/completions", {"messages": prefix + [{"role": "user", "content": user(
                "Notes", "other", False, "Warm up the cache.")}], "temperature": 0, "max_tokens": 16, "cache_prompt": True})
            warm_ms = (time.perf_counter() - t0) * 1000
            rows = []
            with open(out, "w", encoding="utf-8") as fo:
                for c in cases:
                    n_in = len(post(a.port, "/tokenize", {"content": c["input"]})["tokens"])
                    msgs = prefix + [{"role": "user", "content": user(c["app_id"] or "the focused app", CAT[c["app_category"]],
                                                                       c["app_category"] in CASUAL_CATS, c["input"])}]
                    body = {"messages": msgs, "temperature": 0, "top_k": 1, "cache_prompt": True,
                            "max_tokens": 2 * n_in + 32}
                    t0 = time.perf_counter()
                    err, raw, t = None, None, {}
                    try:
                        r = post(a.port, "/v1/chat/completions", body)
                        raw, t = r["choices"][0]["message"]["content"], r.get("timings", {})
                    except Exception as e:  # noqa: BLE001
                        err = f"llm_error: {e}"
                    ms = round((time.perf_counter() - t0) * 1000)
                    row = {"id": c["id"], "llm_raw": raw, "error": err, "llm_ms": ms,
                           "prompt_n": t.get("prompt_n"), "prompt_ms": t.get("prompt_ms"),
                           "gen_n": t.get("predicted_n"), "gen_ms": t.get("predicted_ms"),
                           "in_words": len(c["input"].split())}
                    rows.append(row)
                    fo.write(json.dumps(row, ensure_ascii=False) + "\n"); fo.flush()
            ms = sorted(r["llm_ms"] for r in rows)
            gen = [r for r in rows if r["gen_n"] and r["gen_n"] >= 10]
            print(json.dumps({"label": label, "load_s": round(load_s, 1), "warm_ms": round(warm_ms),
                              "p50": ms[len(ms) // 2], "p90": ms[int(0.9 * len(ms)) - 1], "max": ms[-1],
                              "over1500": sum(x > 1500 for x in ms), "n": len(ms),
                              "gen_tps": round(sum(r["gen_n"] for r in gen) / sum(r["gen_ms"] for r in gen) * 1000, 1) if gen else None,
                              "pp_tps": round(sum(r["prompt_n"] for r in rows) / max(1e-9, sum(r["prompt_ms"] for r in rows)) * 1000),
                              "out": out.name}), flush=True)
        finally:
            proc.terminate(); proc.wait()
            log.close()


if __name__ == "__main__":
    main()
