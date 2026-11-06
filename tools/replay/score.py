#!/usr/bin/env python3
"""Score a replay run (tools/replay/run.sh output dir).

usage: score.py --rows OUT_DIR [--expected FILE] [--pairs FILE] [--detail]

Reads OUT_DIR/rows.jsonl (one row per recording x speech model x cleanup x
prompt), OUT_DIR/recordings.jsonl (the corpus with the phone's results) and
OUT_DIR/run.json, and prints one markdown table row per combination plus a
`phone` row (what the phone pasted, from its debug log):

* expected text per recording: the corpus's expected-text JSON
  ({recording id: text}, owner-editable; default CORPUS/expected.json via
  run.sh), else the Wispr output of the A/B pair whose script matches the
  phone's transcript (tools/probes/prototypes/wispr_ab_2026_10_25/pairs.jsonl,
  closeness >= 0.6).
* closeness / exact vs expected: ai_cleanup_bench_2026_10_26/score.py's
  target-closeness (token edit distance, curly quotes, dashes and the final
  period normalised) and its period-insensitive exact match.
* vs phone: the same metrics against the phone's pasted text: how well the
  replay reproduces the phone (1.000 / all exact = identical).
* AI fallbacks: rows where AI cleanup fell back (cloud_fallback: timeout,
  error or empty answer; D59 removed the output guard), and the cleanup
  routes (cloud / local / rules).
* llm / asr p50 (ms) and the cloud cost total (cleanup + STT).
"""
import argparse
import collections
import importlib.util
import json
import os
import sys

here = os.path.dirname(os.path.abspath(__file__))
protos = os.path.join(here, "..", "probes", "prototypes")
_spec = importlib.util.spec_from_file_location(
    "bench_score", os.path.join(protos, "ai_cleanup_bench_2026_10_26", "score.py"))
sc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sc)
PAIRS = os.path.join(protos, "wispr_ab_2026_10_25", "pairs.jsonl")


def load_jsonl(p):
    with open(p) as f:
        return [json.loads(l) for l in f if l.strip()]


def exact(a, b):
    return sc.norm(a) == sc.norm(b)


def expected_texts(recs, expected_file, pairs_file):
    """{rec id: (text, source)}."""
    out = {}
    given = {}
    if expected_file:
        with open(expected_file) as f:
            given = {k: v for k, v in json.load(f).items() if not k.startswith("_")}
    pairs = load_jsonl(pairs_file) if pairs_file and os.path.exists(pairs_file) else []
    for r in recs:
        if r["id"] in given:
            out[r["id"]] = (given[r["id"]], "file")
            continue
        raw = ((r.get("phone") or {}).get("raw") or "")
        best = max(pairs, key=lambda p: sc.closeness(raw, p["spoken"]), default=None)
        if raw and best and sc.closeness(raw, best["spoken"]) >= 0.6:
            out[r["id"]] = (best["wispr"], "wispr:" + best["id"])
    return out


def score(rows, recs, exp):
    phone = {r["id"]: (r.get("phone") or {}) for r in recs}
    groups = collections.OrderedDict()
    groups[("phone", "", "")] = [
        {"rec": i, "output": p.get("text") or "", "cleanup_route": p.get("cleanup_route"),
         "cloud_fallback": p.get("cloud_fallback"), "cost_usd": p.get("llm_cost_usd"),
         "llm_ms": None, "asr_ms": None, "phone_row": True}
        for i, p in phone.items() if p.get("text") is not None]
    for r in rows:
        groups.setdefault((r["asr"], r["cleanup"], r["prompt"]), []).append(r)
    table = []
    for key, rs in groups.items():
        ok = [r for r in rs if not r.get("error")]
        ce = [sc.closeness(r["output"], exp[r["rec"]][0]) for r in ok if r["rec"] in exp]
        xe = [exact(r["output"], exp[r["rec"]][0]) for r in ok if r["rec"] in exp]
        vs = [r for r in ok if phone.get(r["rec"], {}).get("text") is not None and not r.get("phone_row")]
        cp = [sc.closeness(r["output"], phone[r["rec"]]["text"]) for r in vs]
        xp = [exact(r["output"], phone[r["rec"]]["text"]) for r in vs]
        routes = collections.Counter(r.get("cleanup_route") for r in ok if r.get("cleanup_route"))
        fb = collections.Counter(r["cloud_fallback"] for r in ok if r.get("cloud_fallback"))
        llm = [r["llm_ms"] for r in ok if r.get("llm_ms")]
        asr = [r["asr_ms"] for r in ok if r.get("asr_ms")]
        cost = sum((r.get("cost_usd") or 0) + (r.get("asr_cost_usd") or 0) for r in rs)
        table.append({
            "label": "phone (debug log)" if key[0] == "phone" else " / ".join(key),
            "n": len(ok), "errors": len(rs) - len(ok),
            "close": sum(ce) / len(ce) if ce else None, "exact": (sum(xe), len(xe)),
            "close_phone": sum(cp) / len(cp) if cp else None, "exact_phone": (sum(xp), len(xp)),
            "fallbacks": sum(fb.values()), "fb": dict(fb), "routes": dict(routes),
            "llm_p50": sc.pct(llm, .5) if llm else None, "asr_p50": sc.pct(asr, .5) if asr else None,
            "cost": cost, "rows": rs,
        })
    return table


def fmt(v, f="{:.3f}"):
    return "-" if v is None else f.format(v)


def report(table, exp, run, detail=False):
    n_exp = len(exp)
    lines = []
    lines.append(f"expected text: {n_exp} recordings ({collections.Counter(s.split(':')[0] for _, s in exp.values())})")
    lines.append("")
    lines.append("| speech / cleanup / prompt | n | closeness vs expected | exact | closeness vs phone | exact vs phone | AI fallbacks | routes | llm p50 ms | asr p50 ms | cloud $ |")
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|")
    for t in table:
        routes = ", ".join(f"{k} {v}" for k, v in sorted(t["routes"].items())) or "-"
        fb = str(t["fallbacks"]) + (" (" + ", ".join(f"{k} {v}" for k, v in sorted(t["fb"].items())) + ")" if t["fb"] else "")
        n = f"{t['n']}" + (f" (+{t['errors']} err)" if t["errors"] else "")
        lines.append(
            f"| {t['label']} | {n} | {fmt(t['close'])} | {t['exact'][0]}/{t['exact'][1]} | "
            f"{fmt(t['close_phone'])} | {t['exact_phone'][0]}/{t['exact_phone'][1]} | {fb} | {routes} | "
            f"{fmt(t['llm_p50'], '{}')} | {fmt(t['asr_p50'], '{}')} | {t['cost']:.4f} |")
    if run:
        s = run.get("spend_usd", {})
        delta = s.get("key_usage_delta")
        lines.append("")
        lines.append(f"OpenRouter spend this run: ${s.get('total', 0):.4f} (stt ${s.get('asr', 0):.4f}, "
                     f"cleanup ${s.get('cleanup', 0):.4f})"
                     + (f"; key usage delta ${delta:.4f}" if delta is not None else ""))
    if detail:
        for t in table:
            lines.append(f"\n### {t['label']}")
            for r in t["rows"]:
                e = exp.get(r["rec"], ("", ""))[0]
                lines.append(f"- {r['rec']} [{r.get('cleanup_route') or '-'}"
                             f"{' ' + r['cloud_fallback'] if r.get('cloud_fallback') else ''}"
                             f"{' ERROR ' + r['error'] if r.get('error') else ''}]\n"
                             f"    got: {r.get('output', '')!r}\n    exp: {e!r}")
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", required=True, help="run output dir")
    ap.add_argument("--expected", help="expected-text JSON {recording id: text}")
    ap.add_argument("--pairs", default=PAIRS)
    ap.add_argument("--detail", action="store_true")
    a = ap.parse_args(argv)
    rows = load_jsonl(os.path.join(a.rows, "rows.jsonl"))
    recs = load_jsonl(os.path.join(a.rows, "recordings.jsonl"))
    rp = os.path.join(a.rows, "run.json")
    run = json.load(open(rp)) if os.path.exists(rp) else None
    exp = expected_texts(recs, a.expected, a.pairs)
    print(report(score(rows, recs, exp), exp, run, a.detail))


if __name__ == "__main__":
    sys.exit(main())
