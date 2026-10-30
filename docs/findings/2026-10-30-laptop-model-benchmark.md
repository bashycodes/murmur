# 2026-10-30: laptop model benchmark (STT + AI cleanup)

**Question.** Can the owner's Windows PC run a better STT or
AI-cleanup model than today's defaults, within the ≤ 1.5 s cleanup budget?

**Answer.**

* **AI cleanup: yes, with the iGPU.** Through Vulkan on the Arc 140V,
  **Llama-3.2-3B-Instruct Q4_K_M** runs prompt v4c (raw transcript, AI cleanup)
  at **p50 0.62 s / p90 1.18 s** and scores **80 %** on the 88-suite with
  target-closeness **0.876**. That closeness equals the OpenRouter Llama-3.3-70B
  (0.879, same guard binary). **Qwen2.5-1.5B** on Vulkan is the fast option:
  p50 0.39 s, 75 %, 0.855.
* **On the CPU alone** (today's `flow_llm.dll`), Qwen2.5-1.5B takes p50 1.0 s / p90 1.7 s
  (74 %). That is faster than on the home server (1.5 s), but it is at the edge of the
  budget. The shipped default, Qwen2.5-0.5B, scores **43 %**, which is unusable for
  AI cleanup.
* **No local model beats the rules on the 88-suite.** The scores are 80–86 % against
  89 % for the rules alone and 92 % for production (2026-10-26). The local AI is
  better only on target-closeness. This is the same conclusion as the 2026-10-26
  benchmark, but now with a 3B model inside the budget.
* **STT, English: keep Parakeet.** It has 1.2 % WER and takes 0.6–0.75 s per
  ~9 s clip on 4 threads. Nothing tested beats it. Large-v3-turbo has 1.7 % WER and
  takes 14.5 s on the CPU.
* **STT, Hebrew: ivrit-ai whisper-large-v3-turbo is clearly more accurate.** Its WER
  is **20.3 %**, against 27.3 % for the zipformer and 28.3 % for Parakeet-he. On the
  CPU it takes 4.3 s mean and 6.5 s p90 (with `-ac`), which is too slow. It needs a
  GPU (Vulkan) whisper build, and none was available to test (§ App changes). Until
  then the zipformer stays the pick.

## Specs (measured 2026-10-30)

| | |
|---|---|
| CPU | Intel Core Ultra 9 288V (Lunar Lake): 8 cores, 8 threads (4 P + 4 LP-E), no SMT |
| RAM | 32 GB LPDDR5X (31.6 GB visible), shared with the GPU; about 9.5 GB free during the runs |
| GPU | Intel Arc 140V (Xe2 iGPU), driver 32.0.101.8860. Vulkan reports 18 425 MiB of shared memory. No NVIDIA card, so no CUDA. There is also an NPU, which was not tested |
| OS | Windows 11 Home 10.0.26200 |
| Python | 3.13.5 (installed) |
| App today | `asr_engine = parakeet` (parakeet-unified-en-0.6b int8), `llm_enabled = false`. Models folder: Parakeet v2 + unified, ggml-base, ggml-small-q5_1 |

## Method

* **Binaries.** Official release builds only, run at below-normal priority:
  * llama.cpp **b12005** `win-cpu-x64` and `win-vulkan-x64`;
  * whisper.cpp **b5454** `whisper-bin-x64` (CPU; there is no Windows Vulkan release);
  * sherpa-onnx **v2.0.1** `win-x64-shared-MT-Release-no-tts`.
* **Downloads.** About 5.1 GB from the internet: the two 4B GGUFs and about 100 MB of
  binaries. The other models were copied over the LAN from the home cache. App models
  were read in place from `%APPDATA%\FlowClone\models`. Everything was deleted
  afterwards except the binaries and results in `wispr-bench\` (252 MB).
* **Probe:** `tools/probes/prototypes/laptop_bench_2026_10_30/` (see its README).
* **STT.** One process per clip; latency is the engine's own compute time with model
  load excluded (whisper: total − load; sherpa: "Elapsed seconds"). Greedy decoding,
  `-bs 1 -bo 1`.
  * English: the 40-clip LibriSpeech set (`tools/probes/bench/asr_manifest_extended.json`,
    353 s), scored with `wer.py`.
  * Hebrew: 49 FLEURS he_il dev clips (416 s, 793 words; `he_manifest.json`). The
    normaliser strips nikud and punctuation, as on 2026-10-28.
  * ivrit turbo: `ivrit-ai/whisper-large-v3-turbo-ggml` f16 at rev `2130c78e`
    (sha256 `c8090411…`), quantized by us to q5_0 with `whisper-quantize`
    (574 041 195 bytes, sha256 `6c1da92e…a572`).
* **AI cleanup.** The 102 cases of the 2026-10-26 benchmark (`v4c_cases.jsonl`: the
  88-suite plus 14 Wispr targets) run through prompt **v4c**.
  * Rendering: `llm_win.py` builds the messages as flow-core's
    `build_prompt_versioned(…, PromptScope::FULL, V4c)` does, with the session
    `sessions/prod.json` (casual for chats).
  * Server: llama-server with `-c 4096 -np 1`, greedy, `max_tokens = 2 × transcript tokens + 32`
    (the flow-llm rule), `cache_prompt`, and the static prefix prefilled once (as at
    app model load).
  * Vulkan runs use `-ngl 99`. CPU runs use `-t 4` or `-t 8`.
  * `llm_ms` is the request wall time on the PC.
  * The answers then go through `ai-cleanup-bench --mode regard` (product strict
    guard, fallback rules on reject, post-processing), built from this branch, and
    `score.py`.
  * The 70B and 1.5B reference rows from 2026-10-26 were re-judged with the same
    binary, so the 70B shows 0.879 here against the 0.910 it scored then.

## AI cleanup results (prompt v4c, raw input, strict guard)

| model (Q4_K_M) | backend | 88-suite | target-closeness (14) | Wispr exact | guard rejects | LLM ms p50 / p90 | calls > 1.5 s | gen tok/s |
|---|---|---|---|---|---|---|---|---|
| Qwen2.5-0.5B (app default) | CPU t4 | 43 % | 0.804 | 2/14 | 6 | 564 / 934 | 3/102 | 59 |
| Qwen2.5-0.5B | Vulkan | 42 % | 0.823 | 2/14 | 5 | 200 / 312 | 1/102 | 122 |
| Qwen2.5-1.5B | CPU t4 | 74 % | 0.813 | 2/14 | 6 | 1041 / 1708 | 15/102 | 31 |
| Qwen2.5-1.5B | CPU t8 | 76 % | 0.813 | 2/14 | 7 | 1001 / 1720 | 16/102 | 29 |
| **Qwen2.5-1.5B** | **Vulkan** | 75 % | 0.855 | 3/14 | 7 | **388 / 739** | 2/102 | 59 |
| Qwen2.5-3B | Vulkan | 67 % | 0.836 | 1/14 | 6 | 660 / 1139 | 7/102 | 34 |
| **Llama-3.2-3B** | **Vulkan** | **80 %** | **0.876** | 3/14 | 8 | **621 / 1176** | 6/102 | 32 |
| Qwen3-4B-Instruct-2507 | Vulkan | 78 % | 0.873 | 3/14 | 4 | 874 / 1594 | 14/102 | 25 |
| Gemma-3-4B-it | Vulkan | 74 % | 0.846 | 1/14 | 3 | 2826 / 3465 ¹ | 102/102 | 28 |
| Gemma-3-4B-it, `--swa-full` | Vulkan | 75 % | 0.847 | 1/14 | 3 | 823 / 1582 | 14/102 | 25 |
| Llama-3.1-8B | Vulkan | 86 % | 0.858 | 2/14 | 7 | 1441 / 4387 | 47/102 | 10 |
| *ref: Llama-3.3-70B, OpenRouter (2026-10-26)* | cloud | 90 % | 0.879 | 3/14 | 4 | 421 / 728 ² | – | – |
| *ref: rules only* | – | 89 % | 0.759 | 1/14 | – | – | – | – |
| *ref: production, 1.5B, home server* | CPU i7-6700 | 92 % | 0.828 | 2/14 | 0 | 1374 / 2746 | – | – |

¹ Gemma 3 uses sliding-window attention: without `--swa-full`, llama-server cannot reuse
the cached prefix and re-processes about 1 900 prompt tokens on every call.
² From the home server, including the network.

Median LLM ms by dictation length (words; n = 52 / 41 / 5 / 4):

| run | ≤ 10 | 11–20 | 21–35 | > 35 |
|---|---|---|---|---|
| Qwen2.5-1.5B Vulkan | 339 | 456 | 748 | 1454 |
| Llama-3.2-3B Vulkan | 552 | 721 | 1176 | 2303 |
| Qwen3-4B Vulkan | 773 | 1021 | 1594 | 3340 |
| Qwen2.5-1.5B CPU t4 | 796 | 1099 | 1640 | 2682 |
| Llama-3.1-8B Vulkan | 1322 | 1835 | 5219 | 7332 |

Observations:

* **The 3B models fit only on the GPU.**
  * Generation: the iGPU decodes about 32 tok/s for a 3B model and 59 tok/s for a
    1.5B, against 31 tok/s on the CPU for the 1.5B.
  * Prefill of the per-dictation user message (median 33 tokens): 251 ms for the 3B
    and 151 ms for the 1.5B on Vulkan, against 448 ms for the 1.5B on the CPU. Short
    batches run slowly on the iGPU, so this is roughly 40 % of the 3B's p50.
  * One-off costs: the first request compiles shaders and prefills the prefix in
    2–5 s, and model load takes 3 s for the 3B.
* **Llama-3.2-3B errors** (`results/score.txt`, `score.py --fails`): spoken quote
  commands kept as words ("unquote" left in) and corrections not applied ("three,
  sorry, four" kept). It also keeps "um, so", turns "five five five…" into "5-5-5",
  and does not split a sign-off. These are the same weak spots the rules handle exactly.
* **8 CPU threads do not help.** The 4 LP-E cores add nothing for llama.cpp (t8 ≈ t4).
* **Llama-3.1-8B is the most accurate local model but too slow**: 10 tok/s, and dictations
  over 20 words take 5–7 s.

## STT results

English, 40 LibriSpeech clips (mean 8.8 s); compute ms per clip, model resident:

| model | engine | threads | WER | RTF | mean ms | p90 ms |
|---|---|---|---|---|---|---|
| **Parakeet-TDT-0.6B-v2 int8** | sherpa offline | 4 | **1.2 %** | **0.069** | **606** | 790 |
| Parakeet-unified-en-0.6B int8 (app's current) | sherpa offline | 4 | 1.2 % | 0.085 | 755 | 1037 |
| Parakeet-unified-en-0.6B int8 | sherpa offline | 8 | 1.2 % | 0.192 | 1694 | 2172 |
| whisper base (f16) | whisper.cpp CPU | 4 | 3.0 % | 0.115 | 1015 | 1072 |
| whisper small q5_1 | whisper.cpp CPU | 4 | 2.9 % | 0.481 | 4248 | 5036 |
| whisper small q5_1 | whisper.cpp CPU | 8 | 2.9 % | 0.336 | 2972 | 3354 |
| whisper large-v3-turbo q5_0 | whisper.cpp CPU | 8 | 1.7 % | 1.644 | 14522 | 16622 |
| whisper large-v3-turbo q5_0, `-ac` | whisper.cpp CPU | 8 | 2.6 % | 0.479 | 4229 | 6462 |

Hebrew, 49 FLEURS dev clips (mean 8.5 s):

| model | engine | threads | WER | RTF | mean ms | p90 ms |
|---|---|---|---|---|---|---|
| zipformer2-66m-streaming-he int8 | sherpa online (whole file) | 4 | 27.3 % | 0.042 | 353 ³ | 520 |
| parakeet-tdt-0.6b-v3-he int8 | sherpa offline | 4 | 28.3 % | 0.050 | 425 | 577 |
| **ivrit-ai whisper-large-v3-turbo q5_0** | whisper.cpp CPU | 8 | 20.9 % | 1.561 | 13260 | 14193 |
| **ivrit-ai whisper-large-v3-turbo q5_0, `-ac`** | whisper.cpp CPU | 8 | **20.3 %** | 0.507 | 4306 | 6460 |

³ Total compute. Streaming does most of it while the user speaks: on 2026-10-28 the
zipformer finished 48 ms after the last audio.

* The zipformer and Parakeet-he scores match 2026-10-28 (26.8 / 28.8 % on a similar
  50-clip set), so the setup is consistent with that run.
* **sherpa-onnx at 8 threads is 2.2× slower than at 4** on this hybrid CPU. whisper.cpp
  gains from 8 threads (small: 4.2 s → 3.0 s).

## Recommendation for the laptop

| role | pick | why |
|---|---|---|
| English STT | **Parakeet-TDT-0.6B-v2 int8** (or the current unified-en: same WER, 25 % slower), 4 threads | 1.2 % WER, about 0.6 s per 9 s clip on the CPU; nothing better found |
| Hebrew STT | **zipformer2-66m-streaming-he** now; **ivrit turbo q5_0 on GPU** once whisper is built with Vulkan | ivrit is 7 points better on WER, but 4.3 s on the CPU is not interactive |
| AI cleanup with a GPU build | **Llama-3.2-3B-Instruct Q4_K_M, Vulkan** | best local quality inside the budget (p50 0.62 s); target-closeness equal to the cloud 70B |
| AI cleanup, CPU-only build (today) | **Qwen2.5-1.5B-Instruct Q4_K_M** (replaces the 0.5B default) | 74 % vs 43 %; p50 1.0 s, p90 1.7 s |
| AI cleanup, fast GPU option | Qwen2.5-1.5B-Instruct Q4_K_M, Vulkan | p50 0.39 s; 5 points below Llama-3B |

Not recommended:
* **Qwen2.5-3B:** worse than Qwen2.5-1.5B on this prompt, and slower.
* **Gemma-3-4B:** needs SWA-full cache handling, and is no better.
* **Qwen3-4B-2507:** as good as Llama-3B but 40 % slower; a reasonable second choice if
  the Llama licence is a problem.
* **Llama-3.1-8B:** too slow.

## Catalogue entries (cleanup GGUFs)

Same shape as `apps/flow-windows/src/model_download.rs` (`resolve/<40-hex commit>/<file>`).
The SHA-256 values are the Hugging Face LFS ids. The first two were also verified with
`sha256sum` on the files used in this run.

| id | HF repo | revision | file | size (bytes) | SHA-256 | licence |
|---|---|---|---|---|---|---|
| `llama-3.2-3b-instruct-q4_k_m` | `bartowski/Llama-3.2-3B-Instruct-GGUF` | `5ab33fa94d1d04e903623ae72c95d1696f09f9e8` | `Llama-3.2-3B-Instruct-Q4_K_M.gguf` | 2 019 377 696 | `6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff` | Llama 3.2 Community |
| `qwen2.5-1.5b-instruct-q4_k_m` | `Qwen/Qwen2.5-1.5B-Instruct-GGUF` | `91cad51170dc346986eccefdc2dd33a9da36ead9` | `qwen2.5-1.5b-instruct-q4_k_m.gguf` | 1 117 320 736 | `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e` | Apache-2.0 |
| `qwen3-4b-instruct-2507-q4_k_m` (alt.) | `unsloth/Qwen3-4B-Instruct-2507-GGUF` | `a06e946bb6b655725eafa393f4a9745d460374c9` | `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` | 2 497 281 120 | `3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597` | Apache-2.0 |
| `qwen2.5-0.5b-instruct-q4_k_m` (current default) | `Qwen/Qwen2.5-0.5B-Instruct-GGUF` | `9217f5db79a29953eb74d5343926648285ec7e67` | `qwen2.5-0.5b-instruct-q4_k_m.gguf` | 491 400 032 | `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` | Apache-2.0 |

The ivrit turbo q5_0 has no official quantized file. A catalogue entry would need
either the official f16 (`ivrit-ai/whisper-large-v3-turbo-ggml` @ `2130c78e4a9cb4914cc4df91a1c3031407789705`,
`ggml-model.bin`, 1 624 555 275 bytes, sha256 `c8090411113357097bfafc2b8e228ec1639fa7f5fe4ecb5d054ac0ccef8641b1`)
or quantizing on the device.

## App changes needed

1. **A GPU build of `flow_llm.dll`.**
   * Enable `llama-cpp-2`'s `vulkan` feature (`llama-cpp-sys-2/vulkan`) for the Windows
     build. CI needs the Vulkan SDK (`glslc`).
   * Pass `LlamaModelParams::default().with_n_gpu_layers(99)`; `crates/flow-llm/src/engine.rs`
     loads with default params today.
   * Fall back to the CPU when there is no Vulkan device, or ship the CPU and Vulkan
     DLLs side by side.
   * Prefill and shader compilation at model load take 2–5 s; this is already covered
     by the load-time prefill.
2. **Local AI cleanup route.**
   * Prompt v4c on the raw transcript runs only for a cloud cleaner today (`engine.rs`:
     `ai_split`). The local LLM uses the rules → v2 → guard path.
   * A fast local GPU model would need the same route behind a setting.
   * Alternatively, keep production's rules-first path and only swap the model. That
     path was not measured on the PC.
3. **LLM entries in the model catalogue.** `CATALOG` is STT-only, the test pins 2–4
   entries, and `ModelEngine` has no LLM kind. `DEFAULT_LLM_FILE` (0.5B) should become
   the 1.5B on the CPU, or Llama-3.2-3B when a GPU is present.
4. **Threads.** Use 4 threads for sherpa on hybrid CPUs; 8 is 2.2× slower here.
5. **Hebrew on GPU.** whisper.cpp with Vulkan (the `vulkan` feature of whisper-rs or
   whisper.cpp; there is no official Windows Vulkan binary to test with) is what would
   make ivrit turbo interactive.
   * **Estimate**, not measured: the turbo encoder at RTF ≈ 0.05–0.1 on this iGPU.
6. **Avoid Gemma 3** unless flow-llm sets the SWA-full context flag; `llama-cpp-2` 0.1.160
   has no setter for it.
