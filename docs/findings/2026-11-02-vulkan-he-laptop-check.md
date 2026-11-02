# Vulkan Hebrew whisper on the laptop, 1.0.0, 2026-11-02

> **Status:** device acceptance check for D42 (ticket "Windows: Hebrew whisper (ivrit turbo)
> on the GPU via Vulkan"). Hardware: Core Ultra 9 288V, Arc 140V iGPU.
> Probe: `tools/probes/prototypes/vulkan_he_check_2026_11_02/`.
> Builds on `2026-10-31-laptop-model-setup.md`.

## Verdict

| acceptance item | result |
|---|---|
| ~10 s Hebrew clip under ~1.5 s on the GPU | **Borderline.** About 1.5 s: median 1.50 s and max 2.0 s over the 13 clips of 8.5–11.5 s. Over all 49 clips p50 is 1.41 s and p90 1.88 s |
| GPU actually used | **Yes** |
| CPU fallback works | **It works but is unusable.** It transcribes correctly, but takes **~4.5 min per clip** instead of the expected ~13 s (bug 1) |
| Settings says where Hebrew runs | **Yes.** On the CPU it gives the wrong reason (bug 2) |

## Setup

* **Install.** 1.0.0 hybrid NSIS (`Murmur_1.0.0_x64-setup_hybrid.exe`, SHA-256
  `2f7f7b3a…c7de`, equal to `SHA256SUMS-windows-1.0.0.txt`) over 0.12.0 with `/S` (exit 0).
  * `flow_whisper_vulkan.dll` (60.8 MB) is now next to the exe.
  * Prefs were backed up first (`prefs.json.bak-20261102-080919-pre-1.0.0`). After the check
    they are back to that state (`hebrew_model` zipformer), with no key changed.
* **Model.** The 10-31 bench's `ivrit-turbo-q5_0.bin` was copied (not downloaded) to
  `models\ggml-ivrit-large-v3-turbo-q5_0.bin`. Its size and SHA-256 (`6c1da92e…a572`) match
  the catalogue entry.
* **Harness.** `plugin_bench.py` loads the **installed** `flow_whisper_vulkan.dll` with ctypes
  and calls the C ABI exactly as `core_adapter::whisper_engine` does: `threads` = the app
  default (4 on this 8-thread CPU), `fit_ctx: false`, `language: "he"`, empty prompt.
  * One resident engine, with one warm-up decode first. Latency = the wall time of
    `flow_whisper_transcribe`, so model load is excluded, as in the app.
  * Input: the 10-31 bench's 49 FLEURS he_il dev clips (3–20 s); the first 10 form the subset.
* **Silence gate.** 18 of the 49 FLEURS recordings are under the engine's silence gate
  (RMS < 0.003, `whisper.rs` `SILENCE_RMS`; −50 to −60 dBFS). For those the app returns ""
  at once (0 ms) without decoding. The harness scales them to RMS 0.05 and records the gain.
  This is a quirk of the test data, not a dictation bug, but a very quiet mic would hit the
  same gate.

## GPU (Vulkan, Intel Arc 140V)

| set | n | p50 | p90 | max | mean | WER |
|---|---|---|---|---|---|---|
| first 10 clips (incl. the 10.4 s clip) | 10 | 1296 ms | 2172 ms | 2213 ms | 1431 ms | 23.6 % |
| all 49 clips | 49 | **1412 ms** | **1883 ms** | 2144 ms | 1488 ms | **20.7 %** |
| clips of 8.5–11.5 s | 13 | 1501 ms | | 2010 ms | | |

* The 10.38 s clip took 1465 ms.
* WER 20.7 % matches the 10-31 bench (20.5 %), so the app's decode is as accurate as
  whisper-cli.
* **About 1.5× slower than the 10-31 whisper-cli Vulkan run** (mean 971 ms, p90 1144 ms;
  whisper.cpp b5454 with llama.cpp b12005's ggml).
  * Same clip, same 4 threads: 3.24 s clip 1154 vs 915 ms; 10.38 s clip 1465 vs 1181 ms.
  * Today whisper-cli took 1.06–1.37 s on the 10.38 s clip, with or without `-nt`, so
    `no_timestamps` (which the app doesn't set) makes no difference.
  * The likely cause is the older ggml Vulkan backend bundled in whisper-rs-sys 0.15.
* **First decode after install: ~12.4 s.** The first non-silent decode of a fresh install
  built the Vulkan pipelines. Later processes started warm (warm-up 1.15 s), so the driver
  caches them. The owner's first Hebrew dictation after an install or driver update will
  be slow once.
* **GPU confirmed:**
  * `flow_whisper_info` → `{"backend":"Vulkan","device":"Intel(R) Arc(TM) 140V GPU (16GB)","gpu_error":null}`
    and stderr has `ggml_vulkan: 0 = Intel(R) Arc(TM) 140V GPU … KHR_coopmat`.
  * The same DLL forced to the CPU is ~180× slower (below).
  * In the app, the process loads `flow_whisper_vulkan.dll`, `vulkan-1.dll` and
    `igvk64.dll`, with `hebrew_model = ivrit_turbo`.

## CPU fallback

* **Plugin CPU path** (`FLOW_WHISPER_DEVICE=cpu`): `info.backend = "CPU"`. Output is correct:
  * 3.24 s clip: **262 s**, 2/8 word errors;
  * 10.38 s clip: **267 s**, 3/22 word errors.
* **App, forced CPU.** The app started, ivrit loaded in 800 ms, and it shows "Ready" (label
  below).
* **App, no plugin.** This ran from a temp copy of the install without
  `flow_whisper_vulkan.dll`. The built-in CPU whisper loaded ivrit in 721 ms and shows
  "Ready", and no `[model]` problem line was printed. A dictation through it was not timed:
  there is no file input, and the built-in engine is linked into the exe.

### Bug 1: the plugin's CPU whisper is 6–18× slower than whisper.cpp's release build

| model, 10.38 s clip | whisper-cli b5454 CPU (OpenMP) | plugin, CPU |
|---|---|---|
| ivrit turbo q5_0, 4 threads | 15.1 s (encode 14.2 s) | **267 s** (18×) |
| ggml-base f16, 4 threads | 1.60 s | 10.6 s (6.7×) |
| ggml-base f16, 1 thread | 5.28 s | 33.3 s (6.3×) |

* Single-thread is just as slow, so this is per-thread compute, not threading.
* `-fa` changes nothing for whisper-cli (1.60 vs 1.61 s), so the app's flash_attn = 0 is not
  the cause.
* The DLL does contain AVX2 code (vpmaddubsw: 489), but it has few FMA / F16C instructions
  (vfmadd: 104, vcvtph2ps: 29) next to whisper's own `ggml-cpu-haswell.dll` (297 / 45).
* Suspects:
  * the MSVC build of ggml-cpu under `GGML_NATIVE=OFF`: FMA/F16C code paths not taken;
  * `GGML_OPENMP=OFF` (whisper-rs-sys without its `openmp` feature).
* **`flow-windows.exe` has the same instruction counts** (vfmadd 104, vcvtph2ps 29), and its
  whisper.cpp is built the same way. So the built-in CPU whisper is very likely just as
  slow. That covers the no-Vulkan fallback, and also the **default English whisper on every
  PC without a GPU**. Not measured.
* The CHANGELOG's "about 13 s" CPU figure is whisper-cli's number, not the app's.
* Next step: build whisper-rs-sys for Windows with clang-cl and/or `openmp`, then compare
  ggml-base on one clip.

### Bug 2: the CPU label gives the wrong reason

* With a GPU present but the CPU forced (`FLOW_WHISPER_DEVICE=cpu`), Settings says "Runs on
  the CPU (no graphics chip found)". It says the same when the plugin DLL is missing.
* The cause: `runsOnLine` (`ui/src/features/models.ts`) prints "no graphics chip found"
  whenever whisper runs on the CPU without a `gpu_error`. `ModelStatus.backend` does not say
  why the CPU was used: forced, plugin missing, or no GPU.
* "expect several seconds per dictation" understates bug 1 (minutes).

## Settings label (UI Automation text of Settings → Speech to text → Hebrew model)

* GPU: "Ready for Hebrew (loaded in 1300 ms). Runs on the graphics chip (Intel(R) Arc(TM)
  140V GPU (16GB), Vulkan)."
* Forced CPU / no plugin: "Ready for Hebrew (loaded in 800 ms). Runs on the CPU (no
  graphics chip found), so expect several seconds per dictation; Zipformer is faster."
* The screenshot attempt captured the lock screen (the laptop was locked), so the text
  above, read through UI Automation, is the evidence. No screenshot was kept.
