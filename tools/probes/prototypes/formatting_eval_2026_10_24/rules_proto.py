#!/usr/bin/env python3
"""Prototype of the proposed deterministic quote / parenthesis / dash rules
(D27 layer 1), applied as a pre-pass on the eval inputs so the existing
murmur-core pipeline can be scored with them. Throwaway: the real rules go
into crates/murmur-core/src/postprocess/spoken.rs (token-based, like
`spoken_punctuation`).

usage: rules_proto.py cases.jsonl > cases_rules.jsonl

Boundary policy (no ASR timings are plumbed today, see the finding):
* Paired commands ("quote ... unquote", "open quote ... close/end quote",
  "open paren ... close paren"): the span is exactly between them.
* "quote unquote X": X = the next word, or up to the next comma/period when
  Parakeet put one there (it marks pauses with commas).
* Unpaired "quote" / "in parentheses" / "in brackets": a command only when
  Parakeet wrapped it in commas (", quote, ..."), i.e. the speaker paused;
  the span runs to the next comma, period or end. Without the pause commas
  the phrase is left for the LLM ("charger in parentheses the USB-C one to
  the meeting" has no boundary a rule can see).
* "dash" between two words, not after a determiner ("a dash of salt").
"""
import json
import re
import sys



def strip_edge(s: str) -> str:
    return s.strip(" ,")


def rules(t: str) -> str:
    # quote unquote X (X up to a pause comma/period, else one word)
    t = re.sub(r"(?i),?\s*\bquote,?\s+unquote\b,?\s*([^,.!?]+?)(?=[,.!?]|$)",
               lambda m: f' "{strip_edge(m.group(1))}"', t)
    # paired quotes
    t = re.sub(r"(?i),?\s*\b(?:open\s+)?quote\b,?\s*(.+?),?\s*\b(?:unquote|close\s+quote|end\s+(?:of\s+)?quote)\b",
               lambda m: f' "{strip_edge(m.group(1))}"', t)
    # paired parens
    t = re.sub(r"(?i),?\s*\bopen\s+paren(?:thesis|s)?\b,?\s*(.+?),?\s*\bclose\s+paren(?:thesis|s)?\b,?",
               lambda m: f" ({strip_edge(m.group(1))})", t)
    # unpaired, pause-marked: ", quote, X," / ", in parentheses, X," / ", in brackets, X,"
    def unpaired(word, o, c):
        nonlocal t
        t = re.sub(r"(?i),\s*" + word + r",\s*([^,.!?]+)(,?)",
                   lambda m: f" {o}{strip_edge(m.group(1))}{c}", t)
    unpaired(r"quote", '"', '"')
    unpaired(r"in\s+parenthes[ie]s", "(", ")")
    unpaired(r"in\s+brackets", "[", "]")
    # dash between words, not "a dash of"
    t = re.sub(r"(?i)\b(?!(?:a|the|this|that|my|your)\s)(\w+)\s+dash\s+(?=\w)", "\\1 — ", t)
    # tidy: '" ,' -> '",' ; ' "x" .' -> '"x".'
    t = re.sub(r'\s+([,.!?])', r"\1", t)
    t = re.sub(r"\s{2,}", " ", t).strip()
    # American placement: comma/period inside the closing quote
    t = re.sub(r'"([^"]+)"([,.])', r'"\1\2"', t)
    # a quote that opens a sentence starts with a capital
    t = re.sub(r'(^|[.!?]\s)"([a-z])', lambda m: m.group(1) + '"' + m.group(2).upper(), t)
    return t


def main():
    for line in open(sys.argv[1]):
        if not line.strip():
            continue
        c = json.loads(line)
        c["input"] = rules(c["input"])
        print(json.dumps(c, ensure_ascii=False))


if __name__ == "__main__":
    main()
