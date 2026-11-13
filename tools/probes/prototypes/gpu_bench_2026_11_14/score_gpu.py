#!/usr/bin/env python3
"""Quality and latency of the GPU runs (results/ here), and peak memory from
the PC's samplers (pc-logs/<tag>-mem.csv, copied from the bench folder).

usage: score_gpu.py TAG [TAG ...] [--fails]

Quality: the gpu_cleanup_model_2026_11_12 score.py table, unchanged, on
these results (same suites, same checks).

Latency, over every row of the four suites (122 dictations):
* server ms: llama-server's own prefill + decode time for the request (the
  GPU work a murmur-server would spend; no network).
* commit ms: the core's `llm_ms` on this box, which adds the SSH tunnel to
  the PC and back (one /tokenize call plus the request) to the server ms.

Memory (bytes -> GB, 1e9): the peaks of the sampler columns while the
server ran, minus nothing (the "adapter" columns include the rest of the
PC's GPU use; the baseline before the server started is printed beside).
"""
import csv
import json
import os
import re
import statistics
import sys

here = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(here, "..", "gpu_cleanup_model_2026_11_12"))
import score  # noqa: E402


def rows(tag, suite):
    p = os.path.join(here, "results", f"{tag}-{suite}.jsonl")
    if not os.path.exists(p):
        return None
    return [json.loads(l) for l in open(p) if l.strip()]


score.rows = rows
# score.py's junk classes miss Hangul (14B wrote Korean once on the GPU).
HANGUL = re.compile(r"[\u1100-\u11ff\uac00-\ud7af]")


def pct(xs, q):
    xs = sorted(xs)
    if not xs:
        return float("nan")
    k = (len(xs) - 1) * q
    f = int(k)
    c = min(f + 1, len(xs) - 1)
    return xs[f] + (xs[c] - xs[f]) * (k - f)


def latency(tags):
    print("\n| model | n | server p50 s | server p95 s | server max s | commit p50 s | commit p95 s"
          " | prefill tok / gen tok (mean) | decode tok/s (mean) | draft accepted / drafted | over 60 s |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for tag in tags:
        rs = []
        for s in ("bench", "numbers", "he", "ho"):
            rs += rows(tag, s) or []
        last = [r["last"] for r in rs if isinstance(r.get("last"), dict) and r["last"].get("gen_ms") is not None]
        if not last:
            continue
        srv = [(l["prefill_ms"] + l["gen_ms"]) / 1000 for l in last]
        com = [r["llm_ms"] / 1000 for r in rs if r.get("llm_ms")]
        tps = [l["gen_tokens"] / (l["gen_ms"] / 1000) for l in last if l["gen_ms"]]
        dn = sum(l.get("draft_n") or 0 for l in last)
        da = sum(l.get("draft_accepted") or 0 for l in last)
        print(f"| {tag} | {len(last)} | {pct(srv, .5):.2f} | {pct(srv, .95):.2f} | {max(srv):.2f} | "
              f"{pct(com, .5):.2f} | {pct(com, .95):.2f} | "
              f"{statistics.mean(l['prefill_tokens'] for l in last):.0f} / {statistics.mean(l['gen_tokens'] for l in last):.0f} | "
              f"{statistics.mean(tps):.1f} | {da}/{dn} | {sum(x > 60 for x in com)} |")


def memory(tags):
    print("\n| model | proc dedicated GB | proc shared GB | proc committed GB | adapter shared GB (baseline) | working set GB | private GB | OS free min GB |")
    print("|---|---|---|---|---|---|---|---|")
    for tag in tags:
        p = os.path.join(here, "pc-logs", f"{tag}-mem.csv")
        if not os.path.exists(p):
            continue
        with open(p) as f:
            rs = [r for r in csv.DictReader(l for l in f if not l.startswith("#"))]
        base = ""
        bp = os.path.join(here, "pc-logs", f"{tag}-baseline.txt")
        if os.path.exists(bp):
            base = f" ({float(open(bp).read().split(',')[1]) / 1e9:.2f})"

        def peak(k):
            return max(float(r[k] or 0) for r in rs) / 1e9

        print(f"| {tag} | {peak('proc_dedicated'):.2f} | {peak('proc_shared'):.2f} | {peak('proc_committed'):.2f} | "
              f"{peak('adapter_shared'):.2f}{base} | {peak('working_set'):.2f} | {peak('private_bytes'):.2f} | "
              f"{min(float(r['os_free'] or 0) for r in rs) / 1e9:.2f} |")


def same_as_cpu(tags):
    """Rows whose final text equals the 2026-11-12 CPU run's for the same
    model and case (that run's tags are the model names; a GPU tag
    `<model>-<variant>` is compared with `<model>`)."""
    cpu_dir = os.path.join(here, "..", "gpu_cleanup_model_2026_11_12", "results")
    print("\n| model | final text same as the CPU run | of |")
    print("|---|---|---|")
    for tag in tags:
        base = next((t for t in ("qwen2.5-14b", "qwen2.5-7b", "gemma-3-12b", "qwen2.5-1.5b") if tag.startswith(t)), None)
        if not base:
            continue
        same = n = 0
        for s in ("bench", "numbers", "he", "ho"):
            p = os.path.join(cpu_dir, f"{base}-{s}.jsonl")
            g = rows(tag, s)
            if not g or not os.path.exists(p):
                continue
            cpu = {r["id"]: r.get("text") for r in (json.loads(l) for l in open(p) if l.strip())}
            for r in g:
                if r["id"] in cpu:
                    n += 1
                    same += (r.get("text") or "").strip() == (cpu[r["id"]] or "").strip()
        if n:
            print(f"| {tag} | {same} | {n} |")


def raw_junk(tags):
    """The drafting control: Hebrew rows whose *model answer* (`raw`,
    before the core's guards) is junk by score.py's definition. Since
    2026-11-13 (after the CPU run) the D65 guard also rejects foreign words,
    so the final-text junk column undercounts what the model wrote.
    `cpu:<model>` reads the 2026-11-12 CPU run; `cpuctl:<variant>` the
    cpu_control.sh rows (five lines)."""
    he = score.cases(os.path.join(score.P, "hebrew_stays_hebrew_2026_11_07", "he_cases.jsonl"))
    he.update(score.cases(os.path.join(score.P, "hebrew_stays_hebrew_2026_11_07", "he_holdout.jsonl")))
    print("\n| run | Hebrew rows | answers with junk | which | guard fallbacks |")
    print("|---|---|---|---|---|")
    for tag in tags:
        if tag.startswith("cpu:"):
            d = os.path.join(here, "..", "gpu_cleanup_model_2026_11_12", "results")
            files = [os.path.join(d, f"{tag[4:]}-{s}.jsonl") for s in ("he", "ho")]
        elif tag.startswith("cpuctl:"):
            files = [os.path.join(here, "results", f"cpu-qwen2.5-14b-he5-{tag[7:]}.jsonl")]
        else:
            files = [os.path.join(here, "results", f"{tag}-{s}.jsonl") for s in ("he", "ho", "he5")]
        rs = [json.loads(l) for f in files if os.path.exists(f) for l in open(f) if l.strip()]
        if not rs:
            continue
        bad = []
        for r in rs:
            src, out = he[r["id"]]["input"], r.get("raw") or ""
            if set(score.LATIN.findall(out)) - set(score.LATIN.findall(src)) or score.FOREIGN.search(out) or HANGUL.search(out):
                bad.append(r["id"])
        fb = sum(1 for r in rs if r.get("fallback_reason"))
        print(f"| {tag} | {len(rs)} | {len(bad)} | {' '.join(bad)} | {fb} |")


if __name__ == "__main__":
    args = sys.argv[1:]
    if "--raw-junk" in args:
        raw_junk([a for a in args if not a.startswith("--")])
        sys.exit()
    tags = [a for a in args if not a.startswith("--")]
    score.main()
    latency(tags)
    memory(tags)
    same_as_cpu(tags)
