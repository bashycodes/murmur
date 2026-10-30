#!/usr/bin/env bash
# Judge + score llm_win.py rows on Linux (dated probe, 2026-10-30).
# usage: score_pc.sh RESULTS_DIR   (holds v4c-*.jsonl fetched from the PC)
# Each file goes through ai-cleanup-bench --mode regard (product strict/loose guards + fallback
# rules + post-processing, as in the 2026-10-26 bench) into judged/, then score.py prints the
# category table (out_strict column) and latency_buckets.py the per-length latency.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
B="$here/../ai_cleanup_bench_2026_10_26"
bin="${BENCH_BIN:-$here/../../../../target/ai-bench/release/ai-cleanup-bench}"
d="$1"; mkdir -p "$d/judged"
args=()
for f in "$d"/v4c-*.jsonl; do
  n="$(basename "$f" .jsonl)"; lab="${n#v4c-}"; lab="${lab%-2026*}"
  "$bin" --mode regard --from "$f" --cases "$B/results/v4c_cases.jsonl" --session "$B/sessions/prod.json" > "$d/judged/$n.jsonl"
  args+=("$lab=$d/judged/$n.jsonl:out_strict")
done
python3 -I "$B/score.py" "${args[@]}"
python3 -I "$here/latency_buckets.py" "$d"/v4c-*.jsonl
