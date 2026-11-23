#!/usr/bin/env bash
# usage: run.sh SET MODEL TAG [OVERRIDE.json]
#   SET   ai (this probe) | suite | held | held2 | syn | main
#   MODEL 3b | 1.5b
# env: BIN (parity-clean), LIB (libmurmur_llm.so), MODELS (gguf dir),
#      SUITE (the 113 suite cases), MAIN (parity main-set cases, private),
#      MAIN_SESSION (session for the main set), OUT (output dir).
# Runs the cloud (AI cleanup) path under the shared build lock as
# a low-priority, earlyoom-first process, only with model size + 2000 MB free.
# SUFFIX=TEXT: a line after the transcript in every user message (--user-suffix).
# HOLDS_LOCK=1: the caller already holds the lock (several runs in one go).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
p="$here/.."
SET=$1; MODEL=$2; TAG=$3; OVR=${4:-}
case $MODEL in
  3b) G=$MODELS/qwen2.5-3b-instruct-q4_k_m.gguf; TO=30000 ;;
  1.5b) G=$MODELS/qwen2.5-1.5b-instruct-q4_k_m.gguf; TO=20000 ;;
esac
S=$p/ai_cleanup_bench_2026_10_26/sessions/prod.json
case $SET in
  suite) C=$SUITE ;;
  ai) C=$here/cases.jsonl ;;
  held) C=$p/newline_punct_2026_11_12/cases.jsonl ;;
  held2) C=$p/newline_punct_2026_11_12/cases2.jsonl ;;
  syn) C=$p/lists_corrections_2026_11_06/cases.jsonl; S=$MAIN_SESSION ;;
  main) C=$MAIN; S=$MAIN_SESSION ;;
esac
mkdir -p "$OUT"
o=$OUT/${SET}__${MODEL}__${TAG}.jsonl
[ -s "$o" ] && { echo "have $o" >&2; exit 0; }
need=$(( $(stat -c %s "$G") / 1048576 + 2000 ))
for _ in $(seq 120); do
  avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
  (( avail >= need )) && break
  echo "low mem: $avail < $need MB, waiting" >&2; sleep 30
done
(( avail >= need )) || exit 3
args=(--mode cloud --cases "$C" --session "$S" --llm "$G" --llm-lib "$LIB" --threads 4 --timeout-ms $TO)
[ -n "$OVR" ] && args+=(--override "$OVR")
[ -n "${DUMP:-}" ] && args+=(--dump-prompt "$DUMP")
[ -n "${SUFFIX:-}" ] && args+=(--user-suffix "$SUFFIX")
lock=(flock "$HOME/.cache/murmur-build.lock"); [ -n "${HOLDS_LOCK:-}" ] && lock=()
"${lock[@]}" choom -n 1000 -- nice -n 10 "$BIN" "${args[@]}" > "$o.tmp" && mv "$o.tmp" "$o"
echo "wrote $o" >&2
