# 2026-11-13: Whisper decoder thresholds (`entropy_thold` 2.8, `logprob_thold` -1.25)

**Question.** OpenWhispr raised whisper.cpp's `entropy_thold` from 2.4 to 2.8 and
lowered `logprob_thold` from -1.0 to -1.25. They report hallucinated tails falling from
2.25 % to 0.06 % over 4,814 dictations
(`2026-11-07-openwhispr-analysis.md` §B.8, follow-up 3). Should we set the same values in
`murmur_core::whisper`? The bar: WER holds, and fewer stock tails reach
`hallucination.rs`.

**Answer: no. Rejected (D88).** WER is unchanged on base and small, and moves by ±0.1–0.4
points on tiny. Stock phrases do not go down: over all probes they go **82 → 89**.
English-only tiny.en gets worse. It now writes "♪♪" on 6 more noise windows and adds a
"Thank you." to one clean clip. The defaults stay as whisper.cpp ships them.

## Why it doesn't transfer

In whisper.cpp the two values do two jobs (`whisper_full_with_state`):

1. **Temperature fallback.** A decode is retried at a higher temperature when its
   entropy is under `entropy_thold` (a loop), or when its average log-probability is
   under `logprob_thold` and `no_speech_prob` is under 0.6.
2. **The no-speech drop.** A segment is dropped when `no_speech_prob > 0.6` **and**
   `avg_logprob < logprob_thold`. Lowering `logprob_thold` to -1.25 makes this drop
   *harder* to reach. On tiny.en, near-silent windows that used to come back empty now
   come back as "♪♪" or "Thank you.".

OpenWhispr runs `whisper-server` on whole recordings. We decode VAD-trimmed spans with a
fitted encoder window (D16, D20), and D20 re-decodes loops. So the "mostly silent
window" that gave them their tails is rare on our path.

**Side finding: on multilingual models our `no_speech_prob` is always about 0.** Over 342
decodes, the largest value was 0.001 on tiny, base q5_1 and small q5_1. On tiny.en it
reached 0.89. whisper.cpp reads the no-speech token after the full prompt (`sot`,
language, `transcribe`), not at `sot`. For multilingual models that probability is about
0. So with these models:
* whisper.cpp's own no-speech drop never fires;
* `hallucination.rs` gets no evidence from Whisper when the VAD is off (the Windows GPU
  plugin with the VAD off, the server with the VAD off).

With the VAD on, `speech_ms` still guards the stock phrases. This needs its own ticket
(read the no-speech probability at `sot`, or depend on the VAD only). It is not changed
here.

## Method

All inputs are the repo's public eval set: the 40 LibriSpeech test-clean clips of
`tools/probes/bench/asr_manifest_extended.json` (955 words), plus synthetic noise. No
owner audio was used. Models: tiny.en, tiny (multilingual), base q5_1 (D4's CPU
fallback), small q5_1. The fitted window was on, as in production. Each run used 4
threads on a shared 8-core box.

* **40-clip WER:** `crates/murmur-core/tests/audio_ctx_bench.rs` (the D20/D66 bench), with
  `MURMUR_BENCH_FIT=true`.
* **Probes:** `crates/murmur-core/tests/whisper_tails_bench.rs` (new, `#[ignore]`):
  * `noise`: white and brown noise, RMS 0.0035–0.014 (all above the engine's 0.003
    silence gate), 1.5, 3 and 6 s long, 2 seeds each, 36 windows. Decoded by the engine
    directly, as when the VAD is off or passes a noise window by mistake.
  * `blip`: 250 ms from the middle of each clip in 3 s of quiet noise, 40 windows. This
    stands for a breath or a key-press tail that the VAD lets through.
  * `tail`: each clip followed by 4 s of quiet noise, engine direct (the case OpenWhispr
    fixed).
  * `quiet`: each clip at about −60 dBFS through `MurmurCore` with the VAD on. This is
    the quiet-speech path of `2026-11-01-quiet-speech-gate.md` (relative bar plus
    `asr_level`).
* `hallucination.rs` verdicts are taken per output:
  * `stock`: the whole text is a stock phrase, a credit or only punctuation;
  * "ends in a stock sentence": `… Holland. The End.`;
  * `leak`: non-empty text the guard keeps when the VAD is off, so it has only
    `no_speech_prob`.
* The hallucination fixtures (`tests/hallucination_vectors.rs`) are text-only, so the
  decoder settings cannot change them. They pass on both builds.
* "Before" is `origin/integration` at `ce7e398`. "After" is the same code plus
  `set_entropy_thold(2.8)` and `set_logprob_thold(-1.25)` in `WhisperEngine::decode`.
* Reproduce with `tools/probes/prototypes/whisper_thresholds_2026_11_13/run.sh` once per
  build, then run `score.py`. The raw outputs are kept outside the repo.

## Numbers

| model | metric | before (2.4 / -1.0) | after (2.8 / -1.25) |
|---|---|---|---|
| tiny.en | 40-clip WER | 3.98 % | 4.08 % |
| | 40-clip outputs ending in a stock sentence | 0 | 1 |
| | noise: stock phrase / 36 | 2 | 8 |
| | blip: stock phrase / 40 | 1 | 1 |
| | tail: stock tail / 40 | 0 | 0 |
| | tail WER | 3.87 % | 3.98 % |
| | quiet WER (VAD on) | 6.39 % | 6.39 % |
| tiny | 40-clip WER | 7.85 % | 7.43 % |
| | 40-clip outputs ending in a stock sentence | 8 | 6 |
| | noise: stock phrase / 36 | 12 | 10 |
| | blip: stock phrase / 40 | 2 | 3 |
| | tail: stock tail / 40 | 1 | 3 |
| | tail WER | 9.95 % | 10.47 % |
| | quiet WER (VAD on) | 9.42 % | 9.42 % |
| base q5_1 | 40-clip WER | 4.50 % | 4.50 % |
| | 40-clip outputs ending in a stock sentence | 3 | 3 |
| | noise: stock phrase / 36 | 16 | 15 |
| | blip: stock phrase / 40 | 1 | 1 |
| | tail: stock tail / 40 | 0 | 0 |
| | tail WER | 3.98 % | 3.98 % |
| | quiet WER (VAD on) | 6.49 % | 6.49 % |
| small q5_1 | 40-clip WER | 2.93 % | 2.93 % |
| | 40-clip outputs ending in a stock sentence | 0 | 0 |
| | noise: stock phrase / 36 | 35 | 36 |
| | blip: stock phrase / 40 | 0 | 1 |
| | tail: stock tail / 40 | 1 | 1 |
| | tail WER | 3.04 % | 3.04 % |
| | quiet WER (VAD on) | 5.03 % | 5.03 % |
| **all** | **stock phrases reaching `hallucination.rs`** | **82** | **89** |

Other numbers:

* **Quiet path.** 1 of 40 clips is `no_speech` on every model, before and after. The
  quiet clips decode exactly as before.
* **Leaks past the guard with the VAD off.**
  * base q5_1: 33 → 28 and small q5_1: 32 → 27. A few garbage sentences on noise became
    a bare ".".
  * Every multilingual noise window still gives text, before and after. Sampling at a
    higher temperature reshuffles that text; the count does not drop.
  * tiny.en leaks nothing either way, because its `no_speech_prob` works.
* **Blips.** The 250 ms blips give text on 34–38 of 40 windows on every model, before
  and after. This is mostly a guess at the syllable, not a stock phrase. The VAD's
  `MIN_RUN` and the D16 `speech_ms` evidence are what handle these.
* **Speed.** ASR totals are within the noise of a shared box: tiny 22.8 → 26.0 s,
  others ±0.7 s. The full-window re-decode counts are unchanged (0 / 6 / 2 / 0).

## Not covered

* large-v3-turbo, used on the GPU (D84), was not run. It is too slow on this CPU box.
* No real-world dictations were used. OpenWhispr's 2.25 % figure comes from their field
  data. We can only measure tails on public clips plus synthetic silence, where the
  VAD-on path already produces almost no tails (0–1 of 40 on the `tail` case, before
  and after).
