#!/usr/bin/env bash
# Smoke test of the murmur-server image's quick start (docs/server-setup.md,
# D77): the documented `docker run` with a models folder mounted read-only
# and a fresh volume, the admin key read with `docker exec` (never from the
# log), a dictation over the real WebSocket protocol with that key, a second
# key through `docker exec … murmur-server keys add`, then a restart (same key
# still works, still never logged).
#
#   tools/container/smoke.sh --image IMAGE --wav FILE[:LANGS] --models DIR
#                            [--port N]
#
#   --image IMAGE  image to test (e.g. murmur-server)
#   --wav FILE     16 kHz WAV to dictate (LANGS e.g. `en`; see the probe)
#   --models DIR   models folder: one folder per model with its model.toml
#                  (e.g. the colleague bundle's library/), mounted at /models
#   --port N       host port (default 17313; never the production server's)
#
# Uses its own container (murmur-smoke) and volume (murmur-smoke-data) and
# removes both at the end. The key stays in a mode-0600 temp file.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PORT=17313 IMAGE="" WAV="" MODELS=""
NAME=murmur-smoke VOL=murmur-smoke-data
die() { echo "smoke.sh: $*" >&2; exit 1; }
while [ $# -gt 0 ]; do
    case "$1" in
        --image) IMAGE="${2:?}"; shift ;;
        --wav) WAV="${2:?}"; shift ;;
        --models) MODELS="$(readlink -f "${2:?}")"; shift ;;
        --port) PORT="${2:?}"; shift ;;
        -h|--help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
    shift
done
{ [ -n "$IMAGE" ] && [ -n "$WAV" ] && [ -d "$MODELS" ]; } ||
    die "--image, --wav and --models are required"

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
key_not_logged() {
    if docker logs "$NAME" 2>&1 | grep -qF "$(cat "$TMP/admin")"; then
        die "admin key in the log"
    fi
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

echo "==> first start (models from $MODELS)"
t0=$(date +%s)
docker run -d --name "$NAME" -p "127.0.0.1:$PORT:7313" -v "$MODELS:/models:ro" \
    -v "$VOL:/data" -e MURMUR_CONTAINER_NAME="$NAME" "$IMAGE" >/dev/null
wait_log "created its admin key" 120
(umask 077; docker exec "$NAME" cat /data/admin.token >"$TMP/admin")
[ -s "$TMP/admin" ] || die "no /data/admin.token"
key_not_logged
docker logs "$NAME" 2>&1 | grep "models from" | sed 's/^/    /'
wait_log 'murmur_server: ready' 900
echo "    ready after $(($(date +%s) - t0)) s"
echo "==> dictation with the admin key"
dictate "$TMP/admin"
echo "==> docker exec … murmur-server keys add / list"
(umask 077; docker exec "$NAME" murmur-server keys add smoke-phone 2>/dev/null >"$TMP/phone")
docker exec "$NAME" murmur-server keys list | sed 's/^/    /'
dictate "$TMP/phone"
echo "==> restart: keys kept, key still not logged"
docker restart "$NAME" >/dev/null
wait_log 'murmur_server: ready' 900 2
key_not_logged
docker logs "$NAME" 2>&1 | grep -q "admin key: docker exec $NAME cat /data/admin.token" ||
    die "no key hint after the restart"
dictate "$TMP/admin"
docker exec "$NAME" murmur-server library list | sed 's/^/    /'
echo "==> health: $(docker inspect "$NAME" --format '{{.State.Health.Status}}')"
echo "smoke OK"
