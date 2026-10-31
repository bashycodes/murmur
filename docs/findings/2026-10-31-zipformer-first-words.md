# zipformer-he "drops the first words" (2026-10-31)

**Question.** The 2026-10-31 server e2e (`2026-10-31-server-model-setup.md`)
reported that zipformer2-66m-streaming-he dropped the first 3 words of a
13.3 s FLEURS he clip, with forced `he` and with auto-detect. Is that
streaming warm-up, VAD or leading-audio trimming, endpointing, or a stream
reset discarding the first segment?

**Answer.** None of them. No words are dropped: **the clip's speaker does
not read the sentence in reference order.** The FLEURS reference starts
"הפארק הלאומי קרוגר (KNP) נמצא בחלק הצפון מזרחי ...", but the audio starts
at "נמצא" and the speaker says "הפארק הלאומי קרוגר" near the end, just
before "קרוקודייל". zipformer-he writes those words where they are spoken.
The e2e compared the head of the output against the head of the
reference and read the missing head as dropped. No code change in the
decode path is needed.

## Clip

FLEURS he_il **dev** `10043827527204102024.wav` (13.32 s, float32, peak
1.0), the only 13.3 s clip of the five in the local sample set that the
e2e probe used (the 7.6 s and 6.5 s rows match `10068749401638533088` and
`10220829452246804771`).

* 0–1.1 s: breath / room at ≈ −33 to −27 dBFS RMS, no words.
* ≈ 1.1–1.6 s: "נמצא".
* ≈ 9.5–11 s: "הפארק הלאומי קרוגר", then "קרוקודייל" to the end.

## Evidence

All decodes with `crates/flow-asr-sherpa/examples/online_check` (new
`--skip S` / `--lead S` options) unless noted; 2 threads, greedy unless
noted.

| input | zipformer-he output (head … tail) |
|---|---|
| full clip, beam (production) | נמצא בחלק הצפון-מזרחי … וגבולו הדרומי הוא נהר כאמביה, פארק הלאומי קרוגר קרוקודייל. |
| full clip, greedy | same |
| full clip + 0.3 / 0.5 / 1.0 s leading silence | same head ("נמצא בחלק"), "(ה)פארק הלאומי קרוגר" still at the end |
| first 1.0 s only | (empty) |
| first 1.5 s / 1.7 s | "נאמר" / "נמצא" |
| first 2 / 3 / 4 s | נמצא בחדר / נמצא בחלק הצפון-מזרחי / … של דרום אפריקה |
| first 11 s | … נהר כאמביה, פארק הלאומי קרוגר |
| from 9.5 s | הפארק הלאומי קרוגר קרוקודייל |
| from 10 s | לאומי קרוגר קרוקודייל |

**Offline cross-check:** Parakeet-TDT-0.6B-v3-he int8 (offline NeMo
transducer, a different architecture and runtime path; `spot_check`
example) gives the same order on the full clip: "נמצא בחלק הצפון-מזרחי של
דרום אפריקה … נהר כאמביא הפארק הלאומי קרוגר קרוקודייל", and "נמצא בחדר" on
the first 2 s. Two unrelated models agree, so the order is in the audio.

## The suspected mechanisms, checked

* **Streaming warm-up / leading padding.** Prepending 0.3–1.0 s of
  silence changes nothing; cutting the clip so the first word starts at
  sample 0 (`--skip 1.0`) still decodes "נמצא" in full. The streaming
  encoder does not eat the first chunk.
* **VAD / leading trimming.** The lead-in breath is above `vad::ABS_CAP`
  (−36 dBFS), so `vad::detect` keeps the clip from sample 0; nothing before
  the first word is cut. Unit test `vad::tests::a_soft_lead_in_before_loud_speech_is_kept`.
* **Endpointing / stream re-creation.** `OnlineEngine` sets
  `enable_endpoint = false` and decodes one fresh stream per commit (whole
  buffer + 1.5 s tail, `input_finished`, decode while ready); there is no
  endpoint reset or second stream that could discard a first segment. The
  e2e sessions did not enable D22 segmentation (`segment_pause_ms` unset).

## Regression test

`crates/flow-core/tests/sherpa_online_e2e.rs`
`zipformer_keeps_every_spoken_word_of_the_kruger_clip`: the clip through
FlowCore (VAD + sherpa_online, forced `he`) must contain "נמצא", "הלאומי
קרוגר" and "קרוקודייל" in that order, and VAD must start before 1 s. Like
the other tests in that file it skips (passes with a note) when the model
or the clip is absent (`FLOW_HE_KRUGER_WAV` overrides the path).

## Follow-ups

* Score FLEURS-based checks with WER (order-aware alignment) or read the
  whole hypothesis; FLEURS readers sometimes deviate from the reference.
  The 26.8 % zipformer FLEURS WER also counts such reader deviations.
