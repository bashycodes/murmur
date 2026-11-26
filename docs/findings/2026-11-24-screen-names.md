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

**Follow-up (below, "A separate hotword score for screen names").** The
candidates now get their own hotword score, 2.5 (dictionary words stay at
1.0): 40 % of names on Parakeet, bench WER unchanged in English and Hebrew.
Raising Parakeet's recognizer-wide score as well reached 50-55 %, but was
rejected: with a 30-word dictionary it raised the bench WER by 0.4-0.8 pt
("A higher Parakeet base score", below). That leaves two paths to 50 %: a
patched sherpa decoder that adds the phrase's own score before it picks
the beam's tokens, or a second recognizer with a higher base score, used
only when a screen has names.

**Option A, the patched decoder (2026-11-26, research only):**
[2026-11-26-screen-names-option-a.md](2026-11-26-screen-names-option-a.md).
- The per-phrase score at the pick, as asked, reaches 50 % but raises the
  bench WER by up to 1.25 pt. Rejected.
- Used for the pick only (not kept in the score, as upstream v2.0.2 does
  with the global score), it reaches 65 % names with an unchanged bench
  WER, at base 1.5 and screen 4.0.
- Shipping it would need a sherpa fork built on every platform, or an
  upgrade to 2.0.x plus a one-line upstream change.

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

For information only (not shipped; the score of every hotword raised):

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

## A separate hotword score for screen names

**Question.** The table above raised the score of *every* hotword. Can the
screen's name candidates get a stronger score of their own, with dictionary
words left at 1.0, and which score?

**Answer.** sherpa-onnx 1.13.8 takes a per-phrase score (`phrase :score` in
the per-stream hotword string) on both engines, so the core now sends the
candidates (`AsrRequest::screen_terms`) with `asr::SCREEN_HOTWORDS_SCORE` =
**2.5** and every other phrase without one (the recognizer's 1.0). On
Parakeet, names went from **35 % to 40 %** and stayed there from 1.5 up to
4.0; the bench WER was flat up to 3.5 and rose at 4.0. The ticket's target of
**50 % is not reached** with a per-phrase score on Parakeet. The Hebrew bench
WER did not move up to 4.0.

**Why Parakeet stops at 40 %.** In sherpa's NeMo modified beam search
(`offline-transducer-modified-beam-search-nemo-decoder.cc`), every token that
continues a hotword from the current context state gets the
*recognizer-wide* `hotwords_score` added to its log-probability *before* the
top-4 tokens are picked, and that bonus stays in the hypothesis score. The
per-phrase score only sets the context-graph bonus added *after* the pick.
So a rare name's first piece enters the beam with +1.0 whatever its own score
is; the global 2.0 run above let it in with +2.0 (and then added the graph's
2.0), which is where its 55 % came from. The streaming zipformer's beam search
(`online-transducer-modified-beam-search-decoder.cc`) has no such step, so
there the per-phrase score is the whole bonus.

**Simplest way to 50 % on Parakeet: tried and rejected** (2026-11-25, see
below). It was to load Parakeet's beam recognizer with a higher
`hotwords_score` and to send every non-screen phrase with an explicit
` :1.0`. Screen names do reach 50 %, but the higher pre-pick bonus also
applies to dictionary words in every session, and a 30-word dictionary then
raises the bench WER by 0.4-0.8 pt.

### Sweep (per-phrase score for candidates, dictionary at 1.0)

English: Parakeet as above, same 20 name clips and 40 LibriSpeech bench
clips. `none` is the same at every score (10 %, 25.00 %, 1.05 %).

| screen score | candidates: name acc / names WER / bench WER | candidates_full: name acc / names WER / bench WER | bubbles: bench WER |
|---|---|---|---|
| 1.0 | 35 % / 16.85 % / 1.05 % | 35 % / 16.30 % / 1.05 % | 1.05 % |
| 1.5 | 40 % / 15.76 % / 1.05 % | 40 % / 15.76 % / 1.05 % | 1.26 % |
| 2.0 | 40 % / 15.76 % / 1.05 % | 40 % / 15.22 % / 1.05 % | 1.05 % |
| **2.5** | **40 % / 15.76 % / 1.05 %** | **40 % / 15.22 % / 1.05 %** | 1.26 % |
| 3.0 | 40 % / 15.76 % / 1.05 % | 40 % / 15.22 % / 1.05 % | 1.26 % |
| 3.5 | 40 % / 15.76 % / 1.05 % | 40 % / 14.67 % / 1.05 % | 1.26 % |
| 4.0 | 40 % / 15.76 % / 1.36 % | 40 % / 14.67 % / 1.57 % | 1.36 % |

Score 1.0 reproduces the shipped row exactly. `bubbles` (no longer sent) is
listed for its bench WER only.

Hebrew: the streaming zipformer (`zipformer2-66m-streaming-he` int8,
`sherpa_online`). Name clips: 10 invented sentences (`he_said.json`) with the
names of the fixture's two Hebrew screens, spoken by espeak-ng's Hebrew voice.
**That voice is not intelligible to the model** (names WER 95 % with no
hints, 0/10 names at every score and input), so Hebrew name accuracy is not
measured; there is no other Hebrew TTS on the box. Bench: the 120 Hebrew
FLEURS clips of the lang-detect probe, each with a Hebrew screen in turn
(candidates: 2-4 hotwords, unrelated to the clip).

| screen score | none: bench WER | bubbles: bench WER | candidates: bench WER |
|---|---|---|---|
| 1.0 | 23.10 % | 22.95 % | 23.05 % |
| 1.5 | 23.10 % | 22.95 % | 23.05 % |
| 2.0 | 23.10 % | 22.95 % | 23.05 % |
| **2.5** | 23.10 % | 22.95 % | **23.05 %** |
| 3.0 | 23.10 % | 23.00 % | 23.05 % |
| 3.5 | 23.10 % | 23.00 % | 23.10 % |
| 4.0 | 23.10 % | 22.95 % | 23.10 % |

**Choice: 2.5.** The bench WER rose by 0.0 pt up to 3.5 in English and by at
most +0.05 pt up to 4.0 in Hebrew, so the rule (at most +0.1 pt English,
+0.2 pt Hebrew) allows up to 3.5. The names gain stops at 1.5, so going
above the ticket's range (1.0-2.5) buys nothing measured here and moves
toward the 4.0 where English rose; 2.5 is the top of the range.

Probe changes: `prepare.py --lang he`, `score.py --lang he` (Hebrew words
instead of the English normaliser), `run.sh` with `HE_ASR`, and
`MURMUR_SCREEN_HOTWORDS_SCORE` for the sweep (read by both sherpa adapters
at load, like `MURMUR_PARAKEET_HOTWORDS_SCORE`). `screen_names_e2e` now sets
the screen words as `screen_terms` too, as `asr_request` does, and raises
quiet clips with `vad::asr_level` as a session does (the lang-detect probe's
Hebrew clips sit below the engines' silence gate; the English clips are
unchanged). Summary rows with `screen_hotwords_score` are in
`summary-2026-11-24.jsonl`.

## A higher Parakeet base score (rejected, 2026-11-25)

**Question.** If Parakeet's recognizer-wide `hotwords_score` (the pre-pick
bonus above) is raised, and every non-screen phrase is pinned to ` :1.0`,
do screen names reach 50 % without hurting dictionary words?

**Answer: no.** There is no base score that passes every gate. Names reach
50 % only at a base of 1.5 or more, and at 1.5 or more a dictionary of 30
unrelated words raises the bench WER by 0.42 pt or more. The gates were:
screen names at least 50 %; dictionary-name accuracy not worse than at
base 1.0; bench WER, with and without that dictionary, up by at most
0.1 pt. The core keeps the previous behaviour: base 1.0, screen names at
2.5, other phrases with no score of their own.

New probe inputs (`screen_names_e2e`, scored by `score.py`), each built as
`engine.rs` `asr_request` builds it:
- `dictionary`: the screen's name candidates given as dictionary words, with
  no screen;
- `dict30`: a dictionary of 30 invented words that appear in no clip or
  screen (`UNRELATED`), with no screen;
- `candidates_dict30`: the screen's candidates together with that
  dictionary.

`MURMUR_PARAKEET_HOTWORDS_SCORE` sets the base. The screen score is 2.5 in
every run.

**These runs used code that was not shipped.** During the sweep, every
non-screen phrase was pinned to ` :1.0`. The shipped code sends those
phrases with no score of their own. As a result,
`MURMUR_PARAKEET_HOTWORDS_SCORE` above 1.0 on the shipped code raises
dictionary words' context-graph bonus as well, so it does not reproduce
this table. Only the base-1.0 row matches the shipped behaviour.

| base | candidates: names / bench | dictionary: names / bench | dict30: bench | candidates_dict30: names / bench |
|---|---|---|---|---|
| 1.0 (kept) | 40 % / 1.05 % | 35 % / 1.05 % | 1.05 % | 45 % / 1.05 % |
| 1.25 | 40 % / 1.05 % | 40 % / 1.05 % | 1.05 % | 45 % / 1.05 % |
| 1.5 | 55 % / 1.05 % | 50 % / 1.05 % | **1.47 %** | 55 % / **1.47 %** |
| 2.0 | 50 % / 1.05 % | 50 % / 1.05 % | **1.88 %** | 50 % / **1.99 %** |

At every base, `none` scored 10 % names and 1.05 % bench WER.
`candidates_full` scored 40 %, 40 %, 50 % and 45 % names at the four bases,
with a bench WER of 1.05 % each time.

**What the dictionary costs.** The bench errors are not the invented words
turning up in the text. They are real words split into pieces:
- "hasty" became "HAS TY", and "violence" became "VIOULENCE";
- "PHD" became "PH D", and "Leroy" became "Le Roy" or "Ler Roy";
- the quotation marks around a quote moved.

The NeMo pre-pick bonus rewards any token that starts *some* hotword. With
30 words, many common first pieces get the bonus, so a higher base pushes
the beam toward piece-by-piece spellings. At base 1.0, the same dictionary
leaves the bench unchanged.

**What would still get there.** A boost that applies only to screen names
at the pre-pick step. sherpa 1.13.8 has no setting for this: the pre-pick
bonus is recognizer-wide. It would need either a patched decoder that adds
the phrase's own score at that step, or a second recognizer with a higher
base that runs only when a screen has names. The second costs a second
model in memory.

**Update 2026-11-26:** the patched decoder was measured. See
[2026-11-26-screen-names-option-a.md](2026-11-26-screen-names-option-a.md).
