# Proposal: release→text that does not grow with dictation length (D22 candidate)

Status: **implemented** on branch `feat/streaming-asr` (2026-10-23): Phase 0,
Phase 1 and Phase 2 in the crates; shells (Phase 3) work from
`docs/proposals/streaming-asr-shell-api.md`. Measured before/after: §8.
Differences from the design below are listed in §9. Originally: research + design, 2026-10-23. Probe:
`tools/probes/prototypes/segmented_asr_2026_10_23.py` (dated). Raw rows:
`$CACHE/segmented-2026-10-23/20261023-152738/rows.jsonl`
(`$CACHE` = the local bench cache, outside the repo; A+B+C) and `.../20261023-155524/rows.jsonl` (A+C rerun; `20261023-152113` is a void first run that overflowed the CLI's ring).

**Measurement caveat.** This box was at load 10-29 on 8 threads during every
run (other agents: Gradle, rustc, an STT survey). Absolute times here are
inflated by about 1.5-3×, and whisper more than Parakeet. Read the *ratios*
and the *WER*, which do not depend on load. Latency forecasts use the
user's S25+ numbers and `docs/model-benchmarks.md`.

## 1. Where the time goes today

### 1.1 `total − asr` (llm = 0): the commit waits for a live-preview decode that is still running

The work between `MurmurCore::commit()` (t0) and the result (`engine.rs`):
1. snapshot, then `Ctrl::Commit` is sent to the **single** worker thread;
2. the worker drains the ring (i16→f32), then runs `vad::detect` over the whole buffer;
3. ASR (`asr_ms` is timed here and covers only this step);
4. spoken cleanup, post-processing, paste segments.

Steps 1, 2 and 4 together take 2-23 ms on this box, even for 44 s clips
(part A with partials off: `total-asr` = 2-23 ms; at most 151 ms once under
load 29). Resampling never happens in the core: the shells push 16 kHz. The
D20 audio_ctx fit is part of `asr_ms`, and so is the repetition guard's
re-decode. None of these explains up to 0.9 s.

The cause is the **live preview**. Android ships `livePreview = true` with
`PARTIAL_INTERVAL_MS = 1500` (`DataLogic.kt:49,64`). Every 1.5 s the worker
`tick()` decodes the trailing `PARTIAL_WINDOW = 30 s` of the buffer. The
Commit message waits in the worker's queue until that decode returns.
`commit()` does set `stop_partial` (the abort flag), but:

* **whisper**: whisper.cpp (whisper-rs-sys 0.15, `src/whisper.cpp`
  `whisper_encode_internal`) runs the conv, encoder and cross graphs through
  the scheduler without the abort callback. The callback is checked only
  *after* the encoder and between decoder steps. The encoder is the
  expensive part, so it always runs to the end.
* **Parakeet**: `ParakeetAsr::transcribe` checks the flag only *before*
  it starts (`parakeet.rs`: "sherpa-onnx decodes cannot be interrupted").
  An in-flight partial always runs to completion.

On a long dictation the partial covers nearly the whole clip, so it costs
about as much as the final decode, and partials run back-to-back. The wait
is therefore uniform on [0, one encoder pass]. That fits the field data:
12.0 s → 0 ms, 9.2 s → 332 ms, 4.6 s → 112 ms, 28.1 s → 891 ms (random phase,
growing with length).

Reproduced on this box (part C: real-time push, partials at 1500 ms vs
off; same clip, same engine):

| engine | clip | gap with partials | gap without |
|---|---|---|---|
| Parakeet | 24.4 s | 55 / 752 ms | 2 / 5 ms |
| Parakeet | 29.9 s | 606 / 3138 ms | 12 ms |
| whisper base q5_1 | 24.4 s | 1932 / 15 ms | 3 ms |
| whisper base q5_1 | 29.9 s | 41 / 500 ms | 2 ms |

**Remote route.** The `total_ms` the phone shows for a server result is the
**server's** `MurmurCore::commit` wall time (`ws.rs` → `ServerMsg::Result`;
the router does not overwrite it). It does **not** include network upload
or RTT. The shell copies its `SessionConfig`, including
`partial_interval_ms`, to the server, so the server runs Parakeet partials
too. They cannot be interrupted, which explains the 760 ms on 27.2 s.
Upload after stop is small by construction: audio streams in real time,
and the writer is one FIFO (`conn.rs`), so commit can never overtake
audio. Nobody measures it today, because no client-side wall clock is
reported.

### 1.2 Nonlinearity

* **Parakeet is linear in clip length**: RTF 0.09-0.16 from 6 to 62 s
  (part A, both runs), and the curve has no knee. So the 27.2 s → 2479 ms
  point (RTF 0.091) is normal for an i7-6700. The outlier is **11.6 s →
  155 ms** (RTF 0.013, about 10× faster than any Parakeet decode measured
  here). The likeliest explanation is that the VAD trimmed that clip to a
  few seconds of real speech. `asr_audio_ms` would show it, but the
  history and the server log do not record it. **Ask:** the server log
  line for that dictation, or the history row's `asr_audio_ms` once it is
  logged (Phase 0 adds it).
* **whisper**: it has a real knee at the D20 boundary. From 29 s of
  padded audio up, `audio_ctx_for` returns 0, which means the full
  1500-position window. Above 30 s whisper.cpp runs one more 30 s window
  per 30 s of audio. Box run 1: 29.9 s 4.5 s → 33.9 s 6.5 s (+43% for
  +13% audio). On the phone, 15 s → 1144 ms vs 23.8 s → 3511 ms (×3 for
  ×1.6 audio) has two more causes. Encoder attention is quadratic in
  `audio_ctx` (800 → 1240 positions). Thermal throttling also builds up,
  because back-to-back 30 s-window partials have kept the big cores busy
  for 20+ s. Per-second cost is lowest on 5-12 s pieces (phone: 2.7-12 s
  clips have RTF 0.05-0.07).

### 1.3 New bug found: Parakeet returns **empty** on a 41.7 s clip

`long_040s.wav` (41.7 s, 7 LibriSpeech utterances joined with 700 ms gaps)
gives `""` → EMPTY `blank_asr` **3/3 runs** with the shipped config
(`modified_beam_search`, beam 4, bpe vocab for hotwords). With
`MURMUR_PARAKEET_HOTWORDS_SCORE=0` (greedy) it gives the correct text. The
33.9, 34.3, 44.5 and 62 s clips are fine, so the trigger is not length
alone. A user dictating for ~40 s through the server **loses the whole
dictation**. The same clip cut at pauses transcribes fine (§2, WER 1.1-2.3%).
Independent fix (Phase 0): if the beam result is empty and the VAD saw at
least 1 s of speech, re-decode in ≤ 20 s pieces at the longest pauses
(the same segmenter as below), or keep a greedy recognizer as the
fallback. The CLI artefact: `murmur-cli` pushes the whole file at once, so
past 30 s it overflows the default 30 s ring (`dropped_samples`).
`ring_capacity_samples` raised to 130 s for the probe; shells push in
real time and are not affected.

## 2. Evidence for pause-segmented transcription

Part B: each long clip is cut at energy pauses ≥ P ms (the middle of the
pause; 20 ms frames, floor×3; a stand-in for the D16 streaming VAD). Every
piece is transcribed **independently, with no prompt carry-over**, the
texts are joined with a space, and WER is scored against the reference
(LibriSpeech normaliser, `bench/wer.py`). 8 clips, 19.5-62 s, 741 words:

| engine | P | pooled WER segmented | whole clip | note |
|---|---|---|---|---|
| Parakeet | 800 ms | **1.38%** | 1.38% | without the 41.7 s clip (seg 2.25%, whole 100%) |
| Parakeet | 500 ms | 1.69% | 1.38% | |
| whisper base q5_1 | 800 ms | **4.05%** | 4.05% | |
| whisper base q5_1 | 500 ms | 4.05% | 4.05% | |

* With P = 800 ms there are 2-8 segments per clip, the longest 17.8 s. The
  WER is identical to the whole clip for both engines, and segmenting
  **fixes** the 41.7 s Parakeet failure.
* Segment errors are rare words that lose context ("Meadowcroft" →
  "Meadocroft", "Timaeus" → "Timias"). Segmenting also fixed a few
  whole-clip errors (e.g. a "Thank you." appended to a 44 s clip's
  tail). **No cut word appeared**,
  because the cut sits in the middle of a ≥ 800 ms pause. whisper did
  append "thank you" to one 9 s segment, so a per-segment
  hallucination guard is needed (§3.4).
* P = 500 makes more, shorter pieces (down to 1.4 s), loses context, and
  costs Parakeet +0.3 pt. Use 800 ms, with a minimum segment of 4 s.

## 3. Design (recommended): core-side pause segmentation, "segment-ahead"

One mechanism inside `MurmurCore`, opt-in per session through
`SessionConfig`. It works the same way in-process, on murmur-server (which
runs the unmodified core), and on fallback replay, so the router and the
wire protocol need no message changes.

### 3.1 Config (additive, `#[serde(default)]`)

```rust
/// D22: decode finished speech segments while the user is still talking.
/// None = off (today's behaviour, byte-identical).
pub segment_pause_ms: Option<u32>,      // recommended 800
pub segment_min_ms: u32,                 // 4000: shorter pieces keep growing
pub segment_soft_max_ms: u32,            // 20000: then cut at the longest gap ≥ 200 ms in the last 5 s, if any
```
Older servers ignore the field (no `deny_unknown_fields` on
`SessionConfig`). They behave exactly as today, and the result is still
complete.

### 3.2 Segmenter (new `murmur-core/src/segment.rs`)

* It is a streaming detector over the f32 buffer the worker already
  holds. It reuses `vad::frame_stats` and `Thresholds`, like `endpoint.rs`,
  with a noise floor over the last 30 s.
* It emits cut sample indices. A cut is placed at `speech_end + P/2` once
  the current pause reaches P **and** the open segment is ≥ `segment_min_ms`.
* **It must be a pure function of the audio**, so cuts cannot depend on
  push chunking, drain timing or decode speed. The worker runs it over
  newly drained samples; `finalize` runs it over the rest. This is what
  keeps the equivalence suite and fallback replay identical across routes.

### 3.3 Worker scheduling (`engine.rs`)

`Session` gains `cuts: Vec<usize>`, `done: Vec<SegOut>` (text, language,
no_speech_prob, speech_ms) and `next: usize` (the first sample that is not
decoded yet). The worker is still the only thread; no second ASR state is
added, because the phone has no RAM or cores to spare for one.

On each `tick()`, priority order:
1. **closed segment pending** (`cuts` beyond `next`): VAD-trim it, then
   `transcribe(seg, req{purpose: Segment, prior_text})`, push it to
   `done`, and advance `next`. A segment with no speech is skipped.
   `abort` = the session's cancel flag.
2. otherwise, the **partial**, and only over the *open* segment
   `[next, len)` (≤ soft max ≈ 20 s, not 30 s). Preview =
   `join(done) + " " + partial`.

`finalize`: drain, run the segmenter over the rest, decode the closed
segments that are still pending, then decode the **tail** `[next, end)`
(VAD-trimmed; skipped if silent). Join, then run the guard, spoken
cleanup, layout, the optional LLM and post-processing **on the joined
text**, unchanged. `asr_text` = joined raw text.
`timings.asr_ms` = ASR time spent *inside* commit. New additive timings:
`segments`, `pre_asr_ms` (decoded before release), `wait_ms` (commit
queued behind the worker), and `asr_audio_ms` (already present) logged by
the server and history.

### 3.4 Details

* **Prompt carry-over (whisper)**: `AsrRequest.prior_text: Option<String>`
  = the last ~30 words of `done`, appended to the dictionary
  `initial_prompt` (whisper caps the prompt at 224 tokens; keep dictionary
  terms first). It keeps casing and punctuation consistent across
  segments ("…and then" rather than a new "And then" sentence).
  `no_context` stays true. Part B shows WER parity *without* it, so it
  only serves style. Gate it behind the B-bench: if it ever creates a
  loop, the D20 repetition guard triggers the full-window re-decode as
  today. Parakeet ignores `prior_text`; hotwords are unchanged per segment.
* **Join**: one space. When a segment ends without `[.?!]` and the next
  starts with a capital that is not "I" and not a dictionary word,
  lowercase it (whisper only, and only when `prior_text` is off). Spoken
  commands split by a pause ("new … line") are unlikely at P ≥ 800 ms.
  Add a spoken-cleanup vector so "new. Line" still means a new line.
* **Hallucination guard per segment**: drop a segment that is *only* a
  stock phrase, judged on that segment's own speech evidence. Also strip a
  trailing "Thank you." from a segment that is not the last one, when
  `no_speech_prob` is high. Run today's whole-utterance guard on the join.
* **Word cuts**: none by construction, because the cut is in the middle
  of a ≥ 800 ms pause. The soft max cuts only at a ≥ 200 ms gap. If the
  segment has none, it keeps growing (today's behaviour). There is no
  overlap or dedupe logic.
* **Cloud (D18)**: `AsrEngine::segmentable() -> bool`, default true, and
  `CloudAsr` returns false. The core then ignores `segment_pause_ms`. This
  keeps C5 "no upload before commit" and one billed call per dictation.
* **Router fallback (D17)**: no router change. The local core receives
  100% of the replayed samples. Its segmenter finds the same cuts, so the
  text is identical to a local-only run. A replayed backlog is decoded
  in bulk, which at worst equals today's latency and never loses text.
* **Cancel (✕)**: `cancel()` sets the session abort flag and sends
  `Ctrl::Cancel`, and the worker drops `Session` (`done` with it). An
  in-flight segment decode aborts at the next check (whisper: after the
  encoder; Parakeet: when it finishes) and its output is thrown away.
  Nothing escapes, because segment text lives only in `Session` and the
  preview string, and `clear_partial(id)` already runs. Server side:
  `ClientMsg::Cancel` drops the session the same way. The cost is that a
  `start_session` right after ✕ may wait up to one segment decode in the
  worker queue (≤ ~1 s on the phone, ≤ 2 s on the server).
* **CPU contention / battery (phone)**: today, with the preview on, a
  28 s dictation decodes about 10× its audio (a growing 30 s window every
  1.5 s, back to back). Segment-ahead decodes each closed second once,
  plus partials over a ≤ 20 s open segment, roughly 3-4× in total: **less
  CPU and heat than today**. That reduces the throttling seen in §1.2.
  With the preview off it is ~1×, the same work moved earlier, and only a
  cancelled dictation wastes any. Capture is unaffected: `push_audio` is
  lock-light and non-blocking, the ring holds 30 s, and one segment decode
  is ≤ ~1.5 s. Keep `threads ≤ big cores − 1` so the audio thread keeps a
  core (shell setting).

### 3.5 Alternatives rejected

* **Streaming ASR models** (sherpa online zipformer, NeMo cache-aware
  FastConformer): constant latency by design, but a worse WER than
  Parakeet-TDT v2, a new model to download and package (D9), and no
  whisper equivalent for the multilingual path. Segment-ahead keeps both
  current engines and their measured WER.
* **Whisper's own seek / no_context=false over a growing buffer**:
  re-decodes everything at commit, so it is still linear.
* **A second ASR state for partials**: doubles RAM on the phone (whisper
  KV and compute buffers; Parakeet ~640 MB). It hides the partial wait
  but not the linear final decode.
* **Fixed-length chunks with overlap**: cuts words, needs text dedupe, and
  is worse than pause cuts at equal latency.

### 3.6 Hands-free endpoint: 2000 → **1500 ms**, not 1200

With segment-ahead, the last speech run is cut at P/2 = 400 ms into the
final pause and decoded during the remaining endpoint wait. By the time
the endpoint fires (2000 ms), a tail of ≤ 1.4 s / RTF is already
decoded. Phone whisper base at RTF ~0.06 covers a ~20 s last segment;
the server's Parakeet covers ~12 s. So hands-free release→text becomes
≈ `endpoint_silence_ms` + ~50 ms, **whatever the dictation length**. That
is ≈ 2.0 s instead of today's ~6 s, without touching the endpoint. The
endpoint then becomes the *only* latency, which makes lowering it
worthwhile. 1500 ms saves another 0.5 s. 1200 ms starts to end
dictations on thinking pauses: mid-monologue hesitation pauses of
1-2 s are common (D19). Recommendation:
default 1500 ms after Phase 1 ships, keep the 300-10000 setting, and log
the longest mid-dictation pause per session (local history only), so the
default can be tuned on the user's own data.

## 4. Phased plan

**Phase 0: quick fixes, core only (S), shippable alone**
1. Cap the partial window: `PARTIAL_WINDOW` 30 s → 12 s, with a preview
   prefix "…". The commit wait becomes ≤ one 12 s decode (phone ≈ 0.7 s,
   about 0.3 s on average), and phone heat drops.
2. Skip a partial when the endpointer or VAD shows ≥ 400 ms of trailing
   silence. The user is probably about to release, and the decode would
   only block the commit.
3. Parakeet empty-on-speech fallback (§1.3), plus a test on `long_040s`.
4. Add `asr_audio_ms` and `wait_ms` to the server log line and the Android
   history. Add a router-measured `route.client_ms` (wall time of
   `Router::commit`) so the remote total includes the network.

Expected: local 28 s from 3.8 s → ≈ 3.0 s; the remote gap from 0.76 s →
≤ 0.4 s. Still linear.

**Phase 1: segment-ahead in murmur-core (S)**: `segment.rs`, `engine.rs`
scheduling and finalize, the config fields, `AsrRequest.prior_text`,
`AsrEngine::segmentable`, the whisper prompt, per-segment guard, new
timings, murmur-ffi header/JSON docs, and the core-architecture "Changelog
for shells".

**Phase 2: server / protocol (S)**: no message change. Optional, additive:
`HelloOk.features: ["segments"]` (serde default `[]`) so the Developer
row can show it. Golden fixtures for the new `SessionConfig` and
`Timings` fields. The server logs `segments` and `pre_asr_ms`. Contract
tests in §5.

**Phase 3: shells**
* **A** (`apps/murmur-android`): `MurmurCore.kt` SessionConfig
  `segmentPauseMs`, `CoreJson.kt` (de)serialisation, `DataLogic.kt`
  default 800 ms (Developer toggle "Decode while speaking"), endpoint
  default 1500. History shows `segments` / `wait`. Preview uses the
  partial as before (the core already joins it). Tests:
  `DataLogicTest`, `CoreJsonTest` fixtures.
* **W** (`apps/murmur-windows`): `core_api.rs` and `core_adapter.rs` map
  the field, `prefs.rs` default 800 / endpoint 1500, overlay unchanged.
  Tests in `core_adapter.rs`.

**Phase 4 (later; not built): a streaming engine.** The STT survey
(`docs/proposals/stt-model-survey.md`) measured Nemotron-speech-streaming-en
0.6b (sherpa-onnx `OnlineRecognizer`, 560 ms chunks) at 150-330 ms
release→text for any length, with WER 2.2% (Parakeet v2: 1.15%). It would
slot in as an engine that is fed from `push_audio` while the user speaks.
Such an engine returns `false` from `AsrEngine::segmentable()`, so the D22
cuts never apply to it. It needs a new trait method (or a sibling trait)
for incremental accept/finish; the session config, protocol and shells
stay as they are. Its cost is a continuous ~40% of 4 threads while
speaking.

## 5. Tests

Core (S):
* `segment.rs` units: cuts identical for bulk push vs 10 ms / 40 ms /
  1 s chunks (property test over random chunkings); no cut before
  `segment_min_ms`; soft max cuts only at ≥ 200 ms gaps; silence-only →
  no segments.
* `session.rs` with a mock ASR that sleeps `RTF × len` and echoes segment
  ids: (a) `segment_pause_ms: None` gives byte-identical results on all
  existing session tests; (b) **constant latency**: commit time on 10 s
  vs 60 s of speech with pauses differs by < one tail decode; (c) cancel
  during an in-flight segment → no result, no partial, the next session
  is clean; (d) cancel during commit → ERROR `cancelled` (C4 unchanged);
  (e) preview = done segments + open partial, cleared on commit/cancel.
* **Equivalence bench** (ignored, like `audio_ctx_bench.rs`):
  `tests/segment_bench.rs` builds the probe's long clips (19-62 s,
  700 ms gaps) from `asr_manifest_extended.json`. Real Parakeet and
  whisper base, segmented vs whole-clip WER. Gate: pooled segmented WER
  ≤ whole + 0.3 pt per engine (today: equal at P = 800). It also
  reports tail and wait times. Plus: natural (un-padded) LibriSpeech
  chapters ≥ 30 s, if fetched.
* Regression: `long_040s` on Parakeet is not EMPTY (Phase 0).

Contract (S, `murmur-contract-tests`):
* `equivalence_segmented`: the 119 fixtures with `segment_pause_ms` set
  give identical results locally and through the server (mock and real
  Parakeet on the long clips).
* `fallback_replay_segmented`: a 60 s segmented session whose server dies
  at 50 s gives local text == local-only text, 0 dropped.
* Old server (field ignored) still gives the full text.
* The cloud mock with `segment_pause_ms` set makes exactly one upload, at
  commit (C5).

## 6. Expected latency (release → text, PTT, LLM off)

| route | dictation | today (field) | Phase 0 | Phase 1 |
|---|---|---|---|---|
| phone whisper base | 5 s | 0.2-0.4 s | same | same |
| phone whisper base | 25-30 s | 3.5-3.8 s | ≈ 3.0 s | **≈ 0.4-0.9 s** (tail 2-6 s at RTF 0.06-0.07 + ≤ 0.3 s average wait) |
| server Parakeet (i7-6700) | 27 s | 3.2 s (+ network) | ≈ 2.7 s | **≈ 0.4-0.7 s** (tail ≈ RTF 0.1 × 3-5 s + RTT) |
| hands-free, either | 25 s + 2 s endpoint | ~6 s | ~5 s | **≈ 2.0 s** (1.5 s with endpoint 1500) |

The worst case is a release while a ≤ 20 s segment decode is still
running: + RTF × 20 s ≈ 1.3 s on the phone and 2 s on the server. That
needs a pause right before a very short final phrase, and is still
better than today's typical case. These are forecasts until a device run
records `segments` / `pre_asr_ms` / `wait_ms`.

## 7. DECISIONS row (applied as D22 in docs/DECISIONS.md)

| D22 | Release→text latency for long dictations (2026-10-23) | Opt-in `SessionConfig.segment_pause_ms` (recommended 800; `segment_min_ms` 4000, `segment_soft_max_ms` 20000). The core's worker cuts the buffer at pauses with a streaming D16 VAD (cuts a pure function of the audio), decodes each closed segment while the user keeps talking (priority over partials, which shrink to the open segment), and at commit decodes only the tail, joins, then runs guard / spoken cleanup / LLM / post-processing on the joined text. whisper gets the previous segment's last words as prompt; engines that must not pre-upload (cloud, D18) opt out. Runs inside MurmurCore, so murmur-server, router fallback replay and the equivalence suite need no protocol change. Hands-free endpoint default 2000 → 1500 ms. Phase 0 also caps the partial window at 12 s and adds a Parakeet empty-on-speech fallback | Field (S25+): `total − asr` up to 0.9 s is the commit queued behind an unabortable live-preview decode (whisper encoder, any sherpa decode); ASR is linear-to-superlinear in length (3.5-3.8 s at 24-28 s). Probe `tools/probes/prototypes/segmented_asr_2026_10_23.py`: on 8 clips of 19-62 s, pause-segmented WER equals the whole clip (Parakeet 1.38%, whisper base 4.05% at P = 800 ms), and Parakeet beam search returns EMPTY on a 41.7 s clip that segmenting fixes. Forecast: release→text ≈ 0.4-0.9 s at any length, hands-free ≈ endpoint + 50 ms |

## 8. Measured on this box (2026-10-23, after implementation)

`tools/probes/prototypes/streaming_latency_2026_10_23.sh`: the
`hybrid_latency` example pushes each clip in 100 ms frames at real-time
pace, with the Android shell's preview setting (`partial_interval_ms`
1500), then times `commit` (release → text). It runs both in-process
(local Parakeet v2 int8, 4 threads) and through Router → murmur-remote → a
test murmur-server on 127.0.0.1 (one worker, same model). Clips are the
probe's LibriSpeech joins (700 ms gaps). "before" = base commit `0d61885`;
"P0" = this branch without segmentation; "D22" = this branch with
`segment_pause_ms: 800`. Raw TSVs and server logs:
`$CACHE/streaming-latency-2026-10-23/20261023-173931/`.
The box was shared with other agents (load varied during the run), so read the ratios.

| audio | before local / server | P0 local / server | **D22 local / server** | D22 server log: pieces, wait, tail asr, pre-asr |
|---|---|---|---|---|
| 24.4 s | 2755 / 3042 ms | 2464 / 2400 | **1736 / 1642** | 2, 427 ms, 1206 ms, 448 ms |
| 29.9 s | 5245 / 5101 | 2938 / 2458 | **1300 / 1154** | 3, 0, 1149, 1082 |
| 33.9 s | 3989 / 3987 | 3420 / 3306 | **893 / 838** | 4, 75, 758, 1885 |
| 41.7 s | 4685 / 4636, **text EMPTY** | 3211 / 3376 | **687 / 1482** | 4, 700, 775, 2526 |
| 44.5 s | 5632 / 5868 | 3461 / 3434 | **1398 / 1370** | 5, 284, 1076, 2327 |
| 62.0 s | 7567 / 7769, lowercase, no punctuation | 5279 / 5190 | **609 / 521** | 8, 0, 518, 4505 |
| mean | 4978 / 5067 | 3462 / 3360 | **1103 / 1167** | |

With the bridge fix (§9, last bullet; re-run on the same clips, raw TSV in
`…/streaming-latency-2026-10-23/20261023-201014-bridge/`): local / server
2253 / 1693, 1044 / 1136, 1272 / 3542, 1646 / 1165, 699 / 1097,
497 / 582 ms; mean **1235 / 1535 ms**. That is about +0.1-0.4 s of bridge
work. The one outlier (29.9 s via the server, 3.5 s) is a tail bridge
decoded at commit on a loaded box. Still flat in length.

* The text is the same local and remote in every row (`same_text` true).
  Before, the 41.7 s clip came back EMPTY on both routes (beam search), and
  the 62 s one lost its casing and punctuation. P0 and D22 fix both.
* With D22, release→text no longer grows with length. What remains is the
  **tail**: the speech after the last pause of ≥ 800 ms, plus at most one
  piece in flight (`wait_ms` ≤ 700 ms here). These LibriSpeech joins have a
  pause only every 6-17 s, so the tail is one whole utterance. Real
  dictation pauses more often. The 24.4 s clip's last utterance is 17 s
  long, so it gains least.
* P0 alone (12 s preview window, no preview re-decode in silence, Parakeet
  pieces past 20 s) takes 30% off before any segmentation.
* `route.client_ms` is the "remote" number above. The server's own
  `total_ms` (log) is 0-70 ms lower: loopback network plus the client side.
* WER, `tests/segment_bench.rs` (same clips, 743 words, our normaliser):
  Parakeet whole 2.02% → segmented 1.48%; whisper base q5_1 5.92% → 5.38%.
  The gate (segmented ≤ whole + 0.3 pt) passes for both engines.

## 9. Implementation notes (differences from §3-§4)

* `segment_min_ms` / `segment_soft_max_ms` are `Option<u32>` (`None` = 4000
  / 20000), so a config without them serialises exactly as before.
* Parakeet, independent of D22 (the survey's root cause): decodes without
  hotwords run greedy; an empty or < 0.5 words/s beam result is retried
  greedy; and clips over 20 s are always decoded in ≤ 20 s pieces at their
  longest pauses, because one-shot greedy also dropped whole sentences (the
  17 s third utterance of a 30 s clip). This roughly doubles Parakeet RSS
  (two recognizers). `ModelConfig.parakeet_decoding = "greedy"` keeps one.
* A segment decoded while recording is reused at commit only if its
  request (prompt, prior text, language, terms) still matches. A late
  `set_context` that changes the prompt makes commit re-decode that piece,
  so the result never depends on when the context arrived.
* The "lowercase the next segment" join rule (§3.4) is not implemented:
  whisper gets `prior_text`, and Parakeet's pieces end at real pauses. A
  spoken command split by a pause ("new. Line") is covered by a
  spoken-cleanup test.
* No reuse of an in-flight preview as the tail: a commit still waits for
  one decode (`wait_ms`), bounded by the 12 s preview window or one piece.
* **Punctuation across cuts (fix, 2026-10-23, after agent W's report).**
  Each piece was punctuated on its own, so a mid-sentence pause became a
  sentence break (Parakeet on jfk: "ask not. What your country"). The WER
  bench missed it because it ignores punctuation. Now every cut also gets
  a **bridge** decode of the audio 4 s either side of it. It is decoded
  while recording once 4 s follow the cut (after the closed pieces, before
  previews), and at commit only when the tail is shorter. `segment::splice`
  aligns the bridge with the last/first 6 words of the two pieces. Anchors
  are exact matches, never the bridge's edge words. The span between the
  anchors takes the bridge's punctuation and case. Words may change only
  at the 2 positions next to the cut ("asked not" → "ask not" for whisper
  tiny), and any other mismatch keeps the pieces unchanged. The same
  splice joins Parakeet's ≤ 20 s internal pieces. Measured with
  `tests/segment_bench.rs::segmented_punctuation_matches_whole_clip`:
  punctuated-token error against the whole-clip output, with exact case
  and punctuation, for three joins. "plain" = before the fix; "smooth" =
  a text-only heuristic (drop the period and lowercase the next word);
  "bridge" = shipped.

  | engine | set | plain | smooth | **bridge** |
  |---|---|---|---|---|
  | Parakeet | jfk @ 800 ms | 4.55% (0/1 exact) | 4.55% | **0.00% (1/1)** |
  | Parakeet | 35 utterances cut at 300 ms / 2 s (59 mid-sentence cuts) | 7.32% (10/35) | 8.35% | **5.15% (14/35)** |
  | Parakeet | 8 long clips @ 800 ms (cuts at real sentence ends) | 3.23%, 34/36 ends | 6.99%, 20/36 ends | **2.69%, 36/36 ends** |
  | whisper base | jfk | 4.55% | 4.55% | **0.00%** |
  | whisper base | 35 utterances, 300 ms / 2 s | 14.04%, 93 ends vs 52 | 12.18% | **9.16%, 66 ends** |
  | whisper base | 8 long clips | 4.10% | 8.61% | 4.92% |

  Smoothing destroys real sentence ends (20/36). Bridging wins everywhere
  except whisper on the long clips (+0.8 pt). There the "whole" reference
  is itself a > 30 s whisper decode with its own seek boundaries. Word WER
  gate after the fix: Parakeet whole 2.15% / segmented 1.62%, whisper
  5.92% / 5.38% (the Parakeet whole number moved from 2.02% because
  > 20 s whole decodes now splice too). Regression tests:
  `parakeet_e2e::parakeet_segmented_jfk_equals_whole_clip`,
  `whisper_e2e::segmented_jfk_equals_whole_clip` (exact equality),
  `segment.rs` splice units (including a slipped alignment that once
  duplicated "the man who told"). Cost: one extra decode of ≤ 8 s per
  cut, almost always while recording.

* **Join polish (fix, 2026-10-23, after the cliptest on Parakeet-unified
  int8 + whisper base, D25).** Four defects at 800 ms: a lower-case
  sentence start after a cut ("capacity. the Englishman", both engines:
  the bridge started at "Capacity," and its edge word "the" was copied;
  "figures. that is": the splice was refused because the bridge said
  "ill-drawn"), a sentence end lost at a cut ("tender flowers the pain":
  the 4 s left side of the bridge was too short for Parakeet-unified to
  hear the sentence end; 6 s and 8 s hear it), and no closing period
  (Parakeet-unified often ends even whole clips open). Fixes: the 2
  words at each bridge edge keep the pieces' case and punctuation; every
  join (and every spliced span) capitalises the word after `.`/`?`/`!`;
  the bridge covers 6 s before the cut (4 s after, so it is still
  decoded 4 s after the cut while recording); the engine closes an open
  dictation of ≥ 4 words before the terminal-punctuation pass (`postprocess::terminal`). The
  trailing space in `final_text` is the post-processing paste-spacing rule and stays.
  Bridge left side, `segment_bench::segmented_punctuation_matches_whole_clip`
  (punctuated-token error vs the whole clip, `bridge` mode; Parakeet-unified
  int8, so the absolute numbers differ from the table above):

  | engine | set | left 4 s | **6 s** | 8 s |
  |---|---|---|---|---|
  | Parakeet-unified | utterances 300 ms / 2 s | 5.26% | **4.81%** | 4.81% |
  | Parakeet-unified | 8 long clips @ 800 ms | 3.49% | **2.42%** | 3.09% |
  | whisper base | utterances | 9.40% | 13.23% | 13.23% |
  | whisper base | 8 long clips | 4.51% | **4.37%** | 4.92% |

  whisper's `plain` (no bridge) baseline on the utterances also moved
  14.04% → 17.40% between the 4 s and 6 s runs, so that row is run-to-run
  noise under load (bridge vs plain: −4.6 and −4.2 points). Cliptest
  (12 clips, 513 words, `tools/probes/prototypes/seg_join_2026_10_23/`),
  before → after: Parakeet seg WER 1.56 → 1.36% (whole 1.75%),
  seg-vs-whole punctuation agreement 97.7 → 99.0%, lower-case after a
  sentence end 2 → 0, open endings seg/whole 8/6 → 0/0 of 12; whisper
  WER 2.34% (unchanged), agreement 97.4 → 98.0%, lower-case 1 → 0, open
  endings 0/1 → 0/0 (paced run, same text as an unpaced one). Cost: each
  bridge decodes up to 2 s more audio while recording; release→text was
  not re-measured (load average 13-46 during the run, vs 4-11 for the
  original cliptest). Regression test:
  `parakeet_e2e::parakeet_segmented_joins_keep_sentence_case_and_close`.
