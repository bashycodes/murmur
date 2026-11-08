#!/usr/bin/env bash
# Start a local flow-server build, send every clip through it as a live
# session (the parity eval's `run_paths.py server`), stop it.
#
#   live.sh OUT_DIR TAG SPEED [extra run_paths.py args, e.g. --dict D.json]
#
# env: SERVER (flow-server binary), CONFIG (its config.toml: own port, own
# state dir and token file, same models as the home server), PORT,
# TOKEN (that config's token file; read by run_paths.py, never printed),
# RUN_PATHS (the parity eval's run_paths.py), CLIPS (WAV dir), SESSION,
# LD_LIBRARY_PATH (sherpa-onnx libs). Extra server env (FLOW_LLM_DRAFT=...)
# passes through. Output: OUT_DIR/server_TAG.jsonl, server log OUT_DIR/TAG.log.
set -uo pipefail
OUT=$1; TAG=$2; SPEED=$3; shift 3
: "${SERVER:?}" "${CONFIG:?}" "${PORT:?}" "${TOKEN:?}" "${RUN_PATHS:?}" "${CLIPS:?}" "${SESSION:?}"
mkdir -p "$OUT"
# Shared box: wait up to 30 min for 4000 MB available, else skip.
for _ in $(seq 1 90); do
  avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo)
  (( avail >= 4000 )) && break
  sleep 20
done
if (( avail < 4000 )); then echo "skip $TAG: $avail MB available" >&2; exit 1; fi
exec 9>"$HOME/.cache/flow-clone-build.lock"
flock 9
choom -n 1000 -- nice -n 10 "$SERVER" --config "$CONFIG" > "$OUT/$TAG.log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null; wait $pid 2>/dev/null' EXIT
# Ready = the LLM warmed (the pool logs it) or the process died.
for _ in $(seq 1 600); do
  grep -q "llm warm" "$OUT/$TAG.log" && break
  kill -0 $pid 2>/dev/null || { echo "server exited" >&2; exit 1; }
  sleep 1
done
python3 "$RUN_PATHS" server --clips "$CLIPS" --out "$OUT" --url "ws://127.0.0.1:$PORT" \
  --token-file "$TOKEN" --session "$SESSION" --tag "$TAG" --speed "$SPEED" "$@" 2>&1 | grep -v "^hello"
echo "done $TAG" >&2
