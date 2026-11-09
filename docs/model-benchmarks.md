# Local model benchmarks (ASR + cleanup LLM)

Measured 2026-10-20 to check the defaults in `DECISIONS.md` D3–D5 against data. The scripts are in `tools/probes/bench/`. Binaries, models and audio live in the bench cache directory (`$CACHE`, see `run_all.sh`; default `~/.cache/flow-clone`) and are not in git. Set CACHE=<dir> to reuse an existing cache. To reproduce, run `tools/probes/bench/run_all.sh`; the reproduce section at the end has the details.

## TL;DR

| target | ASR default | LLM cleanup | expected latency for a 10 s dictation |
|---|---|---|---|
| **Desktop, CPU only** | **Parakeet-TDT-0.6B int8** (sherpa-onnx) for English. **whisper base q5_1** for other languages or auto-detect. whisper small q5_1 as an opt-in "accurate" mode. | **OFF** by default. Opt-in: Qwen2.5-3B-Instruct Q4_K_M with the few-shot prompt (Qwen2.5-1.5B on slower CPUs). | ASR ≈ 1.2 s (Parakeet) / 2 s (base). The LLM adds 2.5–4.5 s. |
| **Desktop, GPU** (estimate) | whisper large-v3-turbo q5_0 (multilingual). Parakeet on CUDA for English. | **ON** is viable: Qwen2.5-3B, or Qwen2.5-7B, few-shot. | ≈ 0.3–0.6 s ASR plus ≈ 0.3–0.8 s LLM (estimate) |
| **Android** (estimate) | whisper base q5_1 (D4 holds). tiny q5_1 on low-end phones. Parakeet int8 as an English option on flagships. | **OFF**. Opt-in Qwen2.5-1.5B only on phones with ≥ 8 GB RAM. | ≈ 1–2.5 s ASR on a flagship; the LLM adds 1.5–4 s |

What this changes in DECISIONS:
* **D4 (CPU desktop = small q5).** Measured RTF on this box is 0.57–0.86, so a 10 s utterance takes 5–9 s. That is too slow to be the default. Use base q5_1 as the default (RTF 0.20, WER 3.7% vs 2.9% for small on the 40-utterance set), and add Parakeet as the English engine. Parakeet is both faster and more accurate than any whisper model on CPU: WER 1.2%, RTF 0.14.
* **D3 (whisper only, multilingual).** Keep whisper as the multilingual engine, and ship the Parakeet backend behind the ASR trait now. It is not a later nice-to-have.
* **D5 (LLM off by default).** Confirmed for CPU and Android. Two changes:
  * Replace the long rule-list prompt with a **short system prompt plus few-shot turns**. On every model it improved mean similarity by 0.05–0.15 and was the only variant where the ≤3B models produced lists, digits or email layout at all.
  * Fix the edit-distance guardrail: it rejects correct backtracking edits (details below).

## Test machine and caveats

* Intel i7-6700 (Skylake, 2015): 4 cores / 8 threads, AVX2/FMA, no AVX-512. DDR4. 19 GB RAM. HDD RAID0. No GPU.
* whisper.cpp `231638a`, llama.cpp `a01ce9f` (2026-10-20 HEAD). Static CPU builds with OpenMP. Flash-attention is on (the whisper default; turning it off made the small encoder 2.2× slower).
* **The box was shared.** Other agents were running cargo and gradle builds the whole time, with a 1-minute load average of 5–25 on 8 threads (recorded per run in the JSONL). Every latency number here is therefore **pessimistic and noisy**. LLM latencies use the per-case minimum over 2 runs. Quality numbers are not affected: decoding is deterministic, and two independent runs gave identical LLM scores.
* **4 threads beat 8 threads everywhere.** The machine has 4 physical cores, and 8 threads also fight with the background load (llama-bench: 0.5B tg 41.6 → 21.6 tok/s). The clone should default to `threads = physical cores`, capped at about 4–6 on desktop.
* Cold runs evict the model file from the page cache (`posix_fadvise DONTNEED`) and then time one jfk.wav run as a fresh process. The disk is an HDD, so cold times are close to worst case.

## ASR

### Test sets
* **Core set** (`asr_manifest.json`): 13 clips, 117.6 s, 308 words. These are `jfk.wav` (whisper.cpp sample; public domain) and 12 LibriSpeech test-clean utterances, one per speaker, 5–13 s each.
* **Extended set** (`asr_manifest_extended.json`): the first 5–20 s utterance of each of the 40 test-clean speakers. It overlaps the core set by 12 clips. It exists because 308 words cannot tell base and small apart (1 error = 0.3%).
* Licence: LibriSpeech is CC BY 4.0 (OpenSLR SLR12; Panayotov et al. 2015). The audio is read speech from public-domain LibriVox audiobooks. **Caveat:** this is clean read speech. Real dictation is spontaneous and noisier and contains fillers, so expect higher absolute WER for every model. The ranking should hold.
* WER is computed by `wer.py`, a cut-down Whisper English normaliser: lowercasing, removing punctuation, numbers → words, and a few LibriSpeech compound joins such as "to day" → "today".
* Decoding settings:
  * whisper: greedy (`-bs 1 -bo 1`, temperature fallback on), `-l en`;
  * Parakeet: sherpa-onnx `nemo_transducer` greedy.

### Results (warm = model already in page cache; RTF = inference time / audio duration, lower is better)

Core set:

| model (file size) | thr | WER | RTF (inference) | warm jfk process wall | cold jfk process wall | mean latency per clip (≈9 s) |
|---|---|---|---|---|---|---|
| whisper tiny q5_1 (31 MB) | 4 | 5.8% | 0.082 | 0.8 s | 1.1 s | 0.74 s |
| whisper tiny q5_1 | 8 | 5.8% | 0.100 | 0.9 s | 1.3 s | 0.90 s |
| whisper base q5_1 (57 MB) | 4 | 1.3% | 0.196 | 1.6 s | 2.0 s | 1.77 s |
| whisper base q5_1 | 8 | 1.9% | 0.227 | 1.5 s | 2.3 s | 2.05 s |
| whisper small q5_1 (181 MB) | 4 | 1.6% | 0.567 | 5.1 s | 6.5 s | 5.13 s |
| whisper small q5_1 | 8 | 1.6% | 0.607 | 5.5 s | 6.9 s | 5.49 s |
| whisper large-v3-turbo q5_0 (547 MB) | 4 | 1.0% | 3.26 | 25.7 s | 31.1 s | 29.5 s |
| whisper large-v3-turbo q5_0 | 8 | 1.0% | 3.64 | 26.2 s | 29.5 s | 32.9 s |
| Parakeet-TDT-0.6B-v2 int8 (sherpa-onnx, 640 MB) | 4 | **0.6%** | **0.122** | 4.8 s (3.5 s is session init) | 12–47 s | **1.11 s** |
| Parakeet-TDT-0.6B-v2 int8 | 8 | 0.6% | 0.136 | 4.5 s | 43 s | 1.24 s |

Extended set, 40 utterances, 4 threads:

| model | WER | RTF (inference) | mean latency per clip |
|---|---|---|---|
| whisper tiny q5_1 | 6.8% | 0.098 | 0.87 s |
| whisper base q5_1 | 3.7% | 0.245 | 2.16 s |
| whisper small q5_1 | 2.9% | 0.855 | 7.56 s |
| Parakeet-TDT-0.6B-v2 int8 | **1.2%** | **0.137** | **1.21 s** |

(large-v3-turbo was left out of the extended run because one pass takes about 20 min on this CPU.)

Observations:
* **whisper's latency is set by the encoder, and the encoder always processes a 30 s window.** A 5 s clip costs about as much as a 30 s one: tiny encode ≈ 0.6 s, base ≈ 1.2 s, small ≈ 4.5 s, turbo ≈ 25 s at 4 threads. RTF is therefore worst on exactly the short utterances that dictation produces. Parakeet's cost scales with clip length.
* **Shrinking the encoder window with `-ac` (audio_ctx sized to the clip, plus 1 s)** on the core set, 4 threads:

  | model | WER | RTF | vs. full 30 s window |
  |---|---|---|---|
  | tiny | 11.4% | 0.084 | — |
  | base | 11.7% | 0.207 | — |
  | small | 1.0% | 0.308 | 1.8× faster, WER no worse |
  | turbo | 3.6% | 0.985 | 3.3× faster |

  On base, `-ac` caused a repetition loop: one clip had its sentence repeated 3×, which accounts for 30 errors. On tiny it garbled two clips (7–8 errors each), and turbo lost 6 words on one clip. It is a real latency lever, but only with a repetition/length guard (hypothesis words ≤ ~4 × seconds) that re-runs with the full window. Do not enable it blindly.
* **large-v3-turbo is not usable on CPU-only machines of this class.** It takes 25 s for any clip. On a modern 8–16-core AVX-512 CPU it might reach RTF ≈ 0.5–1 (estimate), which is still too slow. It belongs to the GPU tier.
* **Cold start.**
  * whisper: model load is 0.3 s for tiny, 1.5 s for small and 5.6 s for turbo from HDD.
  * Parakeet: initialising the onnxruntime session costs about 3.5 s even when warm, and 10–47 s cold from HDD. The app must preload the engine at startup or on first hotkey press, never per dictation. Inside a resident process only the "inference" column applies.
* Errors are mostly proper nouns: "Battleax" → "Adelax" for every model, "Hollan" → "Holland". The personal-dictionary prompt in the clone should help with these. Small once dropped a trailing "well well". Tiny makes content errors ("training of people" → "train in the people").

## Cleanup LLM

### Setup
* 20 hand-written cases in `cleanup_cases.json`. Case c01 was removed later (19 cases now), so the scores below were computed on a set that still included it.

  | phenomenon | cases |
  |---|---|
  | fillers | c01, c02, c18 |
  | backtracking: "scratch that", "actually I mean", "no wait", "never mind" | c03, c04, c12, c20 |
  | "actually" that is *not* a correction | c05 |
  | spoken punctuation and new line | c06, c07 |
  | lists (3+ items → HTML list, 2 items stay a sentence) | c08, c09, c10 |
  | email layout | c11, c12 |
  | numbers, money, times | c13, c14 |
  | prompt injection (question or command that must *not* be answered) | c15, c16 |
  | only filler → empty output | c17 |
  | already clean | c19 |

  Expected outputs follow the rule list of the **rules** variant below, at level light, style formal.
* Prompt variants:
  * **rules**: `cleanup_system_prompt.txt`, a long rule-list template rendered for light/formal and made app-agnostic so the KV prefix can be cached, with a matching user message. About 600 prefix tokens.
  * **fewshot**: `cleanup_fewshot.json`, a short system prompt plus 7 example turns that do not overlap the test content. About 560 prefix tokens.
* Runtime: `llama-server`, CPU, 4 threads, `temperature 0`, `max_tokens = min(2·words+32, 256)`, the model's own chat template, `cache_prompt` on. The constant prefix is processed once; each dictation then costs only its ~35–70 new prompt tokens plus generation. This is how the clone should run the model: keep the model resident and the prefix cached.
* Quality metrics:
  * **exact**: exact match after whitespace normalisation.
  * **sim**: 1 − character Levenshtein distance / max length. Pasting the raw input unchanged scores **0.712**, so that is the floor to beat.
  * **better/worse**: number of cases where sim beat or fell below the raw input's sim by more than 0.02.
  * **guard**: how many outputs the §7.1 guardrail would replace with raw ASR.
  * **manual A/M/H**: my judgement, recorded in `llm_judgements.json`. A = paste-ready. M = safe, but missed a fix. **H = harmful**: meaning changed, content dropped or added, or junk emitted.

### Quality

| model (Q4_K_M) | prompt | exact | mean sim (raw 0.712) | better/worse | guard rejects | manual A/M/**H** |
|---|---|---|---|---|---|---|
| Qwen2.5-0.5B | rules | 4/20 | 0.775 | 6/0 | 0 | — |
| Qwen2.5-1.5B | rules | 3/20 | 0.754 | 11/1 | 0 | — |
| Qwen2.5-3B | rules | 2/20 | 0.725 | 9/3 | 1 | — |
| Llama-3.2-1B | rules | 2/20 | 0.626 | 4/8 | 6 | — |
| Llama-3.2-3B | rules | 5/20 | 0.825 | 11/1 | 0 | — |
| Qwen2.5-7B (reference) | rules | 6/20 | 0.863 | 14/1 | 2 | — |
| Qwen2.5-0.5B | fewshot | 4/20 | 0.858 | 10/2 | 0 | 9/8/**3** |
| Qwen2.5-1.5B | fewshot | 7/20 | 0.876 | 12/0 | 1 | 14/5/**1** |
| **Qwen2.5-3B** | fewshot | 3/20 | 0.875 | 14/1 | 1 | **16/4/0** |
| Llama-3.2-1B | fewshot | 3/20 | 0.720 | 8/6 | 5 | 8/5/**7** |
| **Llama-3.2-3B** | fewshot | 7/20 | **0.924** | 14/1 | 0 | **16/3/1** |
| Qwen2.5-7B (reference) | fewshot | 9/20 | **0.952** | 14/1 | 1 | 16/2/**2** |

Judgement notes:
* **The rules prompt is too much for small models.**
  * The ≤3B models mostly echo the input: no lists, no digits, no email layout, and "scratch that" is often left in.
  * Qwen2.5-3B answered the c15 injection ("The capital of France is Paris.").
  * Llama-3.2-1B wrapped outputs in `<ol>` and copied "3:30 PM" from the rule text into unrelated outputs.
* **Few-shot fixes most of that.**
  * Lists (c08, c09), money and times (c13, c14) and email layout (c11, c12) start working on the 1.5B and larger models.
  * Every model left c15 and c16 unanswered; only Llama-3.2-1B garbled c16.
* **The harmful failures are what matter for a default-on decision.**
  * Over-applying "never mind" deletes the sentence that follows it (c20: Llama-3B, Qwen-7B, Llama-1B).
  * Content is dropped (c06 "Not quite": Qwen-1.5B, Llama-1B).
  * Qwen-7B appended a stray CJK token (c06, `日记`).
  * Llama-3.2-1B rewrites meaning (c02 becomes "I'll take a look at the build") and repeats sentences.
  * Even the best small model is harmful in roughly 1 out of 20 dictations. That is acceptable for an opt-in feature with "undo AI edit", not for a silent default.
* **Llama-3.2-1B and Qwen2.5-0.5B are not worth shipping.** Llama-1B is net harmful (7 of 20 cases). Qwen-0.5B makes few changes and is still harmful in 3 of 20 cases.
* **The §7.1 guardrail (word edit distance / words(raw) > 0.6 → paste raw) rejects correct backtracking.** For example, c03 "…3pm. Scratch that, … library at 4pm" → "Let's meet at the library at 4:00 PM" was rejected for Qwen-7B. It barely catches real harm: the dropped-sentence c20 failures pass at a ratio of about 0.3. The clone should:
  * exempt utterances that contain a backtrack trigger, or compare against the deterministic backtrack-resolved text instead of raw;
  * add a check that each output sentence is **contained** in the input, a better signal for dropped content than edit distance.
* **The 30–60% of fixes that the LLM gets right are mostly deterministic.** Spoken punctuation, "scratch that" and "no wait" backtracking, filler removal, and money/time normalisation (an ITN table) can all be done reliably in the post-processing pass that is always on. That is a stronger default than a small LLM.

### Latency (CPU, 4 threads, contended box; per-case min over 2 runs)

| model | file size | per dictation: mean (p90) | new prompt tok (prefix cached) / output tok | best observed gen tok/s | llama-bench pp512 / tg64 tok/s | first request (uncached ~600-tok prefix + 60 tok) | server load |
|---|---|---|---|---|---|---|---|
| Qwen2.5-0.5B | 0.47 GB | **1.0 s** (1.6) | 36 / 23 | 40 | 112 / 26 (earlier, at lower load: 193 / 42) | 5.3 s | 1–18 s |
| Qwen2.5-1.5B | 1.07 GB | **2.4 s** (3.8) | 36 / 23 | 19 | 52 / 14 | 22 s | 25–35 s |
| Qwen2.5-3B | 1.96 GB | **3.9 s** (6.4) | 36 / 20 | 10 | 23 / 6.5 | 22 s | 23–61 s |
| Llama-3.2-1B | 0.77 GB | 1.4 s (2.3) | 35 / 19 | 23 | 72 / 16 | 9.4 s | 8–16 s |
| Llama-3.2-3B | 1.88 GB | 4.5 s (6.5) | 35 / 23 | 8 | 35 / 6.9 | 23 s | 22–29 s |
| Qwen2.5-7B | 4.36 GB | 17 s (33) | 36 / 21 | 4 | 3.4 tg | 50 s | 81–128 s |
| Qwen2.5-1.5B @ 8 threads | | 4.9 s (6.8) | | 11 | | | |
| Qwen2.5-3B @ 8 threads | | 8.0 s (8.9) | | 7 | | | |

* A "60-token" fixed generation (cold prompt, `ignore_eos`) is in each JSONL as `fixed60`. It is dominated by the one-time ~600-token prefix fill, which is why the "first request" column is so large. With prefix caching the real per-dictation cost is ≈ new prompt tokens / pp + output tokens / tg. Output tokens average about 20–23 because typical dictations are short.
* Server load times are inflated by the HDD and by contention. Keep the server resident; loading the model per dictation is not an option.
* Rough uncontended estimate from the best observed rates: Qwen-1.5B ≈ 1.5–2 s per dictation and Qwen-3B ≈ 2.5–3.5 s on this CPU. A current desktop CPU (Zen 4 / Raptor Lake, DDR5, roughly 2–3× this box's memory bandwidth) should roughly halve that (estimate).

## Phone feasibility (estimate: no device measured)

Method: decode speed (tg) is bound by memory bandwidth; prompt fill (pp) and the whisper encoder are bound by compute. The reference box has dual-channel DDR4-2133 (≈34 GB/s peak) and 4 Skylake cores.

| tier (example SoC) | vs this box | whisper base, 10 s clip | Parakeet int8 | Qwen2.5-1.5B per dictation | Qwen2.5-3B |
|---|---|---|---|---|---|
| flagship 2024–25 (Snapdragon 8 Gen 3 / 8 Elite, LPDDR5X) | tg ≈ 1–1.5×; pp/encoder ≈ 1–2× (i8mm int8 dot-product kernels) | ≈ 0.8–1.5 s | ≈ 0.7–1.5 s | ≈ 1.5–2.5 s | ≈ 3–5 s, 2.3 GB resident |
| mid-range (Snapdragon 7-series / Tensor G2, LPDDR4X/5) | ≈ 0.5–0.8× | ≈ 2–3 s | ≈ 2–3 s | ≈ 3–5 s | not viable |
| low-end (≤ 4 GB RAM) | ≈ 0.3× | use tiny ≈ 1.5–2.5 s | not viable (640 MB model) | not viable | not viable |

Phone caveats: thermal throttling after about 30–60 s of sustained load (−30–50%); the low-memory killer on a background accessibility service holding a 1–2 GB model; battery. whisper base q5_1 (57 MB, 1.7 s per clip here) is the safe Android default. Parakeet is the accuracy upgrade on flagships, for English only with v2. The multilingual `parakeet-tdt-0.6b-v3` (25 European languages, also packaged for sherpa-onnx) was not benchmarked. LLM cleanup on Android should stay opt-in and limited to phones with ≥ 8 GB RAM, using Qwen2.5-1.5B.

## Desktop GPU (estimate: no GPU on the test box)

Extrapolated from published whisper.cpp and llama.cpp CUDA/Vulkan results for a mid-range RTX 3060-class card:
* whisper large-v3-turbo q5_0: encoder ≈ 0.2–0.4 s per 30 s window, ≈ 0.3–0.6 s per dictation.
* Qwen2.5-3B Q4_K_M: pp in the thousands of tok/s, tg ≈ 80–120 tok/s, so ≈ 0.3–0.5 s per dictation.
* Qwen2.5-7B Q4_K_M: tg ≈ 50–70 tok/s, so ≈ 0.5–0.8 s.

At those speeds, cleanup is cheap enough to default **ON** on GPU machines with Qwen2.5-3B, or 7B with ≥ 8 GB VRAM. Use the few-shot prompt and the improved guardrail, and keep "undo AI edit" one click away. iGPU (Vulkan) results vary too much to estimate; treat it as CPU tier until measured.

## Recommended defaults (for D4/D5 update)

1. `asr.engine`:
   * `parakeet-tdt-0.6b-v2-int8` when the user's language is English and the device has ≥ 4 GB free RAM.
   * Otherwise `whisper-base-q5_1`.
   * GPU detected: `whisper-large-v3-turbo-q5_0`.
   * Android low-RAM: `whisper-tiny-q5_1`.
   * `whisper-small-q5_1` is a user-selectable "accurate (slow)" option on CPU.
2. `asr.threads = min(physical_cores, 6)`. Preload the engine at app start.
3. `llm.cleanup`:
   * Default off on CPU and Android.
   * Default on when a GPU with ≥ 6 GB VRAM is present.
   * Opt-in model: Qwen2.5-3B-Instruct Q4_K_M (Qwen2.5-1.5B on Android or slow CPUs), using the few-shot prompt with a cached prefix.
4. Move fillers, spoken punctuation, explicit backtrack phrases and number/money/time ITN into the deterministic, always-on post-processing pass. Most of the LLM's useful work in this test was in these categories.
5. Revise the cleanup prompt in `flow-core`:
   * short system prompt plus few-shot examples;
   * app and category go in the user turn, so the prefix is cacheable;
   * guardrail: containment check plus a backtrack exemption.

## Reproduce

```
tools/probes/bench/run_all.sh          # build, fetch models + audio, run ASR + LLM + llama-bench
tools/probes/bench/run_all.sh skip-setup
python3 -I tools/probes/bench/report_llm.py "$CACHE"/results/llm-*.jsonl              # per-case outputs
python3 -I tools/probes/bench/report_llm.py --summary --judgements tools/probes/bench/llm_judgements.json <files>
python3 -I tools/probes/bench/report_llm.py --latency <files>
tools/probes/bench/fetch_models.sh llm7b   # optional 7B reference model (4.4 GB)
```

| file | role |
|---|---|
| `build_engines.sh` | clones and builds whisper.cpp and llama.cpp into the cache |
| `fetch_models.sh` | downloads models from Hugging Face and the sherpa-onnx GitHub releases |
| `fetch_audio.py` | needs the cache venv with `soundfile`; downloads the 346 MB test-clean tarball once and extracts the manifest clips |
| `bench_asr.py`, `bench_llm.py` | benchmarks; write per-session JSONL to `.cache/results/` (never overwritten) |
| `wer.py` | WER and normaliser; run it with no arguments for its self-test |
| `run_2026_10_20_extra.sh` | dated probe for the extended-set, `-ac`, llama-bench and whisper-bench follow-ups |

Raw result files from this session (in `.cache/results/`):

| result | file |
|---|---|
| ASR core | `asr-20261020-151539.jsonl` |
| ASR core, Parakeet | `asr-20261020-154455.jsonl` |
| ASR extended | `asr-asr_manifest_extended-20261020-165225.jsonl` |
| ASR `-ac` | `asr-asr_manifest-ac-20261020-170*.jsonl` |
| LLM rules | `llm-20261020-155222-t4.jsonl`, `llm-rules-20261020-162610-t4.jsonl` |
| LLM fewshot | `llm-fewshot-20261020-160825-t4.jsonl`, `-161801-t4`, `-170741-t4`, `-171725-t8` |

## Hybrid: local vs via flow-server (2026-10-22)

Smoke test of the D17 path on the i7-6700 (load ≈ 8-11 from other jobs):
a release `flow-server` (Parakeet-TDT-0.6B-v2 int8, 4 threads, 1 worker) in
the foreground on `127.0.0.1:17313` with a bearer token, and the same 10
LibriSpeech clips (5.3-14.4 s) dictated in-process (FlowCore, Parakeet,
4 threads) and through `Router` → `flow-remote` → server. Audio was pushed in
100 ms frames at real-time pace, so the number is commit → result
("release → text"). Script: `tools/probes/hybrid_smoke/run.sh`; raw rows:
`docs/bench/hybrid-2026-10-22/latency.tsv`.

| | Local (in-process) | Via server (loopback) |
|---|---|---|
| commit → result, mean of 10 | 1687 ms | 1666 ms |
| ASR time inside it | = commit | server `asr_ms` |
| Overhead beyond ASR (remote commit − server ASR) | — | median 3 ms (1-10 ms, one 112 ms outlier) |
| Probe RTT (hello + 3 pings) | — | 0-1 ms |
| Transcript equal to local | — | 10 / 10 |
| Server model load before listening | — | 16.8 s (cold) |

Both sides ran on the same loaded CPU one after the other, so per-file
differences (e.g. 720 vs 2045 ms) are load noise, not protocol cost: the
protocol and pipeline overhead is single-digit milliseconds on loopback.
Over a VPN add the network RTT (a few ms on a LAN path). The win of the
server mode comes from running on a faster machine than the client (a phone),
which this single-box test cannot show. After the run the server's state
directory (its cwd, HOME and TMPDIR) was empty, and its log had 10 `result`
lines with no transcript text.
