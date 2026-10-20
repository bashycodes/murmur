#!/usr/bin/env python3
"""Word error rate with a small English normaliser (stdlib only).

Normalisation (applied to both ref and hyp) is a cut-down version of the Whisper English normaliser:
lowercase; hyphens -> space; drop punctuation except in-word apostrophes; integers -> words;
a few LibriSpeech-specific compound joins ("to day" -> "today"); "mr" -> "mister".

CLI:  python3 -I wer.py "reference text" "hypothesis text"
"""
import re, sys

_ONES = "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen".split()
_TENS = "_ _ twenty thirty forty fifty sixty seventy eighty ninety".split()


def num2words(n: int) -> str:
    if n < 20: return _ONES[n]
    if n < 100: return _TENS[n // 10] + ("" if n % 10 == 0 else " " + _ONES[n % 10])
    if n < 1000: return _ONES[n // 100] + " hundred" + ("" if n % 100 == 0 else " " + num2words(n % 100))
    if n < 1_000_000: return num2words(n // 1000) + " thousand" + ("" if n % 1000 == 0 else " " + num2words(n % 1000))
    return str(n)


_JOINS = {"to day": "today", "to morrow": "tomorrow", "to night": "tonight", "text books": "textbooks",
          "ph d": "phd", "battle axe": "battleax"}
_SUBS = {"mr": "mister", "mrs": "missus", "dr": "doctor", "ok": "okay"}


def normalise(s: str) -> list[str]:
    s = s.lower().replace("’", "'")
    s = re.sub(r"[-–—/]", " ", s)
    s = re.sub(r"(\d),(\d)", r"\1\2", s)
    s = re.sub(r"\d+", lambda m: " " + num2words(int(m.group())) + " ", s)
    s = re.sub(r"[^a-z' ]", " ", s)
    s = re.sub(r"(?<![a-z])'|'(?![a-z])", " ", s)
    s = " " + " ".join(s.split()) + " "
    for a, b in _JOINS.items():
        s = s.replace(f" {a} ", f" {b} ")
    return [_SUBS.get(w, w) for w in s.split()]


def edit_distance(a: list, b: list) -> int:
    prev = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        cur = [i]
        for j, y in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x != y)))
        prev = cur
    return prev[-1]


def wer_counts(ref: str, hyp: str) -> tuple[int, int]:
    """Returns (errors, ref_words)."""
    r, h = normalise(ref), normalise(hyp)
    return edit_distance(r, h), len(r)


def wer(ref: str, hyp: str) -> float:
    e, n = wer_counts(ref, hyp)
    return e / max(n, 1)


def _selftest():
    assert wer("Hello, world!", "hello world") == 0
    assert wer("TWENTY FOUR HOURS", "24 hours") == 0
    assert wer("HOW QUEER EVERYTHING IS TO DAY", "How queer everything is today!") == 0
    assert wer("a b c d", "a x c") == 0.5
    assert wer("I'M GOING TO SEE MISTER MARSHALL", "I'm going to see Mr. Marshall") == 0


if __name__ == "__main__":
    _selftest()
    if len(sys.argv) == 3:
        print(f"{wer(sys.argv[1], sys.argv[2]):.4f}")
    else:
        print("selftest ok")
