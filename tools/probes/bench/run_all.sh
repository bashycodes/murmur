#!/usr/bin/env bash
# Full benchmark run: build -> fetch -> ASR -> LLM (4 and 8 threads) -> llama-bench throughput.
# Logs go to $CACHE/logs/<step>-<ts>.log; per-run JSONL to $CACHE/results/ (never overwritten).
# Usage: tools/probes/bench/run_all.sh [skip-setup]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CACHE="${CACHE:-$HOME/Claude/research/wispr/.cache}"
TS="$(date +%Y%m%d-%H%M%S)"; L="$CACHE/logs"; mkdir -p "$L"
PY="$CACHE/venv/bin/python"
if [[ "${1:-}" != skip-setup ]]; then
  "$HERE/build_engines.sh"
  "$HERE/fetch_models.sh" all
  [ -x "$PY" ] || { python3 -m venv "$CACHE/venv" && "$PY" -m pip install -q soundfile numpy; }
  "$PY" -I "$HERE/fetch_audio.py"
fi
python3 -I "$HERE/bench_asr.py" --threads 4,8 | tee "$L/asr-$TS.log"
python3 -I "$HERE/bench_llm.py" --threads 4 | tee "$L/llm-t4-$TS.log"
python3 -I "$HERE/bench_llm.py" --threads 8 | tee "$L/llm-t8-$TS.log"
for m in qwen2.5-0.5b-instruct-q4_k_m.gguf qwen2.5-1.5b-instruct-q4_k_m.gguf qwen2.5-3b-instruct-q4_k_m.gguf \
         Llama-3.2-1B-Instruct-Q4_K_M.gguf Llama-3.2-3B-Instruct-Q4_K_M.gguf; do
  "$CACHE/llama.cpp/build/bin/llama-bench" -m "$CACHE/models/$m" -t 4,8 -p 512 -n 64 -r 3 -o md
done | tee "$L/llama-bench-$TS.log"
uptime | tee -a "$L/llama-bench-$TS.log"
