# STT model survey and CPU benchmark (2026-10-23)

Status: applied as D23 in docs/DECISIONS.md (server default switched to the unified model 2026-10-23; the greedy/retry change landed with D22 on `feat/streaming-asr`).

The question: can the server (i7-6700, 4C/8T, AVX2, no AVX-512, 19 GB RAM, no GPU; today Parakeet-TDT-0.6B-v2 int8 on sherpa-onnx) move to a bigger, more accurate or faster model? And what should the phone (Galaxy S25+, Snapdragon 8 Elite; today whisper base q5_1) run?

## TL;DR

* **No open model is both more accurate and as fast on this CPU.**
  * Cohere Transcribe (2B) is the only model that beat v2 on our set: 0.84% vs 1.15% WER.
  * It is 2.5× slower: 29 s of audio takes 14–17 s.
  * Its decoder also dropped a whole utterance on a 56 s clip.
* **The real problems are latency on long dictations and long-audio failures. Model size is not.** Three findings matter more than any model swap:
  1. **The server's own decoding mode fails silently on long audio.** `flow-asr-sherpa` always uses `modified_beam_search` with beam 4 (for hotwords).
     * With v2 int8 it returns **an empty string** on a 25.1 s clip and on a 42.8 s clip. Both are concatenations of LibriSpeech utterances. The result was deterministic: 6 of 6 runs empty, and beam 8 failed too.
     * Greedy decoding and beam 2 are fine on the same audio.
     * Every single clip (5–20 s) decodes fine with beam 4.
  2. **One-shot long decodes drop whole utterances.** On the 43–56 s clips (40-clip-set utterances joined with 0.3 s gaps):
     * v2 greedy lost a 22-word utterance at 42.8 s (17.5% WER);
     * v3 lost 27 words at 56 s (23%);
     * Cohere lost 29 words at 56 s (22%).
     * This matches sherpa-onnx PR #4084 (open), which reports it for the int8 Parakeet encoder.
     * **Parakeet-unified-en-0.6b and Nemotron-streaming lost nothing** (0–2.1% and 2.4–5.5%).
  3. **Decode time is linear in audio length, RTF ≈ 0.10 on this box** (interleaved A/B, 4 threads):
     * v2 takes 1.04 s for 11 s of audio and 3.1 s for 29 s.
     * So the phone's "27.2 s → 2.5–3.2 s" is the normal cost.
     * The anomaly is "11.6 s → 155 ms", 7× faster than this CPU can decode 11 s of speech with v2. That number probably measured less audio than 11.6 s of speech (VAD trim, or a mostly silent recording); the other agent's investigation should confirm.
* **Server recommendation (D23):**
  * Switch to **`nvidia/parakeet-unified-en-0.6b` int8 (sherpa-onnx, non-streaming)**. It costs a model-directory change only: same engine, same `nemo_transducer` type, hotwords work.
  * Measured: same WER as v2 (1.15%), same speed (1.17 s / 2.87 s for 11 s / 29 s audio), and no utterance drops at 43 s or 56 s with greedy or beam 4.
  * On the HF leaderboard it is slightly better than v2 (≈5.9 vs 6.05, old format).
  * Also fix the beam-search empty-result bug: decode greedy when the dictionary is empty, and re-decode greedy when a beam result is empty or implausibly short.
* **Next latency step (needs engine code): Nemotron-speech-streaming-en-0.6b (560 ms chunks).**
  * The tail latency (release → text) is **150–330 ms whatever the length**: measured at 11, 29, 43 and 56 s.
  * Offline v2 needs 3 s for a 29 s clip.
  * Cost: WER 2.2% vs 1.15%. It keeps up with real time using about 40% of 4 threads while the user speaks.
  * This is the model `docs/proposals/streaming-asr-latency.md` needs.
* **Phone:**
  * Ship **Parakeet-unified-en-0.6b int8 (or v2)** for English. The arm64 sherpa/onnxruntime build already exists (`tools/build-android-parakeet.sh`).
  * Then the same Nemotron-streaming model on-device.
  * Keep whisper base/small q5_1 as the multilingual or auto-detect option.
  * Moonshine-base v2 is fast (RTF 0.05–0.07 on one core here), but it is **broken in sherpa-onnx 1.13.8 for clips longer than 9.4 s**: every one comes back empty.
* **"CPU barely used":**
  * flow-server runs `threads = 4` (`~/.config/flow-server/config.toml`, `[model] threads = 4`) on 8 logical CPUs, with `workers = 1`. A decode can therefore never show more than 50% in a whole-machine CPU meter.
  * The server also only works for 1–3 s after each release and idles in between.
  * During a decode it does use its threads: 2.9–4.0 cores out of 4 (process user+sys time over wall time).
  * Beam search adds a serial decoder/joiner loop (RTF 0.27 vs 0.20 greedy in the same run).
  * More threads won't buy much, because this box shares its 4 cores with other jobs:
    * at load average 15, 1 thread gave RTF 0.207 and 4 threads 0.204;
    * at load 5, 8 threads gave 0.144.
  * The prior bench (docs/model-benchmarks.md) found 4 threads ≥ 8. Keep 4. The latency lever is streaming, not threads.

## Test setup

* Box: i7-6700. **Heavily shared during the whole run:** the 1-minute load average was 5–49 (gradle, cc1plus and chrome from other agents).
  * Absolute RTFs in the "full run" columns are therefore pessimistic, by 1.5–2× versus the 2026-10-20 bench, where v2 measured RTF 0.137.
  * The **interleaved A/B** (`ab_latency.sh`: round-robin, 3 rounds × 3 reps, load 13–17) is the fair speed comparison.
  * Quality numbers are unaffected by load.
* Engine: sherpa-onnx v1.13.8 (2026-09-10, the latest release), onnxruntime 1.28.2. The shared libs in `.cache/sherpa-onnx/lib` are the same version the `sherpa-onnx = 1.13.8` crate uses.
* Probe: `tools/probes/prototypes/stt_survey_2026_10_23/`.
  * The Rust binary keeps one warm recognizer (like flow-server), warms up on jfk.wav, then decodes:
    * (a) the 40-clip extended LibriSpeech set (`tools/probes/bench/asr_manifest_extended.json`, 955 words);
    * (b) "long" clips: whole utterances joined with 0.3 s gaps, shortest first, so references stay exact.
  * CPU use = process `getrusage` user+sys over wall time.
  * WER uses the same normaliser as the earlier benches (`tools/probes/bench/wer.py`), so numbers compare with docs/model-benchmarks.md.
* Greedy decoding unless noted. 4 threads unless noted. Results go to `$CACHE/results/stt-survey-2026-10-23/`, one file per run (`$CACHE` = the local bench cache, outside the repo).
* Models were downloaded from github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models into `$CACHE/models/survey-2026-10-23/dl-<name>/`.

## Measured on this box

### Accuracy and speed, 40 clips (5–20 s), full sequential run

| model (sherpa-onnx int8 package) | disk | WER | RTF t4 (load) | t1 | t8 | engine work in flow-clone |
|---|---|---|---|---|---|---|
| **Cohere Transcribe 03-2026** (2B, 14 lang) | 2.7 GB | **0.84%** | 0.518 (≈17) | — | — | new adapter (`cohere_transcribe` config) |
| Parakeet-TDT-0.6B-v2 (current), greedy | 631 MB | 1.15% | 0.204 (15) | 0.207 (16) | 0.144 (5) | — |
| Parakeet-TDT-0.6B-v2, beam 4 (**what flow-server runs**) | 631 MB | 1.15% | 0.270 (20) | — | — | — |
| **Parakeet-unified-en-0.6b** (non-streaming) | 633 MB | 1.15% | 0.446 (19–27)\* | 0.302 (27) | 0.241 (17) | **none: model dir only** |
| Parakeet-TDT-0.6B-v3 (25 EU lang) | 641 MB | 1.57% | 0.209 (27) | — | — | none: model dir only |
| Qwen3-ASR-0.6B (30 lang) | 954 MB | 1.57% | 0.472 (22) | — | — | new adapter (`qwen3_asr`) |
| Canary-180M-flash (en/de/es/fr) | 198 MB | 1.68% | 0.243 (15) | — | — | new adapter (`canary`) |
| Nemotron-speech-streaming-en-0.6b, 560 ms | 633 MB | 2.20% | feed 0.37–0.40; tail 150–330 ms | — | — | new **online** engine (streaming) |
| Moonshine-base-en v2 (2026-02-27 quantized) | 135 MB | 45% (22/40 empty) | 0.053 | 0.051 | — | new adapter; **broken > 9.4 s** |
| (ref, 2026-10-20) whisper base q5_1 / small q5_1 / large-v3-turbo q5_0 | 57 / 181 / 547 MB | 3.7 / 2.9 / (1.0 on core set) | 0.245 / 0.855 / 3.26 (−ac ≈ 1) | | | in core |

\* The full-run RTF for unified fell in the most loaded window: 1.70 cores used vs 3 for v2. In the A/B it is as fast as v2 (next table).

Moonshine, on the 28 clips of ≤ 9.2 s where it returns text: WER 2.61% vs v2 1.12%, RTF 0.072 vs 0.212 at 4 threads. Above 9.4 s every clip fails inside onnxruntime with `encoder_attn/Add … broadcast 2 by 457` (sherpa `offline-recognizer-moonshine-v2-impl.h`) and comes back empty. That is a sherpa 1.13.8 bug, or the model's window, not a property of the model.

### Latency vs length (interleaved A/B, 4 threads, median of 9, load 13–17)

| model | ~11 s audio (11.2 s) | ~29 s audio (28.9 s) | scaling |
|---|---|---|---|
| Parakeet v2 | 1038 ms | 3119 ms | linear, RTF ≈ 0.09–0.11 |
| Parakeet-unified | 1172 ms | 2869 ms | linear |
| Parakeet v3 | 1115 ms | 2900 ms | linear |
| Canary-180M-flash | 2237 ms | 10359 ms | **superlinear** (attention-decoder) |
| Nemotron streaming 560 ms: tail after release | **331 ms** | **186 ms** | constant |
| Nemotron streaming: compute while speaking | RTF 0.40 | RTF 0.37 | keeps up with real time |

From the full run (more loaded): Qwen3-ASR-0.6B took 18.5 s and Cohere 13.9–17.3 s for the 28.9 s clip. Both are too slow for interactive dictation on this CPU.

### Long audio (one decode of the whole recording)

| model / decoding | 25.1 s | 42.8 s | 56.3 s |
|---|---|---|---|
| v2 greedy | ok (72/72 words) | **17.5% WER**: dropped 22 words (one utterance) | 0.7% |
| **v2 beam 4 (flow-server default)** | **empty** (6/6 runs; beam 8 also empty; beam 2 ok) | **empty** | 2.1% but all lowercase, no punctuation |
| v3 greedy | — | 1.6% | **23.4%**: dropped 27 words |
| Parakeet-unified, greedy | — | **0.0%** | **2.1%** |
| Parakeet-unified, beam 4 | 0.0% | **0.0%** | **2.1%** |
| Nemotron streaming | — | 2.4% | 5.5% (no drops) |
| Cohere | — | 0.8% | **22.1%**: dropped 29 words |

Reproduce with `long_deletion.sh` (manifests `long43_manifest.json` and `long60_manifest.json` in the probe dir). The 25.1 s beam case is in `beam-long-recheck.txt` in the results dir.

**Mitigation available today with no code change:** `crates/flow-core/src/parakeet.rs` switches to greedy when the environment variable `FLOW_PARAKEET_HOTWORDS_SCORE=0` is set. That would go in the systemd unit as `Environment=`. It turns off dictionary hotwords, which cut WER from 1.2% to 0.9%.

What this means for flow-server today:
* A dictation of about 25 s or more can come back as `blank_asr` with v2 and beam 4. It depends on content: one 28.9 s clip decoded fine.
* The live test's `blank_asr` on a 7.3 s utterance could be the same failure mode on spontaneous speech. Unverified: no audio is kept.
* The mitigations are independent of the model swap:
  * decode greedy when there are no hotwords;
  * treat an empty or implausibly short beam result (fewer than about 0.5 words/s on speech the VAD saw) as a failure and re-decode greedy;
  * longer term, cut audio at VAD pauses into segments of 20 s or less. NeMo's local attention (`rel_pos_local_attn`) is not available in these ONNX exports.

## Survey: 2026 landscape (published numbers)

Sources: the HF Open ASR Leaderboard (live space config, fetched 2026-10-23, plus the repo's `en_shortform.csv` "as of 26 March 2026"), HF model cards, and the sherpa-onnx release assets (GitHub API). In July 2026 the leaderboard switched to a new dataset mix (cleaned AMI / Earnings22 / VoxPopuli, URGENT24, Voice Arena, private sets). **"New" and "old" averages are not comparable.** RTFx is GPU throughput from the leaderboard, not a CPU number.

| model | params / license | langs | new avg | old avg (LS-c / LS-o / E22 / AMI) | CPU runtimes | streaming | P&C | CPU fit here |
|---|---|---|---|---|---|---|---|---|
| Qwen3-ASR-1.7B | 1.7B / Apache | 30 incl. ar, no he | **5.40** (top open) | 5.76 | sherpa (0.6B only), llama.cpp mtmd | vLLM only | yes | LLM decoder, RTF ≈ 1 (est.) |
| IBM granite-speech-5.0-2b | 2.3B / Apache | en fr de es pt ja | 5.68 | — | transformers; 4.1-2b has GGUF | no | yes | too slow (est.) |
| Cohere Transcribe 03-2026 | 2B / Apache (gated) | 14 incl. ar, no he | 5.77 | 5.42 (1.25 / 2.37 / 10.86 / 8.13) | **sherpa int8 2.7 GB** | no | yes | **measured RTF 0.52**, long-drop |
| canary-qwen-2.5b | 2.5B / CC-BY-4.0 | en | 5.94 | 5.63 | NeMo only | no | yes | too slow |
| **Parakeet-TDT-0.6B-v2** (current) | 0.6B / CC-BY-4.0 | en | 6.02 | 6.05 (1.69 / 3.19 / 11.15 / 11.16) | sherpa, NeMo-Speech.cpp | no | yes | **measured RTF ~0.1** |
| **Parakeet-unified-en-0.6b** (2026-04) | 0.6B / NVIDIA Open Model License | en | — | ≈5.91 (card; LS-c 1.63, LS-o 3.11, AMI 10.14) | **sherpa int8 (offline + 240/560/1120 ms streaming)** | **yes (buffered)** | yes | **measured = v2 speed** |
| granite-speech-5.0-470m-turboctc | 0.47B / Apache | 7 | 6.02 | — | transformers only | no | yes | promising, no ONNX yet |
| Parakeet-TDT-0.6B-v3 | 0.6B / CC-BY-4.0 | 25 EU (no he/ar) | 6.18 | 6.32 | sherpa | no | yes | measured RTF ~0.1, long-drop |
| moondream/parakeet-ultra (2026-09) | v3 post-train / CC-BY-4.0 | 25 EU | — | card: 7 English sets 6.26 → 5.80 | third-party sherpa int8 (drop-in for v3) | no | yes | untested; unofficial export |
| Qwen3-ASR-0.6B | 0.6B / Apache | 30 | 6.24 | 6.42 | **sherpa int8** | no | yes | **measured RTF 0.47** |
| Voxtral-Small-24B / Mini-3B / Mini-4B-Realtime | Apache | 13 incl. ar | 6.38 / 7.25 / 8.44 | 6.62 / 7.05 / 7.68 | llama.cpp mtmd (realtime merged 2026-10) | 4B-Realtime | yes | too slow |
| Nemotron-speech-streaming-en-0.6b (2026-01/03) | 0.6B / NVIDIA OML | en | 6.60 | ≈6.9 at 1.12 s chunk | **sherpa int8 80/160/560/1120 ms** | **yes (cache-aware)** | yes | **measured tail 0.15–0.33 s** |
| distil-large-v3.5 | 0.76B / MIT | en | 6.76 | 7.21 | whisper.cpp, CT2, sherpa | no | yes | large-v3 encoder: ≈ turbo cost, RTF ≈ 1 |
| whisper large-v3 / turbo | 1.55B / 0.8B MIT | 99 | 7.16 / 7.64 | 7.44 / 7.83 | whisper.cpp, CT2 | no | yes | turbo RTF 3.3 (≈ 1 with −ac) measured 10-20 |
| canary-1b-flash / 1b-v2 / 180m-flash | CC-BY-4.0 | 4 / 25 / 4 | — / 7.27 / — | 6.35 / 7.15 / 7.12 | sherpa: 180m only | no | yes | 180m measured: superlinear |
| Kyutai stt-2.6b-en / 1b-en_fr | CC-BY-4.0 | en(/fr) | — | 6.40 (2.6b) | Rust/MLX/PyTorch, no ONNX | yes | yes | no CPU runtime of ours |
| Moonshine streaming tiny/small/medium | MIT | en (+ar etc. per lang) | — | 12.0 / 7.84 / 6.66 | Moonshine Voice SDK (not sherpa) | yes | yes | phone candidate (vendor numbers) |
| Moonshine base v2 / tiny | MIT | en | — | 9.99 / 12.65 | sherpa (broken > 9.4 s in 1.13.8) | no | yes | measured 2.6% ≤ 9 s |
| Phi-4-multimodal | 5.6B / MIT | 8 | — | 6.02 | transformers | no | yes | too slow |
| nemotron-3.5-asr-streaming-0.6b (2026-06) | OpenMDW | 40 locales incl. ar (he only "adaptation-ready") | 9.75 | — | sherpa int8 | yes | yes | weak English |
| SenseVoice-small, FireRedASR2, Dolphin | Apache-ish | zh-centric / 40 Asian langs | — | — | sherpa | no | partly | not for an English-first user |
| Zipformer (sherpa) en streaming, kroko 2025-08 | Apache | en | — | Zipformer-XL 6.79 | sherpa | yes | no (some punct-case variants) | fast, less accurate |
| Meta omnilingual-asr 300M / 1B CTC | Apache | 1600 | — | — | sherpa int8 | no | no | breadth, not accuracy |

Published long-form table (new format): parakeet-v3 13.37, v2 13.88, whisper-turbo 13.60, cohere 12.23, canary-qwen 14.08.

## Ranking for the server (this CPU, English-first)

| rank | model | why | integration cost | risk |
|---|---|---|---|---|
| 1 | **Parakeet-unified-en-0.6b int8, non-streaming** | same WER and speed as v2 on our set; slightly better published WER; **no long-audio drops**, greedy or beam 4 | **config only**: `[model] asr_path` → the new dir (same `nemo_transducer`; `flow-asr-sherpa` finds `encoder/decoder/joiner*.int8.onnx` + `tokens.txt`); hotwords vocab derived as today | NVIDIA Open Model License, not CC-BY: check redistribution if bundled; RNNT decodes every frame, so beam 4 is slower than TDT beam (RTF 0.13–0.58 noisy) |
| 2 | **Nemotron-speech-streaming-en-0.6b int8 (560 ms)**; alt: unified streaming-560ms | constant ~0.2–0.3 s release→text for any length; robust on long audio | **new engine**: sherpa `OnlineRecognizer`, fed from `push_audio`; flow-server streams frames into it (protocol already streams audio) | WER 2.2% vs 1.15%; costs ~40% of 4 threads continuously while the user speaks; one stream per worker |
| 3 | Parakeet v2 int8 (keep), **greedy** + VAD segmenting | known-good; RTF ~0.1 | small `flow-asr-sherpa` change (greedy without hotwords, empty → greedy retry) | beam-4 empty results; drops on > 40 s |
| 4 | Cohere Transcribe int8 | best WER (0.84%), Arabic, 13 more languages | new adapter (`cohere_transcribe` config, ~50 lines) | 2.5× slower (29 s → 14–17 s), 2.7 GB, 59 s load, dropped 29 words at 56 s, gated licence acceptance |
| — | v3, Qwen3-ASR-0.6B, Canary-180M, whisper turbo / distil-v3.5 | no gain over v2 in English on this CPU (v3 −0.4 pt WER, long drops; Qwen3 4.5× slower; Canary superlinear; whisper encoder-bound) | | |

Multilingual tradeoff (the user's need is unknown):
* **European languages:** Parakeet v3 has the same speed as v2 and costs 0.4 points of English WER (1.57% vs 1.15%) on our set. It does not cover Hebrew or Arabic.
* **Arabic:** Cohere (slow) or Qwen3-ASR-0.6B (RTF 0.47).
* **Hebrew:** none of the sherpa packages above. Options:
  * ivrit.ai's `zipformer2-66m-streaming-he` (2026-10, CC-BY-4.0, sherpa-onnx, ~70 MB int8, eval WER 5.1 / 8.9; not tested here);
  * ivrit-ai whisper-large-v3-turbo (ggml 1.6 GB; RTF ≈ 1 on this CPU).
* **Suggestion:** keep English on Parakeet, and add a per-language model route only if the user asks for one. The clone already has the engine trait and dictionary per engine.

## Phone (Snapdragon 8 Elite, budget 50–700 MB)

There is no device measurement in this survey. Estimates scale this box's numbers. Assumption: the 8 Elite's Oryon cores are about 1.5–2.5× a Skylake core on int8 GEMM, with dotprod/i8mm.

| rank | model | size | expected (est.) 28 s dictation | engine work | notes |
|---|---|---|---|---|---|
| 1 | **Parakeet-unified-en-0.6b int8** (or v2) via sherpa-onnx CPU | 633 MB | ≈ 1.2–2 s (vs ~3.8 s whisper base today) | Android sherpa/onnxruntime libs are already cross-built (`tools/build-android-parakeet.sh`, `check_parakeet_so.sh`); package into `apps/`, never loaded on a device yet | 1.15% vs whisper base 3.7% WER; English only; ~1.1 GB RSS (flow-server measures 1.13 GB) |
| 2 | **Nemotron-streaming-en-0.6b int8** on device | 633 MB | **≈ 0.2–0.4 s after release** | same online engine as server rank 2 | feed RTF 0.37 at 4 threads here, ≈ 0.2 on phone (est.); thermal/battery cost of decoding while talking |
| 3 | Moonshine streaming small / medium (Moonshine Voice SDK) | 123–245M params | vendor: 234 / 420 ms end-of-speech → text on a Pixel 10a | new SDK, not sherpa | old-format avg 7.84 / 6.66; MIT; has Arabic models |
| 4 | QNN NPU builds (sherpa `asr-models-qnn-binary-*` for SM8750: Parakeet v2/v3 fixed 3–30 s buckets ~440 MB, Nemotron streaming, Moonshine) | ~440 MB | unknown (no published numbers) | QNN provider + bucket padding | fastest in theory; fixed shapes, vendor libs, riskiest |
| keep | whisper base q5_1 (multilingual), small q5_1 | 57 / 181 MB | 3–4 s / slower | in core | the multilingual / auto-detect fallback |

Moonshine-base v2 through sherpa (135 MB, RTF ~0.05 single-thread here) would be the size winner, but it is unusable until the > 9.4 s failure is fixed upstream.

## DECISIONS row (applied as D23 in docs/DECISIONS.md)

| D23 | ASR model refresh (2026-10-23, `docs/proposals/stt-model-survey.md`) | Server + Android English default: **Parakeet-unified-en-0.6b int8** (sherpa-onnx non-streaming package; drop-in for the v2 dir, same `nemo_transducer` engine and hotwords). `flow-asr-sherpa`: greedy search when the session has no hotwords; an empty or < 0.5 words/s beam result on VAD-confirmed speech is re-decoded greedy. Parakeet v2 stays the fallback model. Next: a streaming engine (sherpa `OnlineRecognizer`) with Nemotron-speech-streaming-en-0.6b 560 ms for the long-dictation latency (streaming proposal). Multilingual stays whisper (D4); Parakeet v3 is the opt-in European model; Cohere Transcribe only as an opt-in "max accuracy (slow)" server model. `threads = 4` unchanged | On the 40-clip set unified = v2 (1.15% WER, same A/B latency 1.17 s / 2.87 s for 11 / 29 s audio) but it decoded 43 s and 56 s recordings without loss, where v2 greedy dropped an utterance (17.5% WER), v3 and Cohere dropped 23% / 22%, and v2 with the server's beam 4 returned **empty** text on 25 s and 43 s clips. No open model is more accurate *and* interactive on a 4-core AVX2 CPU (Cohere 0.84% WER at RTF 0.52; Qwen3-ASR-0.6B 1.57% at 0.47). Streaming Nemotron gives 0.15–0.33 s release→text at any length for 2.2% WER |

## Open items and caveats

* All latencies were measured on a heavily loaded shared box. Re-run `ab_latency.sh 3 4` on a quiet box before quoting absolute numbers. RAM (RSS) per model was not measured, except v2 in flow-server (1.13 GB).
* Our set is clean read speech. Spontaneous dictation will have higher WER for every model, and the ranking between v2 and unified (equal here) is not resolved by 955 words.
* Not tested here:
  * the unified *streaming* packages (240/560/1120 ms);
  * parakeet-ultra (third-party sherpa export);
  * ivrit.ai Hebrew zipformer;
  * QNN phone builds;
  * Moonshine streaming;
  * NVIDIA's NeMo-Speech.cpp ggml runtime (2026-07; supports Nemotron, Parakeet v3, ctc-1.1b).
* The beam-4 empty result should get a regression test in `flow-asr-sherpa`: concatenate 3 extended-set clips (as in `long43_manifest.json`), decode with beam 4, and assert non-empty text. It needs the model and is skipped without it.

## Files

* Probe (dated): `tools/probes/prototypes/stt_survey_2026_10_23/`
  * `src/main.rs`: warm-recognizer benchmark; kinds transducer / canary / moonshine2 / qwen3 / cohere / online.
  * `run.sh`: the full sequential run.
  * `ab_latency.sh`: interleaved latency.
  * `long_deletion.sh`: 43 / 56 s drop test.
  * `score.py`: WER/RTF scoring with `bench/wer.py`.
  * `long43_manifest.json`, `long60_manifest.json`.
  * Build: `SHERPA_ONNX_LIB_DIR=$CACHE/sherpa-onnx/lib CARGO_TARGET_DIR=$CACHE/stt-survey-target-2 cargo build --release --offline`.
* Raw results: `$CACHE/results/stt-survey-2026-10-23/`
  * `*.jsonl`: one file per run;
  * `ab-20261023-*/`;
  * `longdel-*/`;
  * `beam-long-recheck.txt`;
  * `unified-beam4-long.jsonl`;
  * `run-main.log`.
* Models: `$CACHE/models/survey-2026-10-23/dl-<package>/<package>/`. The Nemotron-3.5 package was downloaded but not benchmarked: its English WER is weak (new-format 9.75).
