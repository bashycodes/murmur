#!/usr/bin/env python3
"""Count mid-sentence capitals per stage in a history.sqlite (2026-11-07).

Probe for the random-capitals bug (docs/findings/2026-11-07-random-capitals.md).
Reads a copy of a Murmur history database and reports, per row, the
capitalised words that are not at a sentence start, for each stored stage
(`raw` = ASR text after segment joins, `formatted` = AI cleanup output,
`pasted` = what was inserted). Prints counts only unless `--show` is given;
never run `--show` output into the repo (it is the owner's text).

Usage: history_midcaps.py <history.sqlite> [--show]
"""
import re
import sqlite3
import sys

TOK = re.compile(r"\S+")
SENTENCE_START = re.compile(r"[.!?:]\W*\s*$|\n\s*$|^\s*$|[-*]\s*$")


def midcaps(text):
    """(offset, word) of each capitalised word not at a sentence start."""
    if not text:
        return []
    out = []
    first = True
    for m in TOK.finditer(text):
        core = m.group(0).strip("\"'()[]{}*_`,;:")
        if (
            not first
            and core
            and core[0].isupper()
            and not re.match(r"^I('|’|$)", core)
            and not (len(core) > 1 and core.isupper())
            and not SENTENCE_START.search(text[: m.start()])
        ):
            out.append((m.start(), core))
        first = False
    return out


def main():
    db = sys.argv[1]
    show = "--show" in sys.argv
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute(
        "select id, app, lang, route, raw, formatted, pasted from history "
        "where kind = '' order by id"
    ).fetchall()
    totals = {"rows": 0, "raw": 0, "formatted": 0, "pasted": 0, "rows_hit": 0}
    for id_, app, lang, route, raw, fmt, pas in rows:
        if lang != "en":
            continue
        totals["rows"] += 1
        a, b, p = midcaps(raw), midcaps(fmt), midcaps(pas)
        totals["raw"] += len(a)
        totals["formatted"] += len(b)
        totals["pasted"] += len(p)
        if a or b or p:
            totals["rows_hit"] += 1
            if show:
                print(f"#{id_} {app} route={route} cleanup={'Y' if fmt else 'N'}")
                print("  raw:", [w for _, w in a])
                if fmt is not None:
                    print("  fmt:", [w for _, w in b])
                print("  pas:", [w for _, w in p])
                print("  raw>", raw)
                print("  pas>", pas)
    print(totals)


if __name__ == "__main__":
    main()
