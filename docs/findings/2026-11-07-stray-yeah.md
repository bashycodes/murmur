# 2026-11-07: a stray "Yeah." at the end of PC dictations

**Report (owner, 2026-11-07).** "Yeah" shows up in the final text of dictations where the
owner did not say it. It happened twice that day.

**Answer: our ASR stage adds it. A pause-cut piece that holds only the key click at the
end of the recording was raised by up to +40 dB, and Parakeet wrote "Yeah." for it.**
Cleanup only kept what the ASR gave it.

## Evidence (Windows PC, read-only copies, kept out of the repo)

* `history.sqlite` and `debug/dictations.jsonl` were copied from the PC's app data. Two rows
  from 2026-11-07 hold "yeah": both are about 33–34 s long, hands-free (`tap`) dictations,
  `route` local, Parakeet unified-en 0.6B int8, D22 segmentation on (5–6 pieces), and
  on-device cleanup.
* **The first stage that holds the word is the raw ASR text.** In both rows, `raw` ends in a
  separate sentence "Yeah.", and the cleaned text keeps it ("Yeah"). So hypotheses 3 and 4
  (cleanup prompt, few-shots, filler handling) are ruled out.
* The retry audio for both rows was in `history_audio`
  (`tools/probes/prototypes/stray_yeah_2026_11_07/dump_audio.py`). Both recordings have
  the same shape:
  * the speech ends about 31.6 s in;
  * then about 1.4 s of near-silence (≈ −72 dBFS; the microphone is noise-suppressed);
  * then one or two short bursts of 60–140 ms at ≈ −50 dBFS, in the last second. This is
    the stop shortcut being pressed.
* `vad_runs.py`, a Python copy of `vad::detect`, shows what happens to those bursts:
  * the quiet-recording bar (2026-11-01, 8 × the floor) puts the speech bar at ≈ −56 dBFS
    in that tail;
  * each click is a "speech" run of at least `MIN_RUN`;
  * real speech in the same recordings runs at −34…−38 dBFS RMS.

## Feedback loop (dev box, same model, `flow-cli` with the PC's session settings)

| input | pieces | ends with "Yeah." |
|---|---|---|
| row A, segmented (as on the PC) | 6 (PC: 6) | **yes** |
| row A, last 0.25 s (the click) zeroed | 5 | no |
| row B, segmented | 5 (PC: 5) | **yes** |
| row B, last 1.1 s (the clicks) zeroed | 4 | no |
| row A / B, whole-clip decode (segmentation off) | — | no / no |
| the tail after the last word, decoded on its own | — | yes / yes |
| row A / B, segmented, **with the fix** | 6 / 5 | **no / no** |

The replay matches the PC piece for piece. Zeroing only the click removes the word, and so
does decoding the whole clip at once. The word therefore comes from the tail piece being
decoded on its own.

With the fix, the replay of both rows gives the same pieces and the same text as before,
word for word, except that the final "Yeah." is gone (45 → 44 and 53 → 52 words).

## Cause

`decode_piece` (D22) ran `vad::asr_level` on each piece by itself. That rule raises a VAD
span whose RMS is under 0.006 until its peak reaches 0.5, by at most +40 dB. It was built
for recordings that are quiet throughout, a soft voice or a low-gain microphone. A piece
that is only a click on a −72 dBFS floor qualifies, so the click reached Parakeet at full
level, and Parakeet wrote a short word for it.

A whole-clip decode never raises that click, because the speech in the clip already puts its
level above 0.006. In a segmented decode, the engines' own whole-clip RMS gate (0.003)
would otherwise have returned "" for that piece.

Hypothesis 1 holds, but in a narrower form than stated: the cause is the gain, not
silence alone. Hypotheses 2 (join/bridge) and 5 (D66 loop guard, D68 beam: the PC decodes
with greedy search plus hotwords, and the word is in the piece itself) are not involved.

## Fix (D72, `crates/flow-core`)

* `engine::piece_gain`: a piece's gain is its own `vad::asr_gain`, but never more than the
  gain of the dictation's speech up to the piece's end, looking back at most 30 s.
  * In a normal-level dictation, a trailing click now stays at its true level, as in a
    whole-clip decode, and the engine's gate blanks it.
  * A dictation that is quiet throughout still has every piece raised.
  * The gain is a pure function of the audio, so pieces decoded while recording and at
    commit still match.
* `vad::asr_gain` / `vad::with_gain` split `asr_level` into "how much" and "apply". The
  `asr_level` result is unchanged.
* There is no word list and no change to the cleanup.
* The fix is in flow-core, so it covers the phone, the PC and flow-server, which all use
  D22 segmentation.

## Tests

`crates/flow-core/tests/segments.rs`, with invented audio:

* `a_click_after_the_last_word_is_not_raised_into_a_word`: speech, a 1.5 s pause and an
  80 ms −50 dBFS click, over −72 dBFS hiss. The segmented text must equal the whole-clip
  text. It fails before the fix (one extra word) and passes after it.
* `a_quiet_dictation_is_still_raised_piece_by_piece`: a quiet two-piece dictation is still
  raised, and the segmented and whole-clip decodes agree. It passes before and after the
  fix, so the fix does not take the quiet-speech gain away.

## Open

* Trade-off: a piece of real speech whose span RMS is under the engines' 0.003 gate
  (≈ −50 dBFS, 15+ dB softer than the rest of a normal-level dictation) is no longer raised,
  so the engine returns "" for it. Before the fix it was raised on its own. Not seen in the
  two rows; the parity sets were not re-run.

* Older "Yeah." rows (2026-10-30 … 11-05) are mostly 0.5–1.2 s recordings whose whole text
  is "Yeah.", with no audio kept. They look like the same thing in unsegmented form: a stop
  or start tap with no speech. They are not fixed here, because a whole-clip decode of a
  click-only clip still gets raised. One 3.9 s row does hold real sound (≈ 2.6 s at
  −35…−45 dBFS) and may really be the owner speaking.
* Phone history was not checked. The same core code runs there.
* Live previews still raise each preview window on its own. Previews are transient, and the
  final text does not use them.
