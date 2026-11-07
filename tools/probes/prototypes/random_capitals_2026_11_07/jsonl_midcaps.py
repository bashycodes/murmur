#!/usr/bin/env python3
"""Mid-sentence capitals per stage in a dictations.jsonl debug log (2026-11-07).

Probe for docs/findings/2026-11-07-random-capitals.md. For each dictation
row: version, route, speech model, cleanup engine, segment count, and the
capitalised non-sentence-start words in `raw` (ASR after joins) and `text`
(final). `--show` prints the texts (owner content: never into the repo).
`--words FILE` reads a JSON list of `[ts, word]` pairs judged wrong (kept in
eval-private) and prints counts by version / segments / cleanup.

Usage: jsonl_midcaps.py <dictations.jsonl> [--show] [--words FILE]
"""
import collections
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from history_midcaps import midcaps  # noqa: E402


def main():
    rows = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8")]
    rows = [r for r in rows if r.get("kind") == "dictation"]
    show = "--show" in sys.argv
    wrong = None
    if "--words" in sys.argv:
        wrong = {tuple(x) for x in json.load(open(sys.argv[sys.argv.index("--words") + 1]))}
    by = collections.defaultdict(collections.Counter)
    for r in rows:
        if (r.get("language") or "en") != "en" or r.get("mode") not in ("hold", "tap"):
            continue
        raw, text = r.get("raw") or "", r.get("text") or ""
        a, t = midcaps(raw), midcaps(text)
        cleaned = bool(r.get("llm"))
        if wrong is not None:
            ws_raw = [w for _, w in a if (r["ts"], w) in wrong]
            ws_txt = [w for _, w in t if (r["ts"], w) in wrong]
            for key in (
                f"version={r.get('app_version')}",
                f"segments={'1' if (r.get('segments') or 1) <= 1 else '2+'}",
                f"cleanup={r.get('llm_engine') if cleaned else 'none'}",
                f"route={r.get('route')}",
                f"stt={r.get('stt_model')}",
                "all",
            ):
                by[key]["rows"] += 1
                by[key]["rows_wrong_raw"] += bool(ws_raw)
                by[key]["rows_wrong_final"] += bool(ws_txt)
                by[key]["wrong_raw"] += len(ws_raw)
                by[key]["wrong_final"] += len(ws_txt)
        if show and (a or t):
            print(
                f"ts={r['ts']} v={r.get('app_version')} route={r.get('route')} "
                f"stt={r.get('stt_model')} seg={r.get('segments')} llm={r.get('llm')} "
                f"eng={r.get('llm_engine')} style={r.get('style')}"
            )
            print("  raw:", [w for _, w in a], "| final:", [w for _, w in t])
            print("  raw>", raw)
            print("  fin>", text)
    for k in sorted(by):
        print(k, dict(by[k]))


if __name__ == "__main__":
    main()
