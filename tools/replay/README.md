# Replay harness (beta debug tool)

Replaces manual A/B dictation rounds: the owner's saved recordings are re-run
on this box through every speech model x cleanup model x prompt version, on the
**real** flow-core pipeline (`FlowCore::commit`: decode-while-speaking
segmentation, VAD, hallucination guard, D30 AI-cleanup routing, loosened and
strict guards, fallback rules, post-processing), and scored.

* Binary: `crates/flow-replay` (`flow-replay --help`). Library + fake-engine
  test of the matrix: `crates/flow-replay/tests/matrix.rs`.
* Score report: `score.py`, reusing
  `tools/probes/prototypes/ai_cleanup_bench_2026_10_26/score.py`'s
  closeness and exact match. Test: `python3 -I tools/replay/test_score.py`.

## Privacy

Recordings and anything derived from them (transcripts, outputs, expected
text) stay **out of the repo**: the corpus lives in a temp dir (pulled from
the phone), and results go next to it (`run.sh` refuses an `--out` inside the
repo). No expected text is committed.

The OpenRouter key is read by the binary from
`$HOME/.config/flow-clone/openrouter.key` (or `--key-file`) only when an
OpenRouter slot is asked for; it goes only into the request header and is
never printed, logged or written. Rows carry costs and generation metadata,
no request bodies.

## Corpus

A directory with `audio/*.wav` (16 kHz mono) and the phone's
`dictations.jsonl` debug log (rows whose `audio` file exists are replayed;
the row's `target` app gives the field context, its `raw`/`text`/route
fields are the phone's result). Without a log every WAV is replayed with no
phone comparison.

Expected text: `CORPUS/expected.json`, `{recording id (WAV stem): text}`
(keys starting with `_` are comments), owner-editable. Recordings missing
from it fall back to the `expected` of the reference pair (`--pairs FILE`,
JSONL `{id, spoken, expected}`) whose script matches the phone's transcript
(closeness >= 0.6).

## Run

```sh
export SHERPA_ONNX_LIB_DIR=<dir with libsherpa-onnx-c-api.so>   # build time
export FLOW_PARAKEET_DIR=<parakeet model dir>   # e.g. the server's unified-en-0.6b-int8
export FLOW_WHISPER_MODEL=<ggml-base.en.bin>
export FLOW_LLM_GGUF=<qwen2.5-1.5b-instruct-q4_k_m.gguf> FLOW_LLM_LIB=<libflow_llm.so>

tools/replay/run.sh --in "$CORPUS" \
  --asr parakeet,whisper,or-stt \
  --cleanup none,local,or:meta-llama/llama-3.3-70b-instruct \
  --prompt v5,v4c --llm "$FLOW_LLM_GGUF" --max-spend 0.05
tools/replay/run.sh --score "$CORPUS/replay/<run>" --expected "$CORPUS/expected.json" --detail
```

| Option | Values |
|---|---|
| `--asr` | `parakeet[=DIR]`, `whisper[=GGML]` (host stand-in for the phone's whisper), `or-stt[=MODEL]` (OpenRouter cloud STT, the product's `CloudAsr`; default `openai/whisper-large-v3-turbo`) |
| `--cleanup` | `none` (fallback rules), `local` (today's on-device pipeline: rules, Qwen prompt v2, strict guard, skip-clean), `or:MODEL` (OpenRouter AI cleanup, D30; with `--llm` it falls back to the local pipeline exactly as the phone does, row label `or:MODEL+local`; `--no-local-fallback` for the rules) |
| `--prompt` | versions for the `or:` slots: `v5` (what the product sends, the default), `v4c` (its predecessor), `v2`. The local pipeline is always v2 (the product runs v5 only on the cloud path) |
| `--max-spend` | OpenRouter cap for the run, USD (default 0.05); later paid calls become `spend_cap` rows |
| `--only` | recording ids (substring) for smoke runs |
| `--session` | a `SessionConfig` JSON instead of the phone's (v0.6.0 Android: light, casual, formal email, 3 s LLM timeout, 800 ms segments, VAD on) |

Each speech model transcribes a recording once (that commit, without a
cleaner, is the `none` row); the cleanup combinations replay that transcript
with the same audio pushed. Output per run in `CORPUS/replay/<UTC time>/`:
`rows.jsonl`, `recordings.jsonl`, `run.json` (matrix, spend incl. the key's
usage delta), `score.md`.

The report has one row per combination plus `phone (debug log)`: closeness /
exact vs expected, closeness / exact **vs the phone** (reproduction), guard
fallbacks (AI cleanup answer not used, by reason), cleanup routes, LLM and ASR
p50, cloud cost. The total OpenRouter spend is printed at the end.

## Memory rules (shared box with the home flow-server)

* Every cargo build goes through `tools/replay/run.sh cargo <args>` (or the
  build `run.sh` does itself): `--release -j 2`, serial under
  `$HOME/.cache/flow-clone-build.lock`, `choom -n 1000` (earlyoom kills it
  first), `nice -n 10`, and only with >= 4000 MB available.
* A run that loads the local LLM (`--cleanup local` or `--llm`) runs under
  the same lock (one local model at a time, serial with builds) and only
  with model size + 2000 MB available. Runs without a local model are only
  niced.
* The home flow-server is never touched; models are read from its models dir.

Not covered: on-phone replay (WAV-as-mic), the user dictionary (not in a
corpus), and the local 1.5B on prompt v5 (no product path runs it).

The cache default is `~/.cache/flow-clone`; set CACHE=<dir> to reuse an existing cache.
