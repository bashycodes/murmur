#!/usr/bin/env python3
"""Score the on-screen-names probe (screen_names_e2e output).

  score.py RESULTS.jsonl --manifest manifest.jsonl

Per input (none / bubbles / candidates, and the last two on a full 2000-char capture):
  name accuracy  share of `names` clips whose planted name appears in the text, as
                 consecutive words after the bench normaliser (case, hyphens,
                 punctuation ignored; "Severine's" counts for "Severine");
  WER            on the `names` clips and on the 40 LibriSpeech `bench` clips;
  hotwords       mean phrase count handed to sherpa.
--lang he: Hebrew words (niqqud and punctuation dropped, hyphens split; a one-letter
prefix before the name does not count).
"""
import argparse, json, pathlib, re, sys, unicodedata

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "bench"))
from wer import edit_distance, normalise, wer_counts  # noqa: E402

MODES = ["none", "bubbles", "candidates", "bubbles_full", "candidates_full"]


LANG = "en"


def he_words(s):
    s = unicodedata.normalize("NFKD", s)
    s = "".join(c for c in s if not unicodedata.combining(c))
    s = re.sub(r"[-־/]", " ", s)
    s = re.sub(r"[^\w\s]", " ", s)
    return s.split()


def words(s):
    if LANG == "he":
        return he_words(s)
    return [w[:-2] if w.endswith("'s") else w for w in normalise(s)]


def has(name, text):
    n, t = words(name), words(text)
    return any(t[i:i + len(n)] == n for i in range(len(t) - len(n) + 1))


def wer(pairs):
    e = n = 0
    for ref, hyp in pairs:
        if LANG == "he":
            r = words(ref)
            de, dn = edit_distance(r, words(hyp)), len(r)
        else:
            de, dn = wer_counts(ref, hyp)
        e += de; n += dn
    return 100 * e / max(n, 1)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("results")
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--lang", choices=["en", "he"], default="en")
    a = ap.parse_args()
    global LANG
    LANG = a.lang
    man = {r["id"]: r for r in map(json.loads, open(a.manifest, encoding="utf-8"))}
    res = [json.loads(l) for l in open(a.results, encoding="utf-8") if l.strip()]
    names = [r for r in res if r["set"] == "names"]
    bench = [r for r in res if r["set"] == "bench"]
    summary = {}
    print(f"{'input':<16}{'names':>10}{'name acc':>10}{'names WER':>11}{'bench WER':>11}{'hotwords':>10}")
    modes = [m for m in MODES if m in res[0]]  # older and Hebrew runs lack *_full
    for m in modes:
        ok = sum(has(man[r["id"]]["name"], r[m]) for r in names)
        acc = 100 * ok / max(len(names), 1)
        nw = wer((man[r["id"]]["ref"], r[m]) for r in names)
        bw = wer((man[r["id"]]["ref"], r[m]) for r in bench)
        hw = sum(r[f"{m}_hotwords"] for r in res) / max(len(res), 1)
        summary[m] = dict(name_ok=ok, names=len(names), name_acc=round(acc, 1),
                          names_wer=round(nw, 2), bench_wer=round(bw, 2), hotwords=round(hw, 1))
        print(f"{m:<16}{ok:>5}/{len(names):<4}{acc:>9.1f}%{nw:>10.2f}%{bw:>10.2f}%{hw:>10.1f}")
    print()
    for r in names:
        name = man[r["id"]]["name"]
        marks = " ".join("+" if has(name, r[m]) else "-" for m in modes)
        print(f"{r['id']} [{marks}] {name}: {r['candidates']}")
    print(json.dumps(summary))


if __name__ == "__main__":
    main()
