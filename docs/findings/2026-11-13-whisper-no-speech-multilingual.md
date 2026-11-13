# 2026-11-13: Whisper `no_speech_prob` is about 0 on multilingual models (D91)

**Problem.** The decoder-thresholds bench (`2026-11-13-whisper-decoder-thresholds.md`,
branch `feat/whisper-thresholds`) found that our `no_speech_prob` was at most 0.001 over
342 decodes on tiny, base q5_1 and small q5_1, while tiny.en reached 0.89. With the VAD
off (the Windows GPU plugin, the server), `hallucination.rs` then had no no-speech
evidence from Whisper on multilingual models.

**Answer: a whisper.cpp bug, fixed on our side.** `WhisperEngine` now reads the
probability itself, at `<|startoftranscript|>`, once per encoder window. whisper.cpp is
not patched. The hallucination guard now also needs a low avg log-prob, as OpenAI does,
which keeps a real short "you" but catches only 3 of 18 noise windows on tiny.

## Root cause

whisper.cpp 1.8.3 (vendored by `whisper-rs-sys` 0.15.0), `whisper_full_with_state`:

1. It builds the prompt as `[prev, initial_prompt…]` + `sot` + (multilingual only)
   language + `transcribe`, and decodes it in one batch.
2. `whisper_batch_prep_legacy` asks for logits of the **last** prompt token only.
   `whisper_decode_internal` stores them at row `n_tokens - 1` of `state->logits`.
3. The no-speech block then runs `whisper_compute_logprobs(state->logits, …)`, which
   reads **row 0**, and takes `probs[whisper_token_nosp(ctx)]`.

Row 0 holds this decode's logits only when the prompt is one token long: `sot` alone,
which is an English-only model with no prompt. In every other case it holds stale
logits from an earlier one-token decode step, and the probability of `<|nospeech|>`
there is about 0:

* every multilingual model (prompt `sot`, language, `transcribe`);
* an English-only model with a dictionary or `prior_text` prompt (D22). The bench did
  not see this because it ran without a prompt.

OpenAI Whisper reads the value at the `sot` position (`logits[:, sot_index]` in
`decoding.py`). Even with the row fixed, whisper.cpp would read it at the task token,
not at `sot`.

Our code (`murmur_core::whisper`) took the per-segment value through
`whisper_full_get_segment_no_speech_prob` (whisper-rs `no_speech_probability()`), so the
API call was right. The value behind it is wrong.

## Fix

A new-segment callback runs once whisper.cpp has decoded a window, while that window's
encoder output is still in the state. On the first segment of each window it decodes
`sot` alone (`whisper_decode_with_state`, one decoder step) and takes
softmax(logits)[`<|nospeech|>`]. `no_speech_prob` is the maximum over the windows that
gave text, as before. An encoder-begin callback marks each new window. A window that
gave no text (the near-empty tail window after the last timestamp) is not read.

Then the probe zeroes logits row 0. Otherwise whisper.cpp's broken read in the next
decode would find the probe's values, and its own no-speech drop
(`no_speech_prob > 0.6 && avg_logprob < logprob_thold`) would act on an earlier window.
Zeroed, it reads 1/n_vocab ≈ 0, as before. So whisper.cpp's decoding is unchanged and
only the value we report is new.

The probe decodes `sot` with no prompt before it. OpenAI reads at `sot` *after* the
prompt tokens (`prev` and the initial prompt come first in its sequence), so its value
is conditioned on the prompt; ours is not, and a dictionary prompt cannot move it. For an
English-only model with no prompt the two are the same: on JFK both give 0.0585.

The softmax runs over one logits row of `whisper_n_vocab` floats. whisper.cpp's own
softmax uses `id_to_token.size()`, and `whisper_process_logits` asserts that it equals
`n_vocab`, so the two vocab sizes are the same.

Cost: one decoder step per window that gave text, usually one per dictation (two when
whisper.cpp's tail window also gives text). Desktop CPU, 4 threads, on a shared 8-core
box (ignored test `no_speech_probe_latency`, median of 21):

| model | `sot` decode, per window |
|---|---|
| tiny | 2.8–3.1 ms |
| base.en (the base decoder; multilingual base is not on this box) | 4.8–5.5 ms |

For comparison, the whole decode is 120–310 ms on tiny and 240–1100 ms on base.en (JFK
3 s and 11 s). End-to-end timings with and without the probe differ by less than the
box's noise. The phone was not measured; a decoder step there is slower in proportion,
but still one step against a decode of tens of steps.

`whisper-rs` gets its `raw-api` feature in murmur-core for the callbacks' calls into
whisper.cpp (`murmur-whisper` already enables it). The GPU plugin and the server both
transcribe through `WhisperEngine`, so they get the fix too.

The probe is set up inside each whisper.cpp run, so every pass of `decode_guarded` gets
its own reading: the first decode, the re-decode without the prompt when Whisper recites
it (D87), and the full-window re-decode (D20). The result kept is the one of the pass
whose text is used.

## The guard: OpenAI's two-signal rule

Before this change `hallucination.rs` treated `no_speech_prob >= 0.6` alone as
no-speech. OpenAI's `transcribe.py` needs two signals: a segment is silence only when
`no_speech_prob > 0.6` **and** `avg_logprob < -1.0` (`logprob_threshold`), so a confident
transcript is kept whatever the no-speech probability says. The guard now uses that rule
through one helper, `hallucination::whisper_says_no_speech(no_speech_prob, avg_logprob)`
(`NO_SPEECH_THOLD` 0.6, `LOGPROB_THOLD` −1.0, strict bounds, a missing value is never
silence). The single-decode guard, the segmented guard and the trailing stock-phrase strip
all call it.

`AsrOutput` and `Evidence` carry `avg_logprob`. `WhisperEngine` computes it per window
like OpenAI: the sum of the window's token log-probabilities divided by (tokens + 1),
the +1 standing for the end token that whisper.cpp does not keep. It reports the pair
(`no_speech_prob`, `avg_logprob`) of the window with the highest no-speech probability.
A segmented dictation keeps the pair of the piece with the highest value. The GPU plugin's
JSON gains an `avg_logprob` field. Cloud and other engines report `None`, so the Whisper
rule never fires for them, as before.

## Numbers

`crates/murmur-core/tests/whisper_no_speech.rs`, ignored test `no_speech_before_after`.
"Before" is whisper.cpp's own value with the engine's settings; "after" is the engine.
Inputs are 18 synthetic noise windows (white and brown, RMS above the 0.003 silence
gate, 1.5, 3 and 6 s) and the public JFK clip. The fitted window is on, 4 threads.
Values are taken from windows that gave text. The "caught" column is what the guard now
flags: no-speech > 0.6 and avg log-prob < −1.0.

| model | prompt | noise before: max, ≥ 0.6 | noise after: range, ≥ 0.6 | noise caught by the two-signal rule | JFK before → after (avg log-prob) |
|---|---|---|---|---|---|
| tiny (multilingual) | none | 0.000, 0 / 18 | 0.33–0.76, 6 / 18 | **3 / 18** | 0.000005 → 0.073 (−0.34) |
| tiny (multilingual) | made-up dictionary prompt | 0.000, 0 / 18 | max 0.87, 7 / 18 | **4 / 18** | – |
| tiny.en | made-up dictionary prompt | 0.002, 0 / 17 | 0.87–0.96, 17 / 17 | **0 / 17** | 0.00007 → 0.059 (−0.22) |
| tiny.en | none | 0.891, 2 / 2 | 0.891, 2 / 2 (same) | 0 / 2 | 0.059 → 0.059 |

For tiny.en with no prompt, 16 of the 18 noise windows give no text: whisper.cpp's own
no-speech drop works there, because its value is correct. The two values match.

On multilingual tiny the no-speech value now separates noise (≥ 0.33) from JFK speech
(0.07). Only the short (1.5 s) noise windows reach 0.6; 3 and 6 s noise reads 0.33–0.57.

**The two-signal rule loses most of the noise catches.** Whisper is often confident
about the text it makes up on noise. On tiny the avg log-prob of the noise windows is
spread from −5.4 to −0.15: of the 6 windows over 0.6, three are under −1.0 (−2.40,
−3.27, −4.74) and three are not (−0.57, −0.58, −0.48). On tiny.en with a dictionary
prompt every noise window reads 0.87–0.96 but its text is confident (−0.72 to −0.09),
so none is caught. The thresholds are unchanged (OpenAI's 0.6 and −1.0); choosing
different ones is the owner's call.

## Risk: a short spoken stock phrase with the VAD off

A guard that fires on real speech drops it. Ignored test `quiet_stock_phrase_risk`:
synthetic speech from espeak-ng (en-us), "Thank you." and "you", 0.3 s of silence on
each side, at espeak's level (RMS ≈ 0.044), −20 dB and −30 dB. Engine direct (VAD off),
fitted window. Each cell is no-speech / avg log-prob.

| model | clip | 0 dB | −20 dB | −30 dB |
|---|---|---|---|---|
| tiny | "Thank you." | 0.50 / −0.56, kept | 0.58 / −0.59, kept | silence gate, no text |
| tiny | "you" | 0.61 / −0.54, kept | 0.70 / −0.57, kept | silence gate, no text |
| base.en | "Thank you." | 0.003 / −0.03, kept | 0.001 / −0.07, kept | silence gate, no text |
| base.en | "you" | 0.018 / −0.03, kept | 0.016 / −0.02, kept | silence gate, no text |

With no-speech alone, tiny's "you" (0.61 and 0.70) was dropped. With the two-signal rule
it is kept, because its text is confident (−0.54 and −0.57). "Thank you." stays under 0.6.
base.en is far from both bars. A multilingual base was not available. The −30 dB clips
(RMS 0.0014) are below the engine's 0.003 silence gate and never reach Whisper.

## Not verified

* base q5_1, small q5_1 and large-v3-turbo were not run (not on this box, or too slow
  on its CPU). The bug is in the prompt handling, which is the same for every
  multilingual model, so the fix applies to them too.
* The GPU (Vulkan, CUDA) builds: the probe is an ordinary `whisper_decode_with_state`
  call, so it runs on the GPU there too. This was not run.
* The value on non-English speech (for example the FLEURS he clips) was not measured.
* The risk check uses synthetic espeak-ng speech, not recorded voices.
* Phone latency was not measured (no phone available).
* The 40-clip LibriSpeech tails bench of D88 was not rerun. Its audio is not on this
  box.
* The avg log-prob is computed from whisper.cpp's token log-probabilities after its
  logit filters (suppressed tokens, timestamp rules). OpenAI's come from the same kind of
  filtered distribution, but the two were not compared token by token.
