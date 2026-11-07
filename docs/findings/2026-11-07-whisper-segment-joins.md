# 2026-11-07: whisper "repeats text at segment joins" (D66)

**Question.** With D22 segmentation on (`segment_pause_ms` 800), whisper
output on the Wispr parity main set repeated text, and one clip went from
18% to 68% WER. Is it audio overlap at the cuts, the `prior_text` carried
into the next piece, or something else? Fix it at the source.

**Setup.** Probe `tools/probes/prototypes/whisper_joins_2026_11_07/`
(scripts only; clips and transcripts stay private). Parity main set, 13
clips, whisper small q5_1, English, session = the phone config (VAD,
guard, no dictionary). WER uses the parity eval's normaliser. `core` = the
real `FlowCore` commit, whole vs segmented, unpaced (same text as paced:
pieces and bridges decode at commit instead of while recording).

## Root cause

Neither overlap nor the prompt. The repeat sits **inside one piece**, from
D20's shrunken encoder window, and D20's repetition guard missed it.

1. **Reproduced.** Before the fix: whole 16.8% pooled WER (mean 24.8%),
   segmented 20.9% (29.0%). Clip s05: 17.6% → 67.6% (whole 37 words,
   segmented 54: one 18-word insertion).
2. **Not the joins.** `joins pieces` counts joins where the right piece
   repeats a 4-word run from the left piece's tail: 0 of 13 joins on the 4
   affected clips, with and without `prior_text`. Cuts are disjoint sample
   ranges, so there is no audio overlap either.
3. **Not `prior_text`.** s05 decoded piece by piece with the fitted window:
   79.4% WER without `prior_text`, 67.6% with it. The loop is there without
   any prompt.
4. **The encoder window.** Piece 1 of s05 (8.4 s) decoded with the fitted
   window (`audio_ctx` = piece + 1 s) gave 36 words: a 14-word span, 4
   other words, the same 14-word span again. With the full 30 s window the
   same piece gave 18 words and s05 scored 26.5%. A whole clip over 29 s
   always gets the full window, but every segment piece is short, so
   segmenting turns full-window decodes into fitted ones.
5. **Why the guard missed it.** `looks_degenerate` re-decodes with the full
   window on > 4 words/s + 3 (36 words in 8.4 s is just under 36.6) or a
   phrase repeated **back to back**. Here 4 words sat between the copies.

Zero-padding pieces to 10 s (a floor on the fitted window) made it worse
(4 clips: 31.2% → 38.1% pooled; a new loop on s09), so a window floor is
not the fix.

## Fix

`whisper::looks_degenerate` also flags any run of
`LOOP_PHRASE_WORDS` (6) words that occurs twice without overlapping,
anywhere in one decode. That triggers the existing D20 re-decode with the
full window. Generic: no text list, no tuning to the eval. A false alarm
costs one re-decode (its full-window result is used as is). It only runs
on fitted-window decodes (`ctx > 0`), whole clips included.

No join dedupe was added: no join repeated text, so it would have nothing
to remove.

## Results (13 clips, whisper small q5_1, pooled WER, mean in brackets)

| | whole | segmented | gap |
|---|---|---|---|
| before | 16.8% (24.8%) | 20.9% (29.0%) | +4.1 |
| after | 16.8% (24.8%) | 18.3% (26.3%) | +1.5 |

s05 segmented 67.6% → 32.4%. Every other clip's output is unchanged in
both modes (the new check fired nowhere else, and matched no piece decoded
with the full window). Remaining gap: s05 (+14.8 vs whole) and s12 (8
short pieces, +17.1), from pieces decoded with less context, not
repetition.

**Why s05 is 32.4% here but 26.5% piece by piece (Root cause, 4).** The
26.5% decoded all three pieces with the full window. After the fix only
piece 1 (the loop) is decoded again with the full window; pieces 2 and 3
are clean with the fitted window, so they keep their fitted-window text,
which has more errors. Piece by piece with the fix (fitted window, re-decode
on a loop): 32.4%, the same as the full pipeline, so the bridges, the guard
and the join add nothing on s05.

**False alarms (D20 bench, `tests/audio_ctx_bench.rs`, 40 LibriSpeech
clips, fitted window, `redecodes()` counter, before → after):** tiny.en 0 →
0 re-decodes, base.en 2 → 2, small q5_1 0 → 0. No clip re-decodes that did
not before, and all 120 outputs are byte-identical. ASR totals
18.1 → 20.3 s, 67.9 → 67.7 s, 139.4 → 123.2 s: the same work, within run
noise on a shared box (no added decodes). Run with
`tools/probes/prototypes/whisper_joins_2026_11_07/bench.sh`.

**Full window for every piece instead** (fit_ctx off, 4 affected clips):
segmented 19.3% vs whole 11.9% pooled, so the gap stays, and segmented ASR
time is 1.8× (595 vs 333 s summed on this box). Not adopted.

**Limit.** Words are split on whitespace, so scripts written without
spaces (Chinese, Japanese, Thai) get no protection from this check: a
clause is one "word". Only the words/s cap and the old back-to-back checks
on whole clauses can fire there.

## Tests

`whisper::tests::a_loop_with_words_between_is_decoded_again_with_the_full_window`:
`fit_then_full` (the D20 decode-then-re-decode step, now a function that
takes the decoder) with a fake decoder. A synthetic looped fitted result
leads to a second decode with the full window (ctx 0), whose text is
returned, and the counter goes up. A clean fitted result is decoded once,
and a full-window decode is never repeated.


`whisper::tests::repetition_guard_catches_a_loop_with_words_between`:
synthetic 12-word phrase, 4 other words, the phrase again, at a word rate the
length check accepts → degenerate; a 4-word and a 5-word phrase said twice
→ not degenerate. Existing `repetition_guard` cases unchanged.
