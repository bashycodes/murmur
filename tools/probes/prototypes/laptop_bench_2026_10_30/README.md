# Laptop model benchmark, 2026-10-30 (dated probe)

> **Status:** dated probe, research only. Findings:
> `docs/findings/2026-10-30-laptop-model-benchmark.md`.

Measures STT and AI-cleanup candidates on the owner's Windows PC with
official release binaries (llama.cpp b12005 cpu/vulkan, whisper.cpp b5454
cpu, sherpa-onnx v2.0.1 win-x64), at below-normal priority.

| File | What |
|---|---|
| `asr_win.py` | Runs on the PC. WER + per-clip compute ms for whisper-cli / sherpa-onnx-offline / sherpa-onnx (online). Needs `../../bench/wer.py` next to it |
| `he_manifest.json` | 49 FLEURS he_il dev clips (every 3rd unique sentence id; CC BY 4.0). Audio not committed: `google/fleurs` `data/he_il/audio/dev.tar.gz`, float32 → PCM16 |
| `llm_win.py` | Runs on the PC. llama-server.exe per model; prompt v4c rendered exactly as murmur-core does for a local backend; writes ai-cleanup-bench `--from` rows |
| `score_pc.sh` | Runs on Linux: `ai-cleanup-bench --mode regard` (product guards + fallback rules) → `../ai_cleanup_bench_2026_10_26/score.py` + `latency_buckets.py` |
| `results/` | Raw rows (`v4c-*.jsonl`, `asr-*.jsonl`), guard-judged rows (`judged/`), `score.txt` |

Reproduce (PC side, from a folder holding the scripts, `bin/`, `models/`, `audio/`):

```sh
python llm_win.py --bin bin --models models --prompt prompt_v4c.json --cases v4c_cases.jsonl \
  --outdir results/llm --run vulkan:Llama-3.2-3B-Instruct-Q4_K_M.gguf --run cpu:qwen2.5-1.5b-instruct-q4_k_m.gguf:4
python asr_win.py --bin bin --models models --audio audio --manifest asr_manifest_extended.json \
  --lang en --out results/asr-en.jsonl --run offline:sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8:4
```

Linux side: `BENCH_BIN=<ai-cleanup-bench> score_pc.sh <dir with v4c-*.jsonl>`.
