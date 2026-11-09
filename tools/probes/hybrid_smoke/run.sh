#!/usr/bin/env bash
# Hybrid end-to-end smoke test + latency: a real murmur-server (release,
# Parakeet) in the foreground on 127.0.0.1, then the same WAVs dictated
# locally and through Router → murmur-remote → server
# (crates/murmur-contract-tests/examples/hybrid_latency.rs). The server is
# stopped at the end. Nothing is installed; state lives in target/hybrid-smoke.
#
# Env: PARAKEET_DIR (model dir), AUDIO_DIR (16 kHz WAVs), N (files, default 10),
#      PORT (default 17313), plus the usual BINDGEN/SHERPA build env.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
PARAKEET_DIR="${PARAKEET_DIR:?model dir}"
AUDIO_DIR="${AUDIO_DIR:?wav dir}"
N="${N:-10}"
PORT="${PORT:-17313}"
S="$ROOT/target/hybrid-smoke"
# Parakeet builds link libsherpa-onnx-c-api / libonnxruntime dynamically.
export LD_LIBRARY_PATH="${SHERPA_ONNX_LIB_DIR:?sherpa-onnx lib dir}:${LD_LIBRARY_PATH:-}"
rm -rf "$S"; mkdir -p "$S/state"

cargo build --release -p murmur-server --no-default-features --features parakeet,tls
cargo build --release -p murmur-contract-tests --features parakeet --example hybrid_latency

target/release/murmur-server gen-token --out "$S/token"
cat > "$S/config.toml" <<CONF
bind = ["127.0.0.1:$PORT"]
token_file = "$S/token"
workers = 1
[model]
asr_engine = "parakeet"
asr_path = "$PARAKEET_DIR"
threads = 4
[log]
level = "info"
CONF

(cd "$S/state" && HOME="$S/state" TMPDIR="$S/state" exec "$ROOT/target/release/murmur-server" --config "$S/config.toml") 2> "$S/server.log" &
SRV=$!
trap 'kill -TERM $SRV 2>/dev/null; wait $SRV 2>/dev/null || true' EXIT
for _ in $(seq 120); do grep -q " ready" "$S/server.log" && break; sleep 0.5; done
grep " ready" "$S/server.log" || { cat "$S/server.log"; exit 1; }

FILES=$(ls "$AUDIO_DIR"/*.wav | head -n "$N")
target/release/examples/hybrid_latency --server "127.0.0.1:$PORT" --token-file "$S/token" \
    --model "{\"asr_engine\":\"parakeet\",\"asr_path\":\"$PARAKEET_DIR\",\"threads\":4}" $FILES \
    | tee "$S/latency.tsv"
echo "--- server log (no transcripts expected) ---"
grep -c "result" "$S/server.log" || true
echo "files left in state dir: $(find "$S/state" -mindepth 1 | wc -l)"
