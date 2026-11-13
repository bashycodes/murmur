#!/usr/bin/env python3
"""Whisper decoder thresholds experiment (dated probe, 2026-11-13).

Scores before/after runs of
  * crates/murmur-core/tests/audio_ctx_bench.rs   (40-clip WER, fitted window), and
  * crates/murmur-core/tests/whisper_tails_bench.rs (noise / tail / quiet cases).
Finding: docs/findings/2026-11-13-whisper-decoder-thresholds.md.

Usage: python3 -I score.py WER_BEFORE.jsonl WER_AFTER.jsonl TAILS_BEFORE.jsonl TAILS_AFTER.jsonl
Prints one Markdown table per model plus the outputs that changed.
"""
import json, os, re, sys
from collections import defaultdict

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "bench"))
from wer import wer_counts  # noqa: E402


def load(p):
    return [json.loads(l) for l in open(p) if l.strip()]


def wer(rows):
    e = n = 0
    for r in rows:
        a, b = wer_counts(r["ref"], r["hyp"])
        e += a; n += b
    return 100 * e / max(n, 1)


# Stock sentences whisper appends after speech (hallucination.rs STOCK/TRAILING subset).
_TAILS = {"you", "thank you", "thanks", "thanks for watching", "thank you for watching",
          "the end", "bye", "okay", "so"}


def stock_tail(hyp):
    """The last of 2+ sentences is a stock phrase ("... Holland. The End.")."""
    s = [x for x in re.split(r"[.!?]+", hyp) if x.strip()]
    return len(s) > 1 and re.sub(r"[^a-z ]", "", s[-1].lower()).strip() in _TAILS


def stats(wer_rows, tail_rows):
    by = defaultdict(lambda: defaultdict(list))
    for r in wer_rows:
        by[r["model"]]["clean"].append(r)
    for r in tail_rows:
        by[r["model"]][r["case"]].append(r)
    out = {}
    for m, c in by.items():
        noise, blip, tail, quiet = c["noise"], c["blip"], c["tail"], c["quiet"]
        out[m] = {
            "40-clip WER": f"{wer(c['clean']):.2f} %",
            "40-clip ASR total": f"{sum(r['asr_ms'] for r in c['clean']) / 1000:.1f} s",
            "40-clip: ends in a stock sentence": sum(stock_tail(r["hyp"]) for r in c["clean"]),
            f"noise: non-empty / {len(noise)}": sum(bool(r["hyp"].strip()) for r in noise),
            "noise: stock phrase (guard's job)": sum(r["v"]["stock"] for r in noise),
            "noise: leaks past guard, VAD off": sum(r["v"]["leak"] for r in noise),
            f"blip: non-empty / {len(blip)}": sum(bool(r["hyp"].strip()) for r in blip),
            "blip: stock phrase": sum(r["v"]["stock"] for r in blip),
            "blip: leaks past guard, VAD off": sum(r["v"]["leak"] for r in blip),
            "tail (+4 s noise): WER": f"{wer(tail):.2f} %",
            f"tail: stock tail or stock-only / {len(tail)}": sum(r["v"]["tail"] or r["v"]["stock"] for r in tail),
            "quiet (−60 dBFS, VAD on): WER": f"{wer(quiet):.2f} %",
            f"quiet: empty / {len(quiet)}": sum(not r["hyp"].strip() for r in quiet),
        }
    return out, by


def main():
    wb, wa, tb, ta = (load(p) for p in sys.argv[1:5])
    sb, rb = stats(wb, tb)
    sa, ra = stats(wa, ta)
    for m in sb:
        print(f"\n### {m}\n\n| metric | before (2.4 / -1.0) | after (2.8 / -1.25) |\n|---|---|---|")
        for k in sb[m]:
            print(f"| {k} | {sb[m][k]} | {sa[m][k]} |")
    print("\nChanged outputs:")
    for m in rb:
        for case in ("clean", "noise", "blip", "tail", "quiet"):
            b = {r["id"]: r for r in rb[m][case]}
            for r in ra[m][case]:
                o = b.get(r["id"])
                if o and o["hyp"] != r["hyp"]:
                    print(f"  {m} {case} {r['id']}:\n    - {o['hyp'][:160]!r}\n    + {r['hyp'][:160]!r}")


if __name__ == "__main__":
    main()
