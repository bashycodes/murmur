#!/usr/bin/env bash
# Dated probe (2026-10-23) for docs/proposals/streaming-asr-latency.md §8:
# release→text before vs after feat/streaming-asr, local Parakeet and the
# server route, real-time push (hybrid_latency example), long clips.
# Not a permanent tool; the consumer is the proposal's §8 table.
#
#   BASE=<worktree at the base commit, release-built>  NEW=<this worktree, release-built>
#   PARAKEET_DIR=… CLIPS_DIR=<dir with long_0NNs.wav from the 2026-10-23 segmented probe>
#   SHERPA_ONNX_LIB_DIR=…  tools/probes/prototypes/streaming_latency_2026_10_23.sh
#
# Each run gets its own output dir (never overwritten). Test servers run on
# 127.0.0.1:7391/7392 with a throwaway token, never the user's service port.
set -euo pipefail
: "${BASE:?}" "${NEW:?}" "${PARAKEET_DIR:?}" "${CLIPS_DIR:?}"
OUT=${OUT:-$HOME/Claude/research/wispr/.cache/streaming-latency-2026-10-23/$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT"
export LD_LIBRARY_PATH=${SHERPA_ONNX_LIB_DIR:-}:${LD_LIBRARY_PATH:-}
CLIPS=${CLIPS:-"long_010s long_025s long_028s long_032s long_040s long_060s"}
head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' > "$OUT/token"
MODEL="{\"asr_engine\":\"parakeet\",\"asr_path\":\"$PARAKEET_DIR\",\"threads\":4}"

run() { # label tree port session-json
  local label=$1 tree=$2 port=$3 sess=$4
  cat > "$OUT/$label.toml" <<TOML
bind = ["127.0.0.1:$port"]
token_file = "$OUT/token"
workers = 1
[model]
asr_engine = "parakeet"
asr_path = "$PARAKEET_DIR"
threads = 4
TOML
  "$tree/target/release/flow-server" --config "$OUT/$label.toml" > "$OUT/$label.server.log" 2>&1 &
  local pid=$!
  for _ in $(seq 120); do grep -q ready "$OUT/$label.server.log" && break; sleep 1; done
  local files=()
  for c in $CLIPS; do files+=("$CLIPS_DIR/$c.wav"); done
  "$tree/target/release/examples/hybrid_latency" --server "127.0.0.1:$port" \
      --token-file "$OUT/token" --model "$MODEL" --session "$sess" "${files[@]}" \
      > "$OUT/$label.tsv" 2> "$OUT/$label.err" || true
  kill "$pid"; wait "$pid" 2>/dev/null || true
  echo "== $label ($sess)"; cat "$OUT/$label.tsv" | cut -f1-7
}

run before "$BASE" 7391 '{"partial_interval_ms":1500}'
run after_p0 "$NEW" 7392 '{"partial_interval_ms":1500}'
run after_d22 "$NEW" 7392 '{"partial_interval_ms":1500,"segment_pause_ms":800}'
echo "out: $OUT"
