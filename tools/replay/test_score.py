#!/usr/bin/env python3
"""Unit test of the replay score report on a fake run (no models, no
network): python3 -I tools/replay/test_score.py"""
import importlib.util
import json
import os
import tempfile
import unittest

here = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("replay_score", os.path.join(here, "score.py"))
score = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(score)

PAIRS = [{"id": "ab6", "spoken": "the meeting is at ten no wait eleven so don't come early",
          "wispr": "The meeting is at 11 so don't come early."}]
RECS = [
    {"id": "a", "wav": "a.wav", "app_id": "x",
     "phone": {"raw": "The meeting is at 10, no wait, 11, so don't come early.",
               "text": "The meeting is at 11, so don't come early", "cleanup_route": "cloud", "llm_cost_usd": 0.0016}},
    {"id": "b", "wav": "b.wav", "app_id": "x",
     "phone": {"raw": "Is that something that exists already?", "text": "Is that something that exists already?",
               "cleanup_route": "cloud", "llm_cost_usd": 0.0016}},
    {"id": "c", "wav": "c.wav", "app_id": "x", "phone": None},
]


def row(rec, cleanup, out, **kw):
    r = {"rec": rec, "asr": "fake", "cleanup": cleanup, "prompt": "v4c" if cleanup != "none" else "-",
         "transcript": "t", "output": out, "llm_ms": 0, "asr_ms": 100}
    r.update(kw)
    return r


ROWS = [
    row("a", "none", "The meeting is at 10, no wait, 11, so don't come early"),
    row("a", "or", "The meeting is at 11, so don't come early.", cleanup_route="cloud", llm_ms=400, cost_usd=0.002),
    row("b", "none", "Is that something that exists already?"),
    row("b", "or", "Is that something that exists already?", cleanup_route="local",
        cloud_fallback="llm_unsupported", llm_ms=900, cost_usd=0.001),
    row("c", "none", "hello"),
    row("c", "or", "", error="spend_cap"),
]


class ScoreTest(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        for name, rows in (("rows.jsonl", ROWS), ("recordings.jsonl", RECS)):
            with open(os.path.join(self.d, name), "w") as f:
                f.write("".join(json.dumps(r) + "\n" for r in rows))
        self.pairs = os.path.join(self.d, "pairs.jsonl")
        with open(self.pairs, "w") as f:
            f.write("".join(json.dumps(p) + "\n" for p in PAIRS))
        self.exp = os.path.join(self.d, "expected.json")
        with open(self.exp, "w") as f:
            json.dump({"_note": "owner-editable", "b": "Is that something that exists already?"}, f)

    def test_expected_from_file_then_wispr_pairs(self):
        exp = score.expected_texts(RECS, self.exp, self.pairs)
        self.assertEqual(exp["a"], ("The meeting is at 11 so don't come early.", "wispr:ab6"))
        self.assertEqual(exp["b"][1], "file")
        self.assertNotIn("c", exp)

    def test_table(self):
        exp = score.expected_texts(RECS, self.exp, self.pairs)
        t = {x["label"]: x for x in score.score(ROWS, RECS, exp)}
        self.assertEqual(list(t), ["phone (debug log)", "fake / none / -", "fake / or / v4c"])
        ph, ai, none = t["phone (debug log)"], t["fake / or / v4c"], t["fake / none / -"]
        self.assertEqual(ph["n"], 2)
        # Wispr's comma-free line vs the phone's: close, not exact.
        self.assertEqual(ph["exact"], (1, 2))
        self.assertEqual(ai["exact"], (1, 2))
        self.assertEqual(ai["exact_phone"], (2, 2))  # final period ignored
        self.assertAlmostEqual(ai["close_phone"], 1.0)
        self.assertEqual((ai["n"], ai["errors"]), (2, 1))
        self.assertEqual(ai["fallbacks"], 1)
        self.assertEqual(ai["fb"], {"llm_unsupported": 1})
        self.assertEqual(ai["routes"], {"cloud": 1, "local": 1})
        self.assertAlmostEqual(ai["cost"], 0.003)
        self.assertAlmostEqual(ph["cost"], 0.0032)
        self.assertLess(none["close"], ai["close"])
        self.assertEqual(ai["llm_p50"], 400)

    def test_report_renders(self):
        out = score.report(score.score(ROWS, RECS, {}), {}, {"spend_usd": {"total": 0.003, "asr": 0, "cleanup": 0.003}})
        self.assertIn("| fake / or / v4c | 2 (+1 err) |", out)
        self.assertIn("OpenRouter spend this run: $0.0030", out)


if __name__ == "__main__":
    unittest.main()
