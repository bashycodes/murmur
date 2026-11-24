#!/usr/bin/env python3
"""Score the language-detection probe's JSONL output.

  score.py route ROUTE.jsonl [--manifest manifest.jsonl]
      accuracy per set (got == expect); with real decoders (rows carry text and the
      manifest has refs) also English WER and Hebrew WER on the he / en sets.
  score.py rules SCORES.jsonl
      accuracy per set of candidate routing rules over the per-window LID scores
      (lang_detect_scores), e.g. the D34 rule (first window, p(he) >= 0.1).
      Only clips where the VAD finds speech have windows and are counted.
  score.py compare BEFORE.jsonl AFTER.jsonl
      commit latency per set and English texts that changed between two route runs.
  score.py summary ROUTE.jsonl... [--manifest manifest.jsonl]
      per-set counts, WER and latency as JSON lines (summary-2026-11-24.jsonl).
"""
import json, math, pathlib, re, sys, unicodedata

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "bench"))
from wer import edit_distance, wer_counts  # noqa: E402

SETS = ["he", "en", "he_short", "en_short", "mix_en_he", "mix_he_en"]


def rows(p):
    return [json.loads(l) for l in open(p, encoding="utf-8") if l.strip()]


def he_words(s):
    s = unicodedata.normalize("NFKD", s)
    s = "".join(c for c in s if not unicodedata.combining(c))
    s = re.sub(r"[^\w\s]", " ", s)
    return s.split()


def table(acc):
    print(f"{'rule':<34}" + "".join(f"{s:>11}" for s in SETS) + f"{'total':>9}")
    for name, per in acc:
        tot_ok = sum(o for o, n in per.values()); tot = sum(n for o, n in per.values())
        print(f"{name:<34}" + "".join(
            f"{per[s][0]:>5}/{per[s][1]:<5}" if s in per else f"{'-':>11}" for s in SETS)
            + f"{100 * tot_ok / max(tot, 1):>8.1f}%")


def route(path, manifest=None):
    rs = rows(path)
    per = {}
    for r in rs:
        o, n = per.get(r["set"], (0, 0))
        per[r["set"]] = (o + (r["got"] == r["expect"]), n + 1)
    table([(pathlib.Path(path).name, per)])
    wrong = [f'{r["id"]}:{r["got"]}' for r in rs if r["got"] != r["expect"]]
    print("wrong:", " ".join(wrong))
    lid = sorted(r["lid_ms"] for r in rs if r["lid_ms"])
    if lid:
        print(f"lid_ms mean {sum(lid) / len(lid):.0f} p90 {lid[len(lid) * 9 // 10]} max {lid[-1]}")
    mock = all(r["text"].lower().startswith(("english", "עברית")) or not r["text"] for r in rs)
    if not manifest or mock:
        return
    refs = {m["id"]: m.get("ref") for m in rows(manifest)}
    for set_, words in (("en", None), ("he", he_words)):
        e = n = 0
        for r in rs:
            ref = refs.get(r["id"])
            if r["set"] != set_ or not ref:
                continue
            if words is None:
                a, b = wer_counts(ref, r["text"])
            else:
                a, b = edit_distance(words(ref), words(r["text"])), len(words(ref))
            e += a; n += b
        if n:
            print(f"{set_} WER {100 * e / n:.2f}% ({e}/{n} words)")


def rules(path):
    rs = rows(path)

    def first(t):
        return lambda w: w[0][1] >= t

    def mean(t, k=None):
        def f(w):
            w = w[:k] if k else w
            tot = sum(x[2] for x in w)
            return sum(x[1] * x[2] for x in w) / tot >= t
        return f

    def share(t):
        # Speech time whose window says he (p >= t) vs not.
        def f(w):
            he = sum(x[2] for x in w if x[1] >= t)
            return he >= sum(x[2] for x in w) / 2
        return f

    def first_or_majority(t):
        # The early D34 call, overturned only when most later speech disagrees.
        def f(w):
            early = w[0][1] >= t
            if len(w) < 2:
                return early
            rest = w[1:]
            he = sum(x[2] for x in rest if x[1] >= t)
            tot = sum(x[2] for x in rest)
            later = he > tot / 2
            whole = (he + (w[0][2] if early else 0)) > (tot + w[0][2]) / 2
            return whole if later != early else early
        return f

    def log_odds(t, clip=3.0):
        # Speech-weighted sum of each window's log-odds for he, measured from
        # the floor t and clipped to +-clip so one confident window cannot
        # outvote the rest; he when the sum is >= 0.
        floor = math.log(t / (1 - t))

        def f(w):
            tot = 0.0
            for _, p, secs in w:
                p = min(max(p, 1e-6), 1 - 1e-6)
                tot += secs * min(max(math.log(p / (1 - p)) - floor, -clip), clip)
            return tot >= 0
        return f

    cand = [("D34 first window p>=0.1", first(0.1)), ("first window argmax", first(0.5)),
            ("first window p>=0.05", first(0.05)), ("first window p>=0.2", first(0.2))]
    for t in (0.05, 0.1, 0.2, 0.3):
        cand.append((f"mean all windows p>={t}", mean(t)))
    for t in (0.1, 0.2):
        cand.append((f"mean first 2 windows p>={t}", mean(t, 2)))
    for t in (0.1, 0.2, 0.3):
        cand.append((f"speech share p>={t}", share(t)))
    for t in (0.1,):
        cand.append((f"first, majority overturns p>={t}", first_or_majority(t)))
    for t in (0.1, 0.05):
        cand.append((f"clipped log-odds p>={t}", log_odds(t)))
    acc = []
    for name, f in cand:
        per = {}
        for r in rs:
            if not r["windows"]:
                continue
            got = "he" if f(r["windows"]) else "en"
            o, n = per.get(r["set"], (0, 0))
            per[r["set"]] = (o + (got == r["expect"]), n + 1)
        acc.append((name, per))
    table(acc)
    empty = [r["id"] for r in rs if not r["windows"]]
    print("no speech (VAD):", len(empty), " ".join(empty[:10]))


def compare(before, after):
    """Commit latency (release -> text) per set, and English texts that changed."""
    b = {r["id"]: r for r in rows(before)}
    a = {r["id"]: r for r in rows(after)}
    print(f"{'set':<11}{'commit ms median before/after':>32}{'p90 before/after':>20}")
    for s in SETS:
        ids = [i for i in b if b[i]["set"] == s and i in a]
        if not ids:
            continue
        cb = sorted(b[i]["commit_ms"] for i in ids)
        ca = sorted(a[i]["commit_ms"] for i in ids)
        print(f"{s:<11}{cb[len(cb) // 2]:>24} / {ca[len(ca) // 2]:<6}"
              f"{cb[len(cb) * 9 // 10]:>12} / {ca[len(ca) * 9 // 10]}")
    for s in ("en", "en_short"):
        n = sum(1 for i in b if b[i]["set"] == s and i in a and b[i]["text"] != a[i]["text"])
        print(f"{s} texts changed: {n}")


def summary(paths, manifest):
    """One JSON line per route run: per-set right/total, WER, commit ms.
    Counts only, no clip text, so it can be committed next to the findings."""
    refs = {m["id"]: m.get("ref") for m in rows(manifest)} if manifest else {}
    for path in paths:
        rs = rows(path)
        out = {"run": pathlib.Path(path).name, "sets": {}}
        for s in SETS:
            sr = [r for r in rs if r["set"] == s]
            if not sr:
                continue
            ms = sorted(r["commit_ms"] for r in sr)
            out["sets"][s] = {"right": sum(r["got"] == r["expect"] for r in sr), "n": len(sr),
                              "no_speech": sum(r.get("empty_reason") == "no_speech" for r in sr),
                              "commit_ms_median": ms[len(ms) // 2],
                              "commit_ms_p90": ms[len(ms) * 9 // 10]}
        mock = all(r["text"].lower().startswith(("english", "עברית")) or not r["text"] for r in rs)
        if refs and not mock:
            for set_, words in (("en", None), ("he", he_words)):
                e = n = 0
                for r in rs:
                    ref = refs.get(r["id"])
                    if r["set"] != set_ or not ref:
                        continue
                    if words is None:
                        a, b = wer_counts(ref, r["text"])
                    else:
                        a, b = edit_distance(words(ref), words(r["text"])), len(words(ref))
                    e += a; n += b
                out[f"{set_}_wer"] = {"errors": e, "words": n}
        print(json.dumps(out, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    if sys.argv[1] == "summary":
        m = sys.argv.index("--manifest") if "--manifest" in sys.argv else None
        summary(sys.argv[2:m], sys.argv[m + 1] if m else None)
    elif sys.argv[1] == "compare":
        compare(sys.argv[2], sys.argv[3])
    elif sys.argv[1] == "route":
        route(sys.argv[2], sys.argv[4] if len(sys.argv) > 4 and sys.argv[3] == "--manifest" else None)
    else:
        rules(sys.argv[2])
