#!/usr/bin/env python3
"""Grade cliptest runs (fix/seg-polish, 2026-10-23).

usage: python3 -I analyze.py REFS_JSON RUN_DIR [RUN_DIR...]
RUN_DIR holds {parakeet,whisper}.jsonl from src/bin/cliptest.rs. Per run
and engine: pooled WER (seg / whole, asr_text, repo normaliser), and on
final_text: punctuation agreement seg vs whole (1 - token edit distance
with case and punctuation kept / whole tokens), lowercase words right
after a sentence end, and outputs not ending in . ? !.
"""
import json, os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'bench'))
from wer import wer_counts, edit_distance  # noqa: E402

ABBR = {'mr.', 'mrs.', 'dr.', 'st.', 'e.g.', 'i.e.', 'vs.', 'etc.'}


def low_after_stop(t):
    w = t.split()
    return sum(1 for a, b in zip(w, w[1:])
               if a.endswith(('.', '?', '!')) and not a.endswith('..') and a.lower() not in ABBR
               and b[:1].islower())


def open_end(t):
    return not t.rstrip().endswith(('.', '?', '!'))


refs = json.load(open(sys.argv[1]))
for run in sys.argv[2:]:
    for eng in ['parakeet', 'whisper']:
        p = f'{run}/{eng}.jsonl'
        if not os.path.exists(p):
            continue
        rows = {}
        for line in open(p):
            r = json.loads(line)
            rows[(r['clip'], r['mode'])] = r
        tw = ts = tn = pd = pn = 0
        low = {'seg': 0, 'whole': 0}
        opn = {'seg': 0, 'whole': 0}
        space = 0
        nclip = 0
        ms = {'seg': 0, 'whole': 0}
        for c in refs:
            if (c, 'whole') not in rows or (c, 'seg') not in rows:
                continue
            nclip += 1
            w, s = rows[(c, 'whole')], rows[(c, 'seg')]
            ew, n = wer_counts(refs[c], w['asr_text'])
            es, _ = wer_counts(refs[c], s['asr_text'])
            tw += ew; ts += es; tn += n
            pd += edit_distance(s['final_text'].split(), w['final_text'].split())
            pn += len(w['final_text'].split())
            for m, r in (('seg', s), ('whole', w)):
                ms[m] += r['commit_ms']
                low[m] += low_after_stop(r['final_text'])
                opn[m] += open_end(r['final_text'])
                space += r['final_text'].endswith(' ')
        print(f"{run} {eng}: WER seg/whole {100*ts/tn:.2f}/{100*tw/tn:.2f}% ({tn} words) | "
              f"punct agreement seg~whole {100*(1-pd/pn):.1f}% ({pd} tok diffs) | "
              f"lowercase after stop seg/whole {low['seg']}/{low['whole']} | "
              f"open endings seg/whole {opn['seg']}/{opn['whole']} of {nclip} | trailing space {space}/{2*nclip} | "
              f"mean release->text seg/whole {ms['seg']//nclip}/{ms['whole']//nclip} ms")
