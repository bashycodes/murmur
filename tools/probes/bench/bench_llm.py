#!/usr/bin/env python3
"""Local LLM dictation-cleanup benchmark (llama.cpp llama-server, CPU).

For each GGUF model: start llama-server (model stays resident, like the app would keep it), then for
every case in cleanup_cases.json send the 03 §7.2-derived system prompt + §7.2 user message via
/v1/chat/completions (temperature 0, max_tokens = 2*input_tokens+32 capped at 256, model's own chat
template). The system prompt is app-agnostic so its KV prefix is reused across requests
(cache_prompt), which is how the clone should run it. Before the cases, one "cold-prompt" request with
cache_prompt=false measures full prompt processing.

Quality per case:
  * exact  — output == expected after whitespace/quote normalisation
  * sim    — 1 - char-level Levenshtein / max(len) (1.0 = identical)
  * guard  — would the §7.1 guardrail reject it (word edit distance raw->out / words(raw) > 0.6, or
             empty output for non-empty expected)? A rejected output means raw ASR is pasted instead.

Writes $CACHE/results/llm-<ts>.jsonl. Usage:
  python3 -I tools/probes/bench/bench_llm.py [--models a.gguf,b.gguf] [--threads 8] [--port 8091]
"""
import argparse, json, os, pathlib, re, subprocess, sys, time, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from wer import edit_distance  # noqa: E402

CACHE = pathlib.Path(os.environ.get("CACHE", pathlib.Path.home() / "Claude/research/wispr/.cache"))
SERVER = CACHE / "llama.cpp/build/bin/llama-server"
DEFAULT_MODELS = ["qwen2.5-0.5b-instruct-q4_k_m.gguf", "qwen2.5-1.5b-instruct-q4_k_m.gguf",
                  "qwen2.5-3b-instruct-q4_k_m.gguf", "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
                  "Llama-3.2-3B-Instruct-Q4_K_M.gguf"]


def post(port, path, body, timeout=600):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read())


def wait_ready(port, proc, timeout=180):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if proc.poll() is not None:
            raise RuntimeError("server exited")
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                if r.status == 200:
                    return time.time() - t0
        except Exception:
            pass
        time.sleep(0.25)
    raise TimeoutError


def user_msg(c):
    return (f"APP: {c['app']} | CATEGORY: {c['category']} | BUCKET: {c['category']} | LANGUAGE: en\n"
            f"DICTIONARY: \nTEXT BEFORE CURSOR: «»\nSELECTED TEXT (will be replaced): «»\n"
            f"TEXT AFTER CURSOR: «»\nTRANSCRIPT: «{c['input']}»")


def norm(s):
    s = s.strip().strip("«»\"'").strip()
    s = re.sub(r"[ \t]+", " ", s)
    s = re.sub(r"\s*\n\s*", "\n", s)
    return s.replace("’", "'")


def char_sim(a, b):
    if not a and not b:
        return 1.0
    return 1 - edit_distance(list(a), list(b)) / max(len(a), len(b))


def guard_reject(raw, out, expected):
    if not out.strip():
        return bool(expected.strip())
    rw, ow = raw.lower().split(), re.sub(r"<[^>]+>", " ", out).lower().split()
    return edit_distance(rw, ow) / max(len(rw), 1) > 0.6


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", default=",".join(DEFAULT_MODELS))
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--port", type=int, default=8091)
    a = ap.parse_args()
    system = (HERE / "cleanup_system_prompt.txt").read_text()
    cases = json.loads((HERE / "cleanup_cases.json").read_text())["cases"]
    res_dir = CACHE / "results"; res_dir.mkdir(exist_ok=True)
    out = res_dir / f"llm-{time.strftime('%Y%m%d-%H%M%S')}-t{a.threads}.jsonl"
    summary = []
    with open(out, "w") as fo:
        for mname in a.models.split(","):
            mpath = CACHE / "models" / mname
            proc = subprocess.Popen([str(SERVER), "-m", str(mpath), "-t", str(a.threads), "-c", "4096",
                                     "--port", str(a.port), "-np", "1", "--no-webui"],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                load_s = wait_ready(a.port, proc)
                def ask(c, cache=True, max_tokens=None, ignore_eos=False):
                    msgs = [{"role": "system", "content": system}, {"role": "user", "content": user_msg(c)}]
                    n_in = len(c["input"].split()) * 2  # ~tokens; cheap estimate for the max_tokens rule
                    body = {"messages": msgs, "temperature": 0, "cache_prompt": cache,
                            "max_tokens": max_tokens or min(2 * n_in + 32, 256)}
                    if ignore_eos:
                        body["ignore_eos"] = True
                    t0 = time.perf_counter()
                    r = post(a.port, "/v1/chat/completions", body)
                    return r, (time.perf_counter() - t0) * 1000
                # cold prompt + fixed 60-token generation: comparable latency across models
                r, wall = ask(cases[0], cache=False, max_tokens=60, ignore_eos=True)
                t = r["timings"]
                fixed = {"prompt_n": t["prompt_n"], "prompt_ms": t["prompt_ms"], "gen_n": t["predicted_n"],
                         "gen_ms": t["predicted_ms"], "wall_ms": wall}
                fo.write(json.dumps({"kind": "fixed60", "model": mname, "load_s": load_s, **fixed}) + "\n")
                # warm the system-prompt cache once
                ask(cases[1])
                rows = []
                for c in cases:
                    r, wall = ask(c)
                    t = r["timings"]
                    o = norm(r["choices"][0]["message"]["content"])
                    e = norm(c["expected"])
                    rec = {"kind": "case", "model": mname, "id": c["id"], "tags": c["tags"], "input": c["input"],
                           "expected": c["expected"], "output": o, "exact": o == e, "sim": char_sim(o, e),
                           "sim_raw": char_sim(norm(c["input"]), e),
                           "guard_reject": guard_reject(c["input"], o, c["expected"]),
                           "prompt_n": t["prompt_n"], "prompt_ms": t["prompt_ms"], "gen_n": t["predicted_n"],
                           "gen_ms": t["predicted_ms"], "wall_ms": wall}
                    rows.append(rec)
                    fo.write(json.dumps(rec, ensure_ascii=False) + "\n"); fo.flush()
                n = len(rows)
                srow = {"model": mname, "threads": a.threads, "load_s": load_s,
                        "fixed60_prompt_n": fixed["prompt_n"], "fixed60_prompt_ms": fixed["prompt_ms"],
                        "fixed60_gen_tps": fixed["gen_n"] / fixed["gen_ms"] * 1000,
                        "fixed60_pp_tps": fixed["prompt_n"] / fixed["prompt_ms"] * 1000,
                        "fixed60_wall_ms": fixed["wall_ms"],
                        "exact": sum(r["exact"] for r in rows), "n": n,
                        "mean_sim": sum(r["sim"] for r in rows) / n,
                        "mean_sim_raw": sum(r["sim_raw"] for r in rows) / n,
                        "better": sum(r["sim"] > r["sim_raw"] + 0.02 for r in rows),
                        "worse": sum(r["sim"] < r["sim_raw"] - 0.02 for r in rows),
                        "guard_rejects": sum(r["guard_reject"] for r in rows),
                        "mean_case_wall_ms": sum(r["wall_ms"] for r in rows) / n,
                        "p90_case_wall_ms": sorted(r["wall_ms"] for r in rows)[int(0.9 * n) - 1],
                        "mean_cached_prompt_n": sum(r["prompt_n"] for r in rows) / n}
                summary.append(srow)
                print(json.dumps(srow), flush=True)
            finally:
                proc.terminate(); proc.wait()
        fo.write(json.dumps({"kind": "summary", "rows": summary}) + "\n")
    print(f"\n| model | load s | full prompt tok | pp tok/s | gen tok/s | 60-tok wall ms | exact | mean sim (raw={summary[0]['mean_sim_raw']:.3f}) | better/worse vs raw | guard rejects | mean case ms | p90 case ms |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for r in summary:
        print(f"| {r['model']} | {r['load_s']:.1f} | {r['fixed60_prompt_n']} | {r['fixed60_pp_tps']:.0f} | "
              f"{r['fixed60_gen_tps']:.1f} | {r['fixed60_wall_ms']:.0f} | {r['exact']}/{r['n']} | {r['mean_sim']:.3f} | {r['better']}/{r['worse']} | "
              f"{r['guard_rejects']} | {r['mean_case_wall_ms']:.0f} | {r['p90_case_wall_ms']:.0f} |")
    print("results:", out)


if __name__ == "__main__":
    main()
