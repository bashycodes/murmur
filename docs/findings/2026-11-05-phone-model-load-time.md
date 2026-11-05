# 2026-11-05: why model loads are slow on the phone (ticket "Load 3")

**Question.** Why does zipformer-he (69 MB int8, sherpa-onnx online transducer) take
16.6 s to load on the S25 Edge (`2026-10-31-phone-on-device-bench.md`) when a laptop
loads it in 3–9 s? And why does a language switch on the phone wait for the AI cleanup
model too?

**Answer.**

* **Half of the zipformer load was a wasted encoder session.** With no `model_type` in its
  config, sherpa-onnx's online transducer reads the type from the encoder's ONNX metadata by
  creating a whole onnxruntime session for the encoder and throwing it away, then creates the
  real one. The app builds two recognizers (beam search for hotwords, greedy for the rest), so
  a load created **four** encoder sessions, one after the other. Parakeet sets
  `model_type = nemo_transducer`, so it never paid this, which is why Parakeet-he (about 9×
  bigger) loaded faster on the phone (4.7 s) than the zipformer (16.6 s).
* **Fixed** (`flow-asr-sherpa`): the app reads `model_type` from the encoder's metadata
  itself (top-level protobuf fields only; the graph is skipped by its length, so it reads a
  few hundred bytes) and passes it to sherpa; and the beam and greedy recognizers are created
  on two threads at once. On the i7-6700: **6.7 s → 2.2 s**, same text. The phone is not
  measured yet; expected roughly 16.6 s → 5–6 s (§ Phone measurement). Parakeet can do the
  same (`ParakeetConfig::parallel_sessions`) but keeps the serial load by default: two of its
  much bigger encoder sessions initialising at once raise peak RAM, unmeasured on the phone.
* **Not the cause:** ONNX Runtime's graph-optimisation level (the encoder takes about as long
  to set up with optimisation off), the thread count (session setup is single-threaded: 1, 2,
  4 and 8 threads load in the same time), `tokens.txt`, and the hotword vocabulary
  (`bpe.vocab` is derived from `tokens.txt` in < 10 ms, once at load; hotwords themselves are
  per decode). There is no first-run cache in sherpa or the app: every load does the same work.
* **The LLM is now kept across a language switch.** With "Keep both languages loaded" off
  (D54), a switch to or from Hebrew frees the engine and loads the other speech model; the
  cleanup LLM (Qwen2.5-1.5B, 38.1 s on the phone, of which 34.4 s is the static prompt
  prefill) was loaded again with it. The phone now keeps the loaded LLM and the next load
  reuses it (`FlowCore::keep_llm`, `nativeKeepLlm(h)`: the LLM of the engine being freed,
  not whichever loaded last; a load whose speech model fails puts it back for the retry).
* **Every load logs its timings** per model and phase (§ What the log line says).

## Measurements (i7-6700, 8 threads shared, load avg 2.5–7, `nice`)

Model: `zipformer2-66m-streaming-he-int8` (encoder 70.5 MB, decoder 1.0 MB, joiner 0.8 MB),
4 threads, files in the page cache, three runs each. Tool:
`cargo run --release -p flow-asr-sherpa --example online_check -- MODEL_DIR [--serial] [--greedy]`.

| build | beam + greedy | greedy only |
|---|---|---|
| before: no `model_type`, serial | 6.7 s (each recognizer 3.3 s) | 3.2–3.3 s |
| `model_type`, serial (`--serial`) | 3.4–3.6 s (each 1.7 s) | 1.6–1.7 s |
| `model_type`, parallel (the new default) | **2.1–2.3 s** | 1.6–1.7 s |

* `strace` of a greedy-only load showed `encoder.int8.onnx` opened twice, 1.65 s apart, before
  the decoder and joiner; with `model_type` set it is opened once.
* Created in parallel, each recognizer takes ~2.1 s instead of 1.7 s (memory bandwidth and
  the allocator are shared), so the wall time is 65 % of the serial time, not 50 %.
* ONNX Runtime 1.26 (Python, `tools/probes/prototypes/ort_session_levels.py`), encoder
  session setup per optimisation level: disabled 1.55 s, basic 1.73 s, extended 1.66 s,
  all 1.70 s. Decoder and joiner: 1–3 ms. So the cost is building the graph (a zipformer2
  encoder has thousands of nodes), not optimising it. Loading an ORT-optimised copy of the
  encoder took 0.73 s, but sherpa-onnx's C API cannot load one or set the level, and an
  optimised model is specific to the CPU it was made on; not pursued.
* Parakeet-he (beam + greedy, parallel): 8.7 s here (the pins doc measured 16.7 s serial).
  Off by default (peak RAM, above).
* Decode threads are unchanged: beam and greedy each had their own `threads`-sized
  onnxruntime pools before, and only one decodes at a time; the parallel load overlaps only
  their setup.
* Decoding is unchanged: the Hebrew FLEURS clip gave the same text greedy and beam
  (`cargo test -p flow-asr-sherpa --test online` with `FLOW_ZIPFORMER_HE_DIR` + `FLOW_HE_WAV`).

## Why the phone was slower than the laptop

The phone's 16.6 s is four encoder sessions at about 4 s each; the laptop's 6.7 s is four at
1.65 s. That ratio (2.5×) is plausible for building a large int8 graph on one core of a
phone (onnxruntime's ARM64 int8 kernels pre-pack weights at session setup) but is **not
measured**. Two phone-only suspects remain, and the new log line checks both:

* the loading thread may run on the little cores: the app loads on a background coroutine,
  and Android may put a foreground-service process in a cpuset without the big cores;
* storage: the first load after install or a reboot reads 70 MB from flash.

## What the log line says

With the debug log on (Settings → Debug log; nothing is logged or read otherwise), one debug
log event `engine_load_timings` per native load, which the debug log also writes to logcat
(tag `FlowDebug`):

```json
{"total_ms":5400,"models":[
  {"kind":"asr","language":"he","backend":"sherpa_online","ms":5300,"phases":[
    {"name":"files","ms":1},{"name":"model_type","ms":0},{"name":"vocab","ms":3},
    {"name":"beam_sessions","ms":5200},{"name":"greedy_sessions","ms":5250},{"name":"sessions","ms":5260}]},
  {"kind":"llm","backend":"llama","ms":0,"reused":true}],
 "load_set":"Language(code=he)","threads":4,"cpus":"0-7"}
```

(numbers invented). `beam_sessions` / `greedy_sessions` are each recognizer's own time on its
own thread, `sessions` the wall time of both. The LLM has `model` (file load) and `warm`
(static prompt prefill) when it loads, `"reused":true` with 0 ms when kept. `cpus` is the
loading thread's `Cpus_allowed_list`.

## Phone measurement (needs a native build: the fix is in the Rust libraries)

1. Hybrid debug APK built with `tools/build-native.sh --flavor hybrid` (new JNI symbols
   `nativeLastLoadTimings`, `nativeKeepLlm`, `nativeReleaseKeptLlm`; an older library just logs nothing and keeps
   nothing). Debug log on. Keep both languages **off**, AI cleanup on, Hebrew model =
   zipformer-he.
2. Dictate once with the English keyboard (cold load), then switch the keyboard to Hebrew and
   dictate, then back to English and dictate. Read the three `engine_load_timings` lines
   (Settings → Debug log, or logcat tag `FlowDebug`).
3. Check:
   * zipformer `ms` ≪ 16.6 s (expected ~5–6 s) and `beam_sessions` ≈ `greedy_sessions` ≈
     `sessions` (they ran at once);
   * the second and third lines have the LLM `"reused":true`, 0 ms (before: ~38 s);
   * `cpus`: `0-7` (all cores) or a subset (e.g. `0-5`, little + middle cores only). A
     subset means the slow part is the scheduler, not onnxruntime;
   * the first line's LLM `warm` vs `model`: if `warm` is still ~34 s, the prefill is the next
     thing to move off the load path.

## Not done

* The LLM's static-prompt prefill (34 s of the 38 s phone load) still runs inside the first
  load. Running it after the engine is ready would make the first load ~4 s; it needs the
  plugin to accept a deferred warm-up and is a separate change.
* Creating the greedy recognizer lazily (on the first decode without hotwords) would cut the
  zipformer load to one session, but moves ~1.7 s (laptop) onto the first dictation; the
  parallel load gets most of the gain without that.
