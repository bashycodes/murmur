#!/usr/bin/env bash
# Smoke test of the flow-server image's quick start (docs/container.md,
# D77): the documented `docker run` with a fresh volume, the admin key in
# the log, a dictation over the real WebSocket protocol with that key, a
# second key through `docker exec … flow-server keys add`, then a restart
# (key not shown again, same key still works).
#
#   tools/container/smoke.sh --image IMAGE --wav FILE[:LANGS] [--models LIST]
#                            [--seed DIR] [--port N]
#
#   --image IMAGE  image to test (e.g. flow-server:local)
#   --wav FILE     16 kHz WAV to dictate (LANGS e.g. `en`; see the probe)
#   --models LIST  FLOW_MODELS (default: default)
#   --seed DIR     copy this folder's model folders into the volume's
#                  library/ first (folder name = model id, files as in
#                  docs/air-gapped-models.txt), so nothing is downloaded
#   --port N       host port (default 17313; never the production server's)
#
# Uses its own container (flow-smoke) and volume (flow-smoke-data) and
# removes both at the end. The key stays in a mode-0600 temp file.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PORT=17313 IMAGE="" WAV="" MODELS=default SEED=""
NAME=flow-smoke VOL=flow-smoke-data
die() { echo "smoke.sh: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
    case "$1" in
        --image) IMAGE="${2:?}"; shift ;;
        --wav) WAV="${2:?}"; shift ;;
        --models) MODELS="${2:?}"; shift ;;
        --seed) SEED="$(readlink -f "${2:?}")"; shift ;;
        --port) PORT="${2:?}"; shift ;;
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
    shift
done
{ [ -n "$IMAGE" ] && [ -n "$WAV" ]; } || die "--image and --wav are required"

TMP="$(mktemp -d)"
cleanup() {
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker volume rm "$VOL" >/dev/null 2>&1 || true
    rm -rf "$TMP"
}
trap cleanup EXIT
cleanup
mkdir -p "$TMP"

wait_log() { # wait_log PATTERN SECONDS [COUNT]: COUNT matching lines in the log
    local i
    for ((i = 0; i < $2; i++)); do
        [ "$(docker logs "$NAME" 2>&1 | grep -c "$1")" -ge "${3:-1}" ] && return 0
        docker inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null | grep -q true || break
        sleep 1
    done
    docker logs "$NAME" 2>&1 | tail -20 >&2
    die "timed out waiting for: $1"
}
dictate() { # dictate TOKEN_FILE
    python3 "$ROOT/tools/probes/prototypes/server_e2e_2026_10_31.py" \
        --url "ws://127.0.0.1:$PORT" --token-file "$1" --wav "$WAV" \
        --llm-timeout-ms 20000 | grep '"wav"' | python3 -c '
import json, sys
r = json.loads(sys.stdin.read())
print("  %s lang=%s commit->result=%s ms: %s" % (r["status"], r["language"], r["commit_to_result_ms"], r["final_text"][:80]))
sys.exit(0 if r["final_text"] else 1)'
}

docker volume create "$VOL" >/dev/null
if [ -n "$SEED" ]; then
    echo "==> seeding the library from $SEED"
    docker run --rm -v "$VOL:/data" -v "$SEED:/seed:ro" --entrypoint sh "$IMAGE" \
        -c 'mkdir -p /data/library && cp -r /seed/. /data/library/'
fi

echo "==> first start (FLOW_MODELS=$MODELS)"
t0=$(date +%s)
docker run -d --name "$NAME" -p "127.0.0.1:$PORT:7313" -v "$VOL:/data" \
    -e FLOW_MODELS="$MODELS" -e FLOW_CONTAINER_NAME="$NAME" "$IMAGE" >/dev/null
wait_log "The admin key, shown only this once" 1800
# The key from the log (the banner's indented line), never echoed.
(umask 077; docker logs "$NAME" 2>&1 | awk '/shown only this once/ { getline; getline; print $1; exit }' >"$TMP/admin")
[ -s "$TMP/admin" ] || die "no admin key in the log"
docker exec "$NAME" cat /data/admin.token | cmp -s - "$TMP/admin" || die "logged key != /data/admin.token"
wait_log 'flow_server: ready' 900
echo "    ready after $(($(date +%s) - t0)) s"
echo "==> dictation with the admin key"
dictate "$TMP/admin"
echo "==> docker exec … flow-server keys add / list"
(umask 077; docker exec "$NAME" flow-server keys add smoke-phone 2>/dev/null >"$TMP/phone")
docker exec "$NAME" flow-server keys list | sed 's/^/    /'
dictate "$TMP/phone"
echo "==> restart: no key in the log, keys kept"
docker restart "$NAME" >/dev/null
wait_log 'flow_server: ready' 900 2
[ "$(docker logs "$NAME" 2>&1 | grep -c "shown only this once")" = 1 ] || die "admin key shown again"
dictate "$TMP/admin"
docker exec "$NAME" flow-server library list | sed 's/^/    /'
echo "==> health: $(docker inspect "$NAME" --format '{{.State.Health.Status}}')"
echo "smoke OK"
