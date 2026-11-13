#!/usr/bin/env bash
# Every row of the findings doc, one model at a time on the Windows PC's
# GPU: start llama-server, warm it on five Hebrew lines (shader compiles,
# first prompt prefill; not scored), run the four suites, stop it.
#
#   batch.sh [ROW ...]     default: all rows below, in priority order
#
# env: PC (user@host), BIN (gpu-http-clean). An SSH tunnel must forward
# 127.0.0.1:18091 to the PC's 127.0.0.1:8091 (see README.md).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
# Plugin-like server: full-size SWA cache (llama.cpp's library default, as
# the plugin gets), no host-RAM prompt cache or context checkpoints (with
# them, every Gemma request re-processed its prompt tail: 4-14 s each).
X=--swa-full,--cache-ram,0,--ctx-checkpoints,0
# row  model file  drafting  suites
ROWS=(
  "gemma-3-12b google_gemma-3-12b-it-Q4_K_M.gguf ngram bench,numbers,he,ho"
  "llama-3.2-3b Llama-3.2-3B-Instruct-Q4_K_M.gguf ngram bench,numbers,he,ho"
  "qwen2.5-7b Qwen2.5-7B-Instruct-Q4_K_M.gguf ngram bench,numbers,he,ho"
  "qwen2.5-14b Qwen2.5-14B-Instruct-Q4_K_M.gguf ngram bench,numbers,he,ho"
  "qwen2.5-14b-nodraft Qwen2.5-14B-Instruct-Q4_K_M.gguf none he,ho"
  "gemma-3-12b-nodraft google_gemma-3-12b-it-Q4_K_M.gguf none bench,numbers,he,ho"
)
want=" ${*:-} "
for r in "${ROWS[@]}"; do
  read -r tag model spec suites <<<"$r"
  [ "$want" != "  " ] && [[ "$want" != *" $tag "* ]] && continue
  echo "== $tag $(date +%T)"
  "$here/serve.sh" start "$tag" "$model" "$spec" "$X" || { echo "no server for $tag"; continue; }
  "$here/run.sh" "warm-$tag" he5
  "$here/run.sh" "$tag" ${suites//,/ }
  "$here/serve.sh" stop "$tag"
  sleep 10
done
"$here/serve.sh" pull
echo "batch done $(date +%T)"
