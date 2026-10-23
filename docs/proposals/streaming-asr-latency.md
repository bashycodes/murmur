# Proposal: release→text that does not grow with dictation length (D22 candidate)

Status: research + design, 2026-10-23. Nothing implemented. Probe:
`tools/probes/prototypes/segmented_asr_2026_10_23.py` (dated). Raw rows:
`~/Claude/research/wispr/.cache/segmented-2026-10-23/20261023-152738/rows.jsonl`
(A+B+C) and `.../20261023-155524/rows.jsonl` (A+C rerun; `20261023-152113` is a void first run that overflowed the CLI's ring).

**Measurement caveat.** This box was at load 10-29 on 8 threads during every
run (other agents: Gradle, rustc, an STT survey). Absolute times here are
inflated by about 1.5-3×, and whisper more than Parakeet. Read the *ratios*
and the *WER*, which do not depend on load. Latency forecasts use the
user's S25+ numbers and `docs/model-benchmarks.md`.

## 1. Where the time goes today

### 1.1 `total − asr` (llm = 0): the commit waits for a live-preview decode that is still running

The work between `FlowCore::commit()` (t0) and the result (`engine.rs`):
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
**server's** `FlowCore::commit` wall time (`ws.rs` → `ServerMsg::Result`;
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
`FLOW_PARAKEET_HOTWORDS_SCORE=0` (greedy) it gives the correct text. The
33.9, 34.3, 44.5 and 62 s clips are fine, so the trigger is not length
alone. A user dictating for ~40 s through the server **loses the whole
dictation**. The same clip cut at pauses transcribes fine (§2, WER 1.1-2.3%).
Independent fix (Phase 0): if the beam result is empty and the VAD saw at
least 1 s of speech, re-decode in ≤ 20 s pieces at the longest pauses
(the same segmenter as below), or keep a greedy recognizer as the
fallback. The CLI artefact: `flow-cli` pushes the whole file at once, so
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

One mechanism inside `FlowCore`, opt-in per session through
`SessionConfig`. It works the same way in-process, on flow-server (which
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

### 3.2 Segmenter (new `flow-core/src/segment.rs`)

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
1-2 s are common, and Wispr has no value to copy (D19). Recommendation:
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

**Phase 1: segment-ahead in flow-core (S)**: `segment.rs`, `engine.rs`
scheduling and finalize, the config fields, `AsrRequest.prior_text`,
`AsrEngine::segmentable`, the whisper prompt, per-segment guard, new
timings, flow-ffi header/JSON docs, and the core-architecture "Changelog
for shells".

**Phase 2: server / protocol (S)**: no message change. Optional, additive:
`HelloOk.features: ["segments"]` (serde default `[]`) so the Developer
row can show it. Golden fixtures for the new `SessionConfig` and
`Timings` fields. The server logs `segments` and `pre_asr_ms`. Contract
tests in §5.

**Phase 3: shells**
* **A** (`apps/flow-android`): `FlowCore.kt` SessionConfig
  `segmentPauseMs`, `CoreJson.kt` (de)serialisation, `DataLogic.kt`
  default 800 ms (Developer toggle "Decode while speaking"), endpoint
  default 1500. History shows `segments` / `wait`. Preview uses the
  partial as before (the core already joins it). Tests:
  `DataLogicTest`, `CoreJsonTest` fixtures.
* **W** (`apps/flow-windows`): `core_api.rs` and `core_adapter.rs` map
  the field, `prefs.rs` default 800 / endpoint 1500, overlay unchanged.
  Tests in `core_adapter.rs`.

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

Contract (S, `flow-contract-tests`):
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

## 7. Proposed DECISIONS row (not applied)

| D22 | Release→text latency for long dictations (2026-10-23) | Opt-in `SessionConfig.segment_pause_ms` (recommended 800; `segment_min_ms` 4000, `segment_soft_max_ms` 20000). The core's worker cuts the buffer at pauses with a streaming D16 VAD (cuts a pure function of the audio), decodes each closed segment while the user keeps talking (priority over partials, which shrink to the open segment), and at commit decodes only the tail, joins, then runs guard / spoken cleanup / LLM / post-processing on the joined text. whisper gets the previous segment's last words as prompt; engines that must not pre-upload (cloud, D18) opt out. Runs inside FlowCore, so flow-server, router fallback replay and the equivalence suite need no protocol change. Hands-free endpoint default 2000 → 1500 ms. Phase 0 also caps the partial window at 12 s and adds a Parakeet empty-on-speech fallback | Field (S25+): `total − asr` up to 0.9 s is the commit queued behind an unabortable live-preview decode (whisper encoder, any sherpa decode); ASR is linear-to-superlinear in length (3.5-3.8 s at 24-28 s). Probe `tools/probes/prototypes/segmented_asr_2026_10_23.py`: on 8 clips of 19-62 s, pause-segmented WER equals the whole clip (Parakeet 1.38%, whisper base 4.05% at P = 800 ms), and Parakeet beam search returns EMPTY on a 41.7 s clip that segmenting fixes. Forecast: release→text ≈ 0.4-0.9 s at any length, hands-free ≈ endpoint + 50 ms |
