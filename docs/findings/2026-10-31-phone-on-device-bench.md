# 2026-10-31: phone on-device benchmark (STT + AI cleanup)

**Question.** How accurate and how fast are the models the phone already has, running
inside the app on the device? Which should the phone default to?

**Answer.**

* **English STT: Parakeet-en.** It has **1.0 % WER** with commit p50 **973 ms** and p90
  1365 ms. whisper base has 3.7 % WER and p90 2.8 s.
* **Hebrew STT: the two models are equally accurate, so keep zipformer-he.**
  * On the clips where the app returned text, the zipformer has **26.6 %** WER and
    Parakeet-he **26.8 %**. This matches the laptop and server benches (26.8–28.3 %).
  * The headline 51 % WER is the app's VAD, not the models. 15 of the 49 FLEURS clips
    came back EMPTY on both models: 12 `no_speech` and 3 `blank_asr` (§ Hebrew EMPTY rows).
  * Parakeet-he loads faster (4.7 s against 16.6 s) and commits faster in this run. But it
    uses 0.5 GB more memory, and the zipformer streams, so this run overstates its latency
    (§ STT).
* **AI cleanup on the phone: Qwen2.5-1.5B is too slow for the 3 s budget.**
  * The `prod` mode scores **92 %** on the 88-suite, the same as production on the home
    server.
  * Most of that comes from the rules: 52 of the 102 cases never reach the LLM.
  * Of the ~50 cases that reach it, **11 time out at 3 s** and 2 more end in a deadline or
    drop. The answered calls take p50 **1189 ms** and p90 2.6 s.
  * Loading takes 38 s and the model then holds **2.1 GB** PSS. It runs on the CPU only.
  * The LLM alone (prompt v4c, no rules in front) scores 75 %, the same as on the laptop
    CPU, at p50 2.0 s.
* **Recommendation:**
  * Phone STT: Parakeet-en for English, zipformer-he for Hebrew.
  * Cleanup: cloud or server first, the on-device LLM only as the offline fallback, with
    the rules covering a timeout.

## Setup

| | |
|---|---|
| Device | SM-S936B (Snapdragon 8 Elite, SM8750, 8 cores), Android SDK 37 |
| App | 0.12.0 hybrid debug, `feat/android-bench` (`BenchActivity`) |
| Run | `r3`: suite all, 4 threads, `pace=fast` (all audio pushed, then commit) |
| Thermal | 0 → 0 for every model (no throttling) |
| Fixtures | 40 LibriSpeech test-clean clips (353 s); 49 FLEURS he_il dev clips (416 s); 102 cleanup cases (88-suite + 14 Wispr targets) |
| Harness | `tools/probes/android_bench/` (README); scored with `score.py` |

All models were read in place from the app's storage. Qwen2.5-0.5B was skipped because
its file is not on the phone.

## STT

Commit latency and RTF are over the non-empty rows. EMPTY rows skip ASR and report 0 ms.

| model | set | WER (all) | EMPTY | WER / CER (non-empty) | commit ms p50 / p90 / max | RTF | load ms | PSS after load |
|---|---|---|---|---|---|---|---|---|
| **parakeet-en** | en | **1.0 %** | 0 | 1.0 % / 0.3 % | **973 / 1365 / 2005** | 0.112 | 3696 | 769 MB |
| whisper-base | en | 3.7 % | 0 | 3.7 % / 1.4 % | 1107 / 2832 / 4368 | 0.160 | 278 | 219 MB |
| **zipformer-he** | he | 51.4 % | 15 | **26.6 % / 15.1 %** | 1198 / 2019 / 2340 | 0.128 | 16617 | 311 MB |
| parakeet-he | he | 51.5 % | 15 | 26.8 % / 14.5 % | 792 / 1330 / 2065 | 0.091 | 4658 | 797 MB |

* The zipformer is a streaming model. With `pace=fast` it decodes the whole clip at
  commit. In real use it decodes while the user speaks, and on the laptop it finished 48 ms
  after the last audio (2026-10-28). So 1198 ms is its worst case, not what a user feels.
  A `pace=realtime --es segment 800` run would measure the real release → text time.
* Parakeet-he is offline, so 792 ms is close to what a user would feel.
* The phone's English WER matches the laptop's (Parakeet 1.2 %, 2026-10-30).
* The zipformer's 16.6 s load was measured once and not investigated. It matters only
  when Hebrew is first used or switched to.

### Hebrew EMPTY rows

The same 15 clips are EMPTY on both models.

| empty reason | clips | clip RMS (PCM16) | clip p99 \|sample\| |
|---|---|---|---|
| `no_speech` (VAD found no speech) | 12 | 20–49 | 61–164 |
| `blank_asr` (VAD passed, ASR returned nothing) | 3 | 75–92 | 251–309 |
| transcribed | 34 | 78–14 599 (median ≈ 5000) | 296–32 767 |

* These FLEURS recordings are very quiet, at about −60 dBFS. The PCM16 files are
  identical to the laptop bench's (converted with `soundfile`), so the conversion is not
  the cause.
* `flow_core::vad` treats a frame as speech only above `ABS_MIN = 0.004` (about 131 in
  PCM16). These clips never reach it, so the app reports no speech. The laptop and server
  benches fed audio to the recognizer directly, with no VAD, which is why they have no
  EMPTY rows.
* A phone microphone with AGC is unlikely to record dictation this quietly, so this does
  not show a product bug. But a whispered dictation would be dropped the same way. If that
  turns out to matter, either lower `ABS_MIN` or let a quiet clip through when its peak is
  well above its own noise floor.

## AI cleanup (Qwen2.5-1.5B Q4_K_M, CPU)

`prod` is the app's own path: `flow_core::refinish` with the fallback rules, prompt v2,
the guard, post-processing and the 3 s Android budget.

| | qwen1.5b `prod` |
|---|---|
| **88-suite pass** | **92 %** (81/88); `v4c` 75 % |
| Wispr exact / closeness / char ratio (14) | 2/14 / 0.796 / 0.859 |
| guard rejects, strict and loose (good answers rejected) | 0 (0) |
| reached the LLM | ~50 of 102 (52 `llm_skipped_clean`) |
| answered | 37 |
| **timed out at 3 s** | **11** (`llm_timeout`) + 1 `llm deadline` + 1 `llm_dropped` |
| LLM ms on answered calls, p50 / p90 | **1189 / 2629** |
| answered calls over 1.5 s | 16 of 37 |
| load / warm-up prefill | 38.1 s / 34.4 s (1355 prompt tokens) |
| PSS after load | 2078 MB |

* Failures by 88-suite category: correction 5/7, list 4/5, run-on 3/6, negative 15/16.
  Every other category is full.
* The timeouts do not depend on input length: the inputs have 7–60 words. Prefill of the
  ~1.3 k-token prompt v2 on the CPU dominates each call. On a timeout the user gets the
  rules' text, which is why the score holds.
* A timed-out call is abandoned, not cancelled. It may still occupy the LLM while the next
  case starts, so these latencies may be pessimistic for back-to-back dictations.
* **`v4c`** sends prompt v4c on the raw transcript, has no rules in front, and is judged
  with the product's strict guard, as in the 2026-10-30 laptop bench.
  * It scores **75 %** on the 88-suite with target-closeness 0.812. The laptop CPU scored
    74–76 % with 0.813 for the same model, so the phone's quantised build behaves the same.
  * Its LLM time is p50 **1977 ms** and p90 3262 ms. 15 of the 102 calls exceed 3 s, and
    the guard rejects 6 answers.
  * It is slower than `prod` because every case calls the LLM, with a longer prompt.
  * Its weak categories are newline (1/5), filler (2/6) and correction (3/7).
* For comparison, Qwen2.5-1.5B runs at p50 1.0 s on the laptop CPU, 0.39 s on the laptop
  iGPU (Vulkan) and 1.4 s on the home server (2026-10-30).

## Recommendation

1. **Phone STT defaults:** Parakeet-en for English. For Hebrew, keep zipformer-he:
   * it is as accurate as Parakeet-he (26.6 % against 26.8 %);
   * it uses 0.5 GB less memory;
   * it streams, so a user would see less latency than this run shows.

   Parakeet-he is the alternative if the 16.6 s load turns out to matter.
2. **Cleanup order on the phone:** cloud or server first, the on-device LLM only when
   offline. Locally, about 1 in 4 LLM calls already misses the 3 s budget, and the model
   costs 2 GB of RAM. When the local call misses, the rules keep the result at the
   production 88-suite score.
3. **Follow-ups (not blocking):**
   * a realtime-paced run for the zipformer's real latency;
   * a GPU (OpenCL/Vulkan) llama.cpp build for the phone, which on the laptop made the
     1.5B 2.6× faster;
   * possibly a quiet-speech path in the VAD.

## Reproduce

```sh
python3 -I tools/probes/android_bench/prepare_bundle.py
S=tools/probes/android_bench/run_on_device.sh   # $ADB set; local debug APK installed
$S push; $S start r3; $S wait r3; $S pull r3
python3 -I tools/probes/android_bench/score.py $CACHE/android-bench/results/r3
```

The results stay in the local cache. The jsonl files contain only public-fixture text.
