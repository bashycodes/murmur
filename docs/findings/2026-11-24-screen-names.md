# 2026-11-24: Name candidates from on-screen words (D107)

**Question.** With "Send on-screen words" on, the speech-recognition hints got
whole chat bubbles. Do short name-like candidates help names come out right,
without hurting ordinary English?

**Answer.** On a **full screen** (the phone's 2000-char capture) the hints from
whole bubbles were spent on the oldest sentences, and names came out right in
**10 %** of clips, the same as with no hints. Candidates, newest first, give
**35 %** at the same bench WER (1.05 %). On a **short screen** bubbles and
candidates are about equal (30 % vs 35 %). The ticket's target, **+30 points over
bubbles, was missed** on short screens (+5) and met only on full screens (+25 at
score 1.0). Most misses are names the model cannot spell from the audio at hotword
score 1.0. AI cleanup is unchanged: it still gets the raw screen words (D107).

## Setup

* Probe: `tools/probes/prototypes/screen_names_2026_11_24/` (`prepare.py`,
  `run.sh`, `score.py`). Summary rows: `summary-2026-11-24.jsonl`.
* Model: Parakeet TDT 0.6b v2 int8 (sherpa-onnx), modified-beam hotwords,
  hotword score 1.0 (`DEFAULT_HOTWORDS_SCORE`).
* Name clips: the 20 English "said" sentences of
  `crates/murmur-core/fixtures/screen-names/chat-screens.json` (all invented),
  synthesised with espeak-ng. A clip counts when every planted name is in the
  output.
* Bench: 40 LibriSpeech clips, no names, with the same hints, to check that the
  hints do not hurt ordinary speech.
* Inputs: `none`; `bubbles` (the screen's entries as today); `candidates`
  (`name_candidates`); `*_full` the same screen at the bottom of a 2000-char capture
  with older bubbles from other screens above it.

## Results (after the review fixes)

Score 1.0 (shipped):

| input           | names | name acc | names WER | bench WER | hotwords |
|-----------------|-------|----------|-----------|-----------|----------|
| none            | 2/20  | 10.0 %   | 25.00 %   | 1.05 %    | 0.0      |
| bubbles         | 6/20  | 30.0 %   | 17.39 %   | 1.05 %    | 16.5     |
| candidates      | 7/20  | 35.0 %   | 16.85 %   | 1.05 %    | 6.4      |
| bubbles_full    | 2/20  | 10.0 %   | 24.46 %   | 1.26 %    | 17.1     |
| candidates_full | 7/20  | 35.0 %   | 16.30 %   | 1.05 %    | 15.9     |

For information only (not shipped; score stays 1.0):

| input           | 1.5: name acc / bench WER | 2.0: name acc / bench WER |
|-----------------|---------------------------|---------------------------|
| none            | 10 % / 1.05 %             | 10 % / 1.05 %             |
| bubbles         | 45 % / 1.05 %             | 45 % / 1.15 %             |
| candidates      | 50 % / 1.05 %             | 55 % / 1.05 %             |
| bubbles_full    | 10 % / 1.26 %             | 10 % / 1.26 %             |
| candidates_full | 50 % / 1.05 %             | 50 % / 1.05 %             |

Before the review fixes (oldest entries first, plain lower-case words kept) the
score-1.0 numbers were 30 % for candidates and candidates_full.

## Notes

* The full-screen gap is a budget effect: the hint list (100 hotwords, the
  200-word / 900-char Whisper prompt) fills from the top, so with bubbles the
  newest names never get in. `tests/screen_names.rs` covers this without audio.
* 13 of 20 names are still wrong with candidates at score 1.0 (for example a hyphenated
  surname or a rare first name heard as English words). A higher score fixes some
  (Quentavius, Jorvik at 2.0) without moving the bench WER here, but 40 clips are
  too few to tune the score on; that needs its own test on real speech.
* espeak voices are flat and unlike real speech, so the absolute numbers are a
  lower bound; the comparison between inputs is what the probe measures.
