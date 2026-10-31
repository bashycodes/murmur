# On-device benchmark (Android)

Measures STT accuracy and latency, and AI cleanup quality and latency, **on the phone**, with
the models already in the app's private storage. Nothing is re-downloaded or re-pushed except
the fixture bundle (about 25 MB, public clips only).

* The harness is `BenchActivity`, debug builds only (`apps/flow-android/app/src/debug/`).
  * It runs inside the app's process, so it can read `files/`.
  * It is exported but guarded by `android.permission.DUMP`, so only the shell can start it.
  * Release builds have no such activity; `verify<Variant>NoDebugWavReceiver` and
    `NoDebugWavReceiverTest` check that.
* It uses the app's real code paths:
  * STT is a normal `FlowCore` dictation: start_session, set_context, push_audio in 100 ms
    frames, commit. Model and session configs are built like `EngineLogic.plan` /
    `toSessionConfig` (Parakeet greedy, Hebrew loaded alone, 4 threads).
  * Cleanup runs through `nativeBenchClean` (`crates/flow-ffi/src/bench.rs`) on the loaded
    on-device LLM, in two modes:
    * `prod` calls `flow_core::refinish`, the product's own text path: fallback rules, prompt
      v2, guard, post-processing, and the app's 3 s budget.
    * `v4c` sends the raw transcript with prompt v4c and returns the answer unjudged, as in the
      2026-10-30 laptop bench.
* The phone writes raw outputs only. WER, CER and the 88-suite score are computed on Linux by
  `score.py`, with the existing scorers (`bench/wer.py`, `ai_cleanup_bench_2026_10_26/score.py`,
  `ai-cleanup-bench --mode regard`).

## Models (read in place under `files/`)

| key | path | fixture set |
|---|---|---|
| `parakeet-en` | `parakeet/parakeet-unified-en-0.6b-int8` | en: 40 LibriSpeech test-clean clips, 353 s |
| `whisper-base` | `models/ggml-base-q5_1.bin` | en |
| `zipformer-he` | `hebrew/zipformer-he` | he: 49 FLEURS he_il dev clips, 416 s |
| `parakeet-he` | `hebrew/parakeet-v3-he` | he |
| `qwen1.5b` | `llm/qwen2.5-1.5b-instruct-q4_k_m.gguf` | 102 cleanup cases (the 88-suite plus 14 Wispr targets) |
| `qwen0.5b` | `llm/qwen2.5-0.5b-instruct-q4_k_m.gguf` | skipped when the file is absent |

A missing model, or one that fails to load, is noted in `meta.json` and skipped.

## 1. Build the bundle (Linux)

```sh
python3 -I tools/probes/android_bench/prepare_bundle.py   # -> $CACHE/android-bench/bundle (~25 MB)
```

* The English clips come from `$CACHE/audio`. If any are missing, run
  `tools/probes/bench/fetch_audio.py --manifest asr_manifest_extended.json`.
* The Hebrew clips come from the FLEURS tarball, which is downloaded once (153 MB) into
  `$CACHE/dl/fleurs-he/`.
* `--en N` / `--he N` give a smaller bundle.

## 2. On the phone (orchestrator; `$ADB` = the device bridge command)

The phone must run a **locally built hybrid debug APK**. If it runs a CI build instead,
installing a debug APK means a different signing key, which wipes `files/`, models included.

```sh
export ADB="<device bridge> -s <serial>"
S=tools/probes/android_bench/run_on_device.sh
$S push                       # bundle -> /data/local/tmp/flow-bench -> (run-as) files/bench/in
$S start r1                   # everything; or e.g. --es suite stt / --es llm qwen1.5b --es modes v4c
$S wait r1                    # polls out/r1/status.json every 30 s
$S pull r1                    # -> $CACHE/android-bench/results/r1
```

* The raw commands behind the script:
  * start: `$ADB shell am start -W -n dev.flowclone/.debug.BenchActivity --es run r1 [--es suite all|stt|cleanup] [--es stt k1,k2] [--es llm k] [--es modes prod,v4c] [--es threads 4] [--es limit N] [--es pace fast|realtime] [--es segment 800]`
  * status: `$ADB shell cat /sdcard/Android/data/dev.flowclone/files/bench/out/r1/status.json`
  * pull: `$ADB pull /sdcard/Android/data/dev.flowclone/files/bench/out/r1/. <dest>/`
  * stop after the current item: `$ADB shell am start -n dev.flowclone/.debug.BenchActivity --ez stop true`
* Keep the activity in front. It holds the screen on, and the work runs on its thread.
* Expected run time with every model is about 15–25 min: 4 × ~6.5 min of audio at RTF < 0.1,
  plus 2 × 102 LLM calls per LLM.
* Pacing:
  * `pace=fast` (the default) pushes all audio, then commits, so `commit_ms` holds the whole
    decode. This is the worst case, and the number to compare with the PC benches.
  * `pace=realtime` with `segment=800` feeds the audio at speaking speed with "decode while
    speaking" on, so `commit_ms` is the release → text latency a user feels.
* Why `push` goes through run-as: files the shell pushes into
  `/sdcard/Android/data/dev.flowclone/` are owned by the shell uid (FUSE) and the app cannot
  read them ("no bundle: manifest.json missing"). So the bundle is staged in
  `/data/local/tmp/flow-bench`, copied into the app's private `files/bench/in`, and the
  staging copy is deleted (`KEEP_TMP=1` keeps it so the next push only sends changes).
  Results still go to `<ext>/bench/out/<run>`, which the app creates itself, so `pull` works
  without run-as. `push` also removes a shell-created `<ext>/bench/in` left by older runs.
  By hand:
  `$ADB push <bundle>/. /data/local/tmp/flow-bench/ && $ADB shell chmod -R a+rX /data/local/tmp/flow-bench && $ADB shell run-as dev.flowclone sh -c "'rm -rf files/bench/in && mkdir -p files/bench && cp -r /data/local/tmp/flow-bench files/bench/in'"`.
* If that copy fails, a fallback that worked on the phone: push to `<ext>/bench/in`, run
  `run-as dev.flowclone cp -r <ext>/bench/in files/bench/`, then `rm -rf <ext>/bench` so the
  app recreates `bench/out` itself.

## 3. Score (Linux)

```sh
python3 -I tools/probes/android_bench/score.py $CACHE/android-bench/results/r1   # also writes r1/score.md
```

* STT output: WER and CER per model, `commit_ms` p50/p90/max, and RTF.
* Cleanup output: the 88-suite table per LLM and mode, LLM ms p50/p90, and the calls over
  1.5 s and over 3 s.
* `v4c` rows need the `ai-cleanup-bench` binary. Build it once:
  `cd tools/probes/prototypes/ai_cleanup_bench_2026_10_26/runner && CARGO_TARGET_DIR=../../../../../target/ai-bench cargo build --release -j 2 --offline`.
  Or point `--bench-bin` / `BENCH_BIN` at an existing one.

## Output files (`bench/out/<run>/`)

| file | content |
|---|---|
| `status.json` | `{state, phase, done, total}`, updated while the run goes |
| `meta.json` | device, app build, args, and per model: load ms, warm-up ms, PSS, thermal status, or why it was skipped |
| `stt-<model>.jsonl` | per clip: `asr_text`, `final_text`, `status`, `audio_ms`, `push_ms`, `commit_ms`, core `asr_ms` / `total_ms` |
| `cleanup-<llm>-<mode>.jsonl` | per case: `llm_raw`, `llm_ms`, `error`, `llm_info` (llama.cpp stats); `prod` also has `output` and `fallback_reason` |
