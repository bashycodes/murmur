# Laptop model setup on 0.12.0, 2026-10-31

> **Status:** ops record + one benchmark. Hardware: Core Ultra 9 288V, 32 GB, Arc 140V
> iGPU (Vulkan). Probe: `tools/probes/prototypes/laptop_ops_2026_10_31/`.
> Builds on `2026-10-30-laptop-model-benchmark.md`.

## What was done

1. **Backup.** `%APPDATA%\FlowClone\prefs.json.bak-20261031-194509-pre-0.12.0` (the
   0.11.1 prefs, before any change).
2. **Install.** 0.12.0 hybrid NSIS (`Murmur_0.12.0_x64-setup_hybrid.exe`, SHA-256
   `1946f5e7…0abc73`, equal to `SHA256SUMS-windows-0.12.0.txt`). The app was closed,
   then the installer ran with `/S` (exit 0). The exe reports 0.12.0. The install
   folder now has `flow_llm.dll` and `flow_llm_vulkan.dll` (0.11.1 had neither).
3. **Models.** Fetched from the catalogue's pinned URLs (`model_download.rs`) into
   `%APPDATA%\FlowClone\models`, the same paths the in-app downloader uses. Size and
   SHA-256 match for every file:
   * `Llama-3.2-3B-Instruct-Q4_K_M.gguf` (2 019 377 696 B, `6c1a2b41…`)
   * `zipformer2-66m-streaming-he-int8\{encoder,decoder,joiner}.int8.onnx, tokens.txt`
   * `ggml-tiny-q5_1.bin`, the language-ID model (32 152 673 B, `81871056…`)
   * English: the current Parakeet unified-en int8 folder is complete, so it was kept.
     Parakeet v2, ggml-base, ggml-small-q5_1 and Qwen 1.5B are still in the folder.
4. **Prefs.** These were edited with the app closed (`set_prefs.py`):

   | key | before | after |
   |---|---|---|
   | `llm_enabled` | false | **true** |
   | `llm_model_path` | qwen2.5-1.5b | **`models\Llama-3.2-3B-Instruct-Q4_K_M.gguf`** |
   | `llm_lib_path` | null | null. The app's backend probe picks `flow_llm_vulkan.dll` |
   | `developer.cleanup_engines` | cloud (on), local (on), server (off) | **local (on), server (off), cloud (on)** |
   | `developer.allow_cloud` | false | false. Cloud cleanup stays off |
   | `hebrew_model` | zipformer | zipformer |
   | `keyboard_language` | (absent, rev 3) | **true**. D37 "Keyboard language" |
   | `hands_free_silence_ms` | 0 | 0 |

   The server engine stays off because the home server's binary predates `clean`
   (D36), so turning it on would add nothing.

## Verification (app relaunched in the interactive session)

* The process loads `flow_llm_vulkan.dll`, `vulkan-1.dll` and `igvk64.dll` (the Intel
  Vulkan driver).
* stderr shows `ggml_vulkan: 0 = Intel(R) Arc(TM) 140V GPU (16GB) … matrix cores:
  KHR_coopmat`.
* After startup the process holds **~2.8 GB of GPU memory**: the 3B weights (1.9 GB),
  plus the KV cache and the compute buffers. Qwen 1.5B would hold about 1.1 GB of
  weights.
* No `[llm]` problem line was printed, so the cleanup model loaded. The tray line reads
  "AI cleanup: … (GPU)" when the backend is a GPU.
* stderr shows `whisper_model_load` of `models\ggml-tiny-q5_1.bin`, so the language-ID
  model loaded. The app loads it only together with the Hebrew route (`load_lid` needs
  a `LanguageSpec`). No `[model]` problem line was printed, so the zipformer route
  loaded.
* Prefs survived the start: llm on, the 3B model, local first, cloud off, keyboard
  language on, hands-free pause 0.
* Not done: a real dictation. That needs the owner at the mic. The first cleanup will
  also run the Vulkan shader warm-up.

## Benchmark: Hebrew, zipformer vs ivrit turbo on Vulkan

This is the same setup as on 2026-10-30: 49 FLEURS he_il dev clips (416 s, 793 words;
`he_manifest.json`), one process per clip, latency = compute minus model load, greedy.

* **Whisper on Vulkan.** whisper.cpp ships no Windows Vulkan binary. Dropping llama.cpp
  b12005's `ggml-vulkan.dll` into whisper b5454 alone does not load (silently
  rejected). Replacing **the whole `ggml*.dll` set** with llama.cpp b12005's works: the
  Vulkan0 backend is used and the English output is correct. Setup took about
  10 minutes.
* **ivrit turbo.** The f16 file was pinned (`c8090411…`) and quantized to q5_0 on the
  laptop. The result is byte-identical to the 2026-10-30 file (`6c1da92e…a572`).

| model | engine | WER | RTF | mean ms | p90 ms | max ms |
|---|---|---|---|---|---|---|
| zipformer2-66m-streaming-he int8 (current) | sherpa online, CPU 4 t | 27.2 % | 0.042 | 361 ¹ | 550 | 820 |
| ivrit whisper-large-v3-turbo q5_0 | whisper.cpp **Vulkan** | 20.5 % | 0.114 | 971 | 1144 | 1231 |
| **ivrit whisper-large-v3-turbo q5_0, `-ac`** | whisper.cpp **Vulkan** | **20.4 %** | 0.052 | **445** | **601** | 934 |
| ivrit whisper-large-v3-turbo f16 | whisper.cpp **Vulkan** | 20.3 % | 0.153 | 1300 | 1553 | 2253 |
| (2026-10-30) ivrit turbo q5_0 `-ac` | whisper.cpp CPU 8 t | 20.3 % | 0.507 | 4306 | 6460 | |

¹ Total compute. When streaming, the zipformer finishes about 50 ms after the last
audio. Whisper runs after release.

* The Vulkan times include the per-process pipeline setup, so a resident engine would
  be somewhat faster.
* The app was running during the runs, with the 3B model resident on the same GPU.

**Recommendation (not switched).** On this GPU, ivrit turbo q5_0 is **6.8 points more
accurate** than the zipformer: 20.4 % vs 27.2 % WER. It takes 0.45 s mean and 0.6 s p90
after release (with `-ac`), or about 1.0 s without `-ac`. That is under the 1.5 s bar,
so it is the better Hebrew model for a laptop with a GPU.

The app can't run it today:
* The app's whisper engine is CPU-only: its log says `use gpu = 0` / `no GPU found`.
* There is no catalogue entry for it, and it needs on-device quantization or a
  hosted q5_0 file.

What it needs:
* a Vulkan whisper build in the Windows app, the way `flow_llm_vulkan.dll` was done
  for cleanup;
* a catalogue entry.

`-ac` raised English WER on 2026-10-30 (1.7 → 2.6 %) and is untested on long
dictations, so the plain q5_0 (about 1 s) is the safer default.

## Rollback

1. Close the app.
2. Restore `prefs.json.bak-20261031-194509-pre-0.12.0` over `prefs.json`.
3. Optionally reinstall 0.11.1 from its release with `/S`.

The new model files can stay or be deleted from `models\`. They are about 2.1 GB.

Leftovers in the bench folder (on the laptop):
* `models\ivrit-turbo-q5_0.bin` (574 MB)
* `audio-he\` (13 MB)
* `bin\whisper-vk2` (whisper + llama ggml DLLs)
* `results\asr-he-20261031.jsonl`
