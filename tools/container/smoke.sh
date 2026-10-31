#!/usr/bin/env bash
# Smoke test of the flow-server container through Docker Compose
# (docs/container.md): first start waits for a key, `keys add` through
# `docker compose exec`, a dictation over the real WebSocket protocol,
# then `down` + `up` and the same key still works.
#
#   tools/container/smoke.sh --models DIR --config FILE --wav FILE[:LANGS] [--port N] [--build]
#
#   --models DIR   host models folder (mounted read-only at /models)
#   --config FILE  config whose model paths point under /models
#   --wav FILE     16 kHz WAV to dictate (LANGS e.g. `en`; see the probe)
#   --port N       host port (default 17313; never the production server's)
#   --build        build the image first (FLOW_BUILD_SOURCE applies)
#
# Uses its own Compose project (flow-smoke) and removes it, with its state
# volume, at the end. The key's secret stays in a mode-0600 temp file.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PORT=17313
MODELS="" CONFIG="" WAV="" BUILD=0
die() { echo "smoke.sh: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
    case "$1" in
        --models) MODELS="$(readlink -f "${2:?}")"; shift ;;
        --config) CONFIG="$(readlink -f "${2:?}")"; shift ;;
        --wav) WAV="${2:?}"; shift ;;
        --port) PORT="${2:?}"; shift ;;
        --build) BUILD=1 ;;
        -h|--help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
    shift
done
if [ ! -d "$MODELS" ] || [ ! -f "$CONFIG" ] || [ -z "$WAV" ]; then
    die "--models, --config and --wav are required"
fi

export FLOW_MODELS_DIR="$MODELS" FLOW_CONFIG_FILE="$CONFIG" FLOW_HOST_PORT="$PORT"
C=(docker compose -p flow-smoke -f "$ROOT/deploy/docker-compose.yml")
TMP="$(mktemp -d)"
trap '"${C[@]}" down -v >/dev/null 2>&1 || true; rm -rf "$TMP"' EXIT

wait_log() { # wait_log PATTERN SECONDS
    local i
    for ((i = 0; i < $2; i++)); do
        "${C[@]}" logs 2>&1 | grep -q "$1" && return 0
        sleep 1
    done
    "${C[@]}" logs 2>&1 | tail -20 >&2
    die "timed out waiting for: $1"
}
dictate() {
    python3 "$ROOT/tools/probes/prototypes/server_e2e_2026_10_31.py" \
        --url "ws://127.0.0.1:$PORT" --token-file "$TMP/token" --wav "$WAV" \
        --llm-timeout-ms 20000 | grep '"wav"' | python3 -c '
import json, sys
r = json.loads(sys.stdin.read())
print("  %s lang=%s commit->result=%s ms: %s" % (r["status"], r["language"], r["commit_to_result_ms"], r["final_text"][:80]))
sys.exit(0 if r["final_text"] else 1)'
}

[ "$BUILD" = 1 ] && "${C[@]}" build
"${C[@]}" down -v >/dev/null 2>&1 || true
echo "==> first start (no keys)"
"${C[@]}" up -d
wait_log "waiting for keys.json" 30
echo "==> keys add via docker compose exec"
(umask 077; "${C[@]}" exec -T flow-server flow-server keys add smoke --admin >"$TMP/token")
wait_log 'flow_server: ready' 600
echo "==> dictation"
dictate
echo "==> down + up: the key persists"
"${C[@]}" down
"${C[@]}" up -d
wait_log 'flow_server: ready' 600
"${C[@]}" logs 2>&1 | grep -q "waiting for keys.json" && die "keys lost across restart"
dictate
echo "==> health: $(docker inspect flow-smoke-flow-server-1 --format '{{.State.Health.Status}}')"
echo "smoke OK"
