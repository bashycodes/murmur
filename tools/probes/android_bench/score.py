#!/usr/bin/env python3
"""Score a pulled on-device benchmark run (tools/probes/android_bench/README.md).

usage: python3 -I tools/probes/android_bench/score.py RUN_DIR [--bench-bin PATH] [--fails LABEL]

RUN_DIR holds what BenchActivity wrote (meta.json, stt-*.jsonl, cleanup-*.jsonl).

* STT (stt-<model>.jsonl): WER on `asr_text` against the repo manifests (English: bench/wer.py
  normaliser; Hebrew: nikud + punctuation stripped, as the 2026-10-28/30 benches), CER (same
  normalisation, spaces removed), and latency: `commit_ms` = release -> result on the phone (all
  audio pushed first, so the whole decode is in it unless `segment` was set), p50/p90, and RTF =
  sum(commit_ms) / sum(audio_ms). Empty / error rows count as all-deleted.
* Cleanup, `prod` rows: the product's own `output` (fallback rules, prompt v2, guard,
  post-processing, the app's 3 s budget), scored as is by ai_cleanup_bench_2026_10_26/score.py.
* Cleanup, `v4c` rows: the raw LLM answers go through `ai-cleanup-bench --mode regard` (product
  strict/loose guards, fallback rules, post-processing; built from this checkout) and score.py's
  `out_strict` column, exactly as the 2026-10-30 laptop bench (score_pc.sh). Without the binary
  the v4c rows are skipped with the build command.
Writes RUN_DIR/score.md (and judged/) and prints it.
"""
import argparse, json, os, pathlib, re, subprocess, sys
from collections import Counter

HERE = pathlib.Path(__file__).resolve().parent
PROBES = HERE.parent
REPO = PROBES.parent.parent
sys.path.insert(0, str(PROBES / "bench"))
from wer import edit_distance, normalise  # noqa: E402

AI = PROBES / "prototypes/ai_cleanup_bench_2026_10_26"
MANIFESTS = {"en": PROBES / "bench/asr_manifest_extended.json",
             "he": PROBES / "prototypes/laptop_bench_2026_10_30/he_manifest.json"}
NIKUD = re.compile(r"[֑-ׇ]")
DEFAULT_BIN = REPO / "target/ai-bench/release/ai-cleanup-bench"


def he_words(s):
    s = NIKUD.sub("", s)
    return re.sub(r"[^\w\s]", " ", s).split()


def words(s, lang):
    return he_words(s) if lang == "he" else normalise(s)


def pct(xs, q):
    if not xs:
        return 0
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def load(p):
    return [json.loads(l) for l in open(p, encoding="utf-8") if l.strip()]


def refs():
    return {s: {u["id"]: u["ref"] for u in json.load(open(p, encoding="utf-8"))["utterances"]} for s, p in MANIFESTS.items()}


def score_stt(rows, ref):
    """Totals for one model's rows (all one set)."""
    we = wn = ce = cn = 0
    commit, audio, bad = [], [], 0
    for r in rows:
        lang = r["set"]
        gold = ref[lang].get(r["id"])
        if gold is None:
            continue
        hyp = r.get("asr_text") or ""
        if r.get("error") or r.get("status") in ("ERROR", None):
            bad += 1
        gw, hw = words(gold, lang), words(hyp, lang)
        we += edit_distance(gw, hw)
        wn += len(gw)
        gc, hc = list("".join(gw)), list("".join(hw))
        ce += edit_distance(gc, hc)
        cn += len(gc)
        if r.get("commit_ms") is not None and r.get("audio_ms"):
            commit.append(r["commit_ms"])
            audio.append(r["audio_ms"])
    return {"n": len(rows), "wer": we / max(wn, 1), "cer": ce / max(cn, 1), "words": wn, "bad": bad,
            "p50": pct(commit, .5), "p90": pct(commit, .9), "max": max(commit, default=0),
            "rtf": sum(commit) / max(sum(audio), 1), "audio_s": sum(audio) / 1000}


def regard(bin_, src, judged):
    """ai-cleanup-bench --mode regard over the cases the run has (a --limit run has fewer)."""
    have = {r["id"] for r in load(src)}
    cases = judged.with_suffix(".cases.jsonl")
    with open(AI / "results/v4c_cases.jsonl", encoding="utf-8") as f, open(cases, "w", encoding="utf-8") as o:
        o.writelines(l for l in f if l.strip() and json.loads(l)["id"] in have)
    with open(judged, "w", encoding="utf-8") as o:
        subprocess.run([str(bin_), "--mode", "regard", "--from", str(src), "--cases", str(cases),
                        "--session", str(AI / "sessions/prod.json")], stdout=o, check=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run", type=pathlib.Path)
    ap.add_argument("--bench-bin", type=pathlib.Path, default=pathlib.Path(os.environ.get("BENCH_BIN", DEFAULT_BIN)))
    ap.add_argument("--fails", default=None, help="score.py --fails LABEL (e.g. qwen1.5b-prod)")
    a = ap.parse_args()
    d = a.run
    out = []
    meta = json.load(open(d / "meta.json")) if (d / "meta.json").exists() else {}
    dev = meta.get("device", {})
    out.append(f"# On-device bench: {d.name}\n")
    out.append(f"{dev.get('model', '?')} ({dev.get('soc', '?')}, {dev.get('cores', '?')} cores), app {dev.get('app_version', '?')} "
               f"({dev.get('app_build', '?')}, {dev.get('flavor', '?')}); args {json.dumps(meta.get('args', {}))}\n")
    for m in meta.get("models", []):
        if m.get("skipped"):
            out.append(f"* {m['key']}: skipped ({m['skipped']})")
        else:
            out.append(f"* {m['key']}: load {m.get('load_ms')} ms, warm-up {m.get('warmup_ms', m.get('warmup_prod_ms', '-'))} ms, "
                       f"PSS after load {m.get('after_load', {}).get('pss_kb', 0) // 1024} MB, thermal {m.get('after_load', {}).get('thermal')}"
                       f"->{m.get('after_run', {}).get('thermal')}")
    out.append("")

    ref = refs()
    stt = sorted(d.glob("stt-*.jsonl"))
    if stt:
        out.append("## STT\n")
        out.append("| model | set | clips | audio s | WER | CER | EMPTY | WER / CER on non-empty | commit ms p50 / p90 / max (non-empty) | RTF (non-empty) | bad rows |")
        out.append("|---|---|---|---|---|---|---|---|---|---|---|")
        for f in stt:
            rows = load(f)
            if not rows:
                continue
            s = score_stt(rows, ref)
            # EMPTY rows (the app's VAD found no speech, or ASR returned nothing) count as all
            # deletions in WER; the non-empty column separates recognition from gating.
            empty = [r for r in rows if r.get("status") == "EMPTY"]
            why = ", ".join(f"{k} {v}" for k, v in sorted(Counter(r.get("empty_reason") for r in empty).items(), key=str))
            ne = score_stt([r for r in rows if r.get("status") != "EMPTY"], ref)
            out.append(f"| {f.stem[4:]} | {rows[0]['set']} | {s['n']} | {s['audio_s']:.0f} | {s['wer']:.1%} | {s['cer']:.1%} | "
                       f"{len(empty)}{f' ({why})' if empty else ''} | {ne['wer']:.1%} / {ne['cer']:.1%} | "
                       f"{ne['p50']} / {ne['p90']} / {ne['max']} | {ne['rtf']:.3f} | {s['bad']} |")
        out.append("")

    clean = sorted(d.glob("cleanup-*.jsonl"))
    if clean:
        judged = d / "judged"
        judged.mkdir(exist_ok=True)
        specs, notes = [], []
        for f in clean:
            label = f.stem[len("cleanup-"):]
            rows = load(f)
            if label.endswith("-prod"):
                # score.py counts `fallback`; the product reports it as `fallback_reason`.
                p = judged / f.name
                with open(p, "w", encoding="utf-8") as o:
                    for r in rows:
                        r["fallback"] = r.get("fallback_reason")
                        o.write(json.dumps(r, ensure_ascii=False) + "\n")
                specs.append(f"{label}={p}:output")
            elif label.endswith("-v4c"):
                if not a.bench_bin.exists():
                    notes.append(f"{label}: skipped, no {a.bench_bin} (build: cd {AI.relative_to(REPO)}/runner && "
                                 "CARGO_TARGET_DIR=../../../../../target/ai-bench cargo build --release -j 2 --offline)")
                    continue
                p = judged / f.name
                regard(a.bench_bin, f, p)
                specs.append(f"{label}={p}:out_strict")
        out.append("## Cleanup (88-suite)\n")
        out.append("`prod` = the app's own pipeline and 3 s budget; `v4c` = raw transcript + prompt v4c, product strict guard "
                   "(as the 2026-10-30 laptop bench). llm ms = one on-device LLM call.\n")
        if specs:
            cmd = [sys.executable, "-I", str(AI / "score.py"), *specs] + (["--fails", a.fails] if a.fails else [])
            out.append(subprocess.run(cmd, capture_output=True, text=True, check=True).stdout)
        out += [f"* {n}" for n in notes]
        for f in clean:
            rows = load(f)
            ms = [r["llm_ms"] for r in rows if r.get("llm_raw") is not None]
            tok = [(r.get("llm_info") or {}) for r in rows]
            errs = sum(1 for r in rows if r.get("error"))
            out.append(f"* {f.stem[8:]}: {len(ms)}/{len(rows)} LLM answers, p50 {pct(ms, .5)} ms, p90 {pct(ms, .9)} ms, "
                       f"> 1500 ms {sum(x > 1500 for x in ms)}, > 3000 ms {sum(x > 3000 for x in ms)}, errors {errs}"
                       + (f"; llm_info keys {sorted(tok[0])}" if tok and tok[0] else ""))
    text = "\n".join(out) + "\n"
    (d / "score.md").write_text(text, encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
