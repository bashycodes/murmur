# GPU cleanup latency and memory: Gemma 3 12B, Qwen2.5-7B / 14B, Llama 3.2 3B (2026-11-14)

The 2026-11-12 GPU cleanup pick (Gemma 3 12B Q4_K_M,
`docs/model-benchmarks.md` §GPU cleanup model) had quality measured on a CPU
only. Its GPU latency and its 9–10 GB memory need were estimates. This run
measures them on a real GPU, scores Llama 3.2 3B (the 6–10 GB fallback) on
the same suites, and runs the Qwen2.5-14B drafting-off control.

Probe: `tools/probes/prototypes/gpu_bench_2026_11_14/` (README there).

## Setup

* **GPU: Intel Arc 140V**, the integrated GPU of an Intel Core Ultra 9 288V
  (Lunar Lake) laptop: the Windows PC. Windows 11, Intel graphics driver
  32.0.101.8860, 32 GB LPDDR5X. **It has no dedicated VRAM.** The GPU uses
  shared system memory (the driver offers it about 18 GB), so "GPU memory"
  below is system RAM the GPU holds. The PC was on mains power, in the
  Balanced power plan, with the owner's usual apps open.
* **llama.cpp b12368**, the official prebuilt
  `llama-b12368-bin-win-vulkan-x64.zip` (SHA-256 `49a9ad97…bbca39e`),
  `llama-server` with every layer on the GPU (`-ngl 99`).
* **Server settings as the murmur-llm plugin uses them**: 4096 context, batch
  and micro-batch 128, one slot, greedy, the GGUF's chat template through
  llama.cpp's legacy (non-Jinja) path, at most 2 × transcript tokens + 32 new
  tokens, the plugin's stop strings, the shared prompt prefix kept in the KV
  cache between dictations. Prompt-lookup drafting at 4 tokens per step
  (`--spec-type ngram-simple`, lookup 3, draft 4), as murmur-server runs it
  (`llm_draft`, D69). It is llama.cpp's own n-gram drafting, not the
  plugin's code.
* **Models**: the GGUFs of the 2026-11-12 run (same pinned files, copied),
  and Llama 3.2 3B Instruct Q4_K_M at the Windows catalogue pin (SHA-256
  checked, `6c1a2b41…c728ff`).
* **Suites and scoring: unchanged from 2026-11-12.** The core runs on the
  Linux box: `MurmurCore` in `server` mode, prompt v2, the rules pass, the
  guards and the D65 script check. Only the model call goes to the PC's
  `llama-server`, through an SSH tunnel. The suites are the 19 bench cases,
  61 numbers cases, and 28 + 14 Hebrew lines (122 dictations), all synthetic.
  `score.py` of 2026-11-12 scores them.
* **Latency** is llama-server's own time per dictation (prefill + decode).
  That is what a murmur-server would spend, without the network. The core's
  `llm_ms` on the Linux box adds the tunnel: 0.03–0.08 s more at p50.
  Each model first ran a 5-line warm-up, which is not scored (see "First
  dictation").
* **Memory** was sampled once a second (in practice about every 2.5 s) from
  the Windows performance counters while the server ran: "GPU Process
  Memory" (dedicated and shared, for the llama-server process), "GPU Adapter
  Memory" (the whole GPU) and the process's working set and private bytes.
  The tables give the peaks.

### Two server flags this llama.cpp needs (measured)

With the default settings, every dictation after the first re-processed
part of its prompt: 4–14 s per Gemma request, even when only 1 token was
new. The cause is llama-server's host-RAM prompt cache and its SWA context
checkpoints (Gemma 3 uses sliding-window attention). `--swa-full --cache-ram
0 --ctx-checkpoints 0` removed it: 1 new token then took 0.10 s instead of
3.8 s, and 32–35 new tokens took 0.4–0.65 s instead of 7.8–14 s
(`prefill_probe.py`). A full-size SWA cache is also what the plugin gets
from llama.cpp's library defaults. Every number below uses these flags.

## Results

### Latency and memory (measured on the Arc 140V iGPU)

Per dictation, over the 122 dictations; EN = bench + numbers (80), HE = the
Hebrew lines (42, which are longer).

| model | p50 | p95 | max | EN p50 / p95 | HE p50 / p95 | decode tok/s | GPU memory (process, shared) | process private bytes |
|---|---|---|---|---|---|---|---|---|
| Llama 3.2 3B | 0.46 s | 2.15 s | 4.2 s | 0.39 / 0.53 s | 1.86 / 2.65 s | 66 | 2.87 GB | 3.07 GB |
| Qwen2.5-7B | 0.85 s | 1.76 s | 2.3 s | 0.68 / 1.03 s | 1.41 / 1.86 s | 37 | 5.00 GB | 5.18 GB |
| **Gemma 3 12B** | **1.54 s** | **3.51 s** | 4.0 s | 1.29 / 2.13 s | 3.04 / 3.73 s | 20 | **9.84 GB** | 10.08 GB |
| Gemma 3 12B, drafting off | 2.10 s | 7.89 s | 8.5 s | 1.64 / 3.36 s | 7.20 / 8.14 s | 10 | 9.83 GB | 10.07 GB |
| Qwen2.5-14B | 1.73 s | 3.55 s | 12.4 s | 1.39 / 2.13 s | 2.91 / 4.31 s | 19 | 9.89 GB | 10.13 GB |
| Qwen2.5-14B, drafting off (HE only) | – | – | 16.4 s | – | 6.20 / 7.25 s | 8 | 9.89 GB | 10.13 GB |

* No dictation took more than 60 s; the slowest was 16.4 s (a 14B answer
  that ran into Chinese text until its token limit).
* "decode tok/s" is generated tokens per second of decode time, with the
  drafted tokens counted. Drafting accepted 74–79 % of the drafted tokens
  and roughly halved Hebrew latency (Gemma HE p50 7.2 → 3.0 s). It did not
  change Gemma's answers (all 122 equal).
* **GPU memory**: the dedicated column was 0 for every model; all of it is
  shared. The whole GPU's shared usage rose by the same amount over its
  baseline (about 2.1 GB for the desktop): to 11.9–12.0 GB with Gemma or
  the 14B loaded. With Gemma or the 14B loaded, the PC's free memory fell
  to 1.0–1.3 GB of 32 GB, with the owner's apps open.
* **Load** (from the SSD, file possibly in the OS cache): Llama 5 s, 7B 9 s,
  14B 14–16 s, Gemma 35–53 s.
* **First dictation** after a load, which prefills the about 1.9k-token
  prompt prefix and compiles the GPU shaders: Llama 3.1 s, 7B 5.1 s, Gemma
  7.8–8.7 s, 14B 9.1–9.5 s. murmur-server prefills the prefix at load, so
  users do not see this.

### Quality (same suites and scorer as 2026-11-12)

| model | bench exact | bench sim | numbers pass | HE stays Hebrew (D65) | HE words kept | HE junk (final text) | HE reworded | HE punctuated | guard fallbacks |
|---|---|---|---|---|---|---|---|---|---|
| Llama 3.2 3B | **14/19** | **0.968** | **53/61** | 42/42 | 0.97 | 0 | **22** | 38/42 | 0/122 |
| Qwen2.5-7B | 14/19 | 0.956 | 58/61 | 42/42 | 0.99 | 0 | 11 | 22/42 | 2/122 |
| Gemma 3 12B | 11/19 | 0.953 | 58/61 | 42/42 | 0.99 | 0 | 8 | 32/42 | 0/122 |
| Qwen2.5-14B | 12/19 | 0.959 | 60/61 | 42/42 | 0.98 | 0 | 12 | 23/42 | 6/122 |

**The GPU gives the same texts as the CPU.** The 2026-11-12 run assumed
this; now it is checked. Gemma's final text equals the CPU run's in 120 of
122 dictations (the other two differ in one apostrophe and one comma).
Qwen2.5-7B: 112/122. 8 English answers lost their first capital and full
stop, and 2 Hebrew answers are the junk below. Qwen2.5-14B: 108/122.

The junk column is now 0 for every model, but not because the models
improved. The D65 guard has rejected foreign words added to a Hebrew
cleanup since 2026-11-13, after the CPU run. Those dictations now fall back
to the rules-only text ("guard fallbacks"). The model answers still hold the
same junk: Qwen2.5-7B wrote an Arabic word and an English sign-off
(`[Your Name]`), as on the CPU.

**Llama 3.2 3B, hand check:**

* English: the best bench score of all (14/19, 0.968).
* Numbers: 8 misses. 3 are harmful digit changes in spoken digit runs (a gate
  code 4816 written as 4616, a booking reference 52091 as 5091, an
  extension 337 as 3-37). It also wrote "Theres" and "Ill" (apostrophes
  dropped).
* Hebrew: 22 reworded lines. Most only drop a leading "ו" where the model
  split a sentence, which is harmless. 6 are harmful, and the D65 guard
  passes them all, because they are plain Hebrew:
  * a subject changed (he asked → I asked)
  * a clause dropped together with a "not", which flips the meaning
  * a phrase garbled into non-words
  * a misspelled word
  * two phrases dropped

  A seventh line repeats a word. Gemma had none of these (8 minor
  rewordings, 2026-11-12).

### The Qwen2.5-14B drafting control

Does prompt-lookup drafting cause the 14B's mixed-script Hebrew? **No.**
The table counts junk in the model's answer (before the core's guards),
using `score.py`'s junk rule plus Hangul.

| run | Hebrew lines | answers with junk |
|---|---|---|
| CPU, plugin, drafting 4 (2026-11-12) | 42 | 7 |
| GPU, llama-server, drafting 4 | 42 | 6 |
| **GPU, llama-server, drafting off** | 42 | **5** |
| CPU, plugin, drafting off (the 5 control lines) | 5 | 4 |
| CPU, plugin, drafting 4 (the same 5 lines, rerun) | 5 | 4 |

* With drafting off, the 14B still writes Chinese instructions, Cyrillic,
  Arabic letters and "ToUpdate" into Hebrew lines. This happens both
  through the real plugin on the CPU and through llama-server on the GPU.
  The same 4–5 lines are junk with drafting on or off.
* Greedy drafting should not change the answer. In practice 5 of 42 GPU
  answers differed between drafting on and off. One extra junk line (Korean
  words) appeared only with drafting on, and one junk line of the
  2026-11-12 run did not come back in the rerun. Batched verification
  rounds slightly differently, and it tips answers the model was already
  unsure about. Both runs are equally unreliable in Hebrew.
* So the junk comes from the model. "Do not use Qwen2.5-14B for Hebrew"
  stands. Since 2026-11-13 the D65 guard catches all of its junk answers
  (6/6 fallbacks), so the user gets the uncleaned text, not the junk.

## What this changes

* **Gemma 3 12B on a GPU: measured.** It needs **9.8 GB of GPU memory** at
  the plugin's 4096 context (estimate: 9–10 GB). That confirms "more than
  10 GB" (a 12 GB card). An 8 or 10 GB card cannot hold it with a
  desktop and driver beside it.
* **Latency on this iGPU**: p50 1.5 s, p95 3.5 s per dictation (Hebrew p50
  3.0 s). The 0.2–0.8 s estimate was for an RTX 3060-class card, which has
  about 2.5× the memory bandwidth of this iGPU (360 vs about 136 GB/s), and
  decoding is bandwidth-bound. So that estimate is plausible but still
  unmeasured. On this iGPU it is 2–4× slower than the estimate, and still
  4× faster than the CPU run (6.1 s p50 on the i7-6700).
* **Memory needs, measured: Qwen2.5-7B 5.0 GB, Llama 3.2 3B 2.9 GB.** Both
  fit a 6–8 GB card. Qwen2.5-14B needs 9.9 GB, the same as Gemma.
* **Llama 3.2 3B as the 6–10 GB fallback**: good for English, but not
  for Hebrew or spoken digit runs. It makes harmful edits there that no
  guard catches. On this data, Qwen2.5-7B is the better fallback when it
  fits. The 2026-11-12 hand check found 2 harmful Hebrew edits that pass
  the guard (a pronoun and a misspelling; Llama has 6), and its 2 junk
  answers now fall back. It is also faster in Hebrew. Qwen2.5-7B is not in
  the catalogue, so this is a note in the docs, not a pick change.
* **No decision row**: the GPU pick (Gemma 3 12B for > 10 GB) is unchanged.

## Not measured

* An NVIDIA (CUDA) card, and murmur-server's own CUDA path. This is
  llama-server on Vulkan, an iGPU.
* Gemma partly offloaded to the CPU on an 8–10 GB card. On an iGPU, CPU and
  GPU share one memory, so this PC cannot show it.
* Qwen2.5-1.5B (the server default) on the GPU. Its file was not on the PC,
  and it was not needed for the acceptance.
* The plugin's own drafting and prefill-ahead on the GPU. llama-server's
  n-gram drafting stands in for the first; the second would lower latency
  further.
