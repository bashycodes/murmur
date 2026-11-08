#!/bin/sh
# flow-server container entrypoint (docs/container.md, D77).
#
#   (no args) | serve [flags]          run the server
#   check-config [flags]               validate, with the same bind rewrite
#   keys … | library … | gen-token …   run that CLI command
#   anything else                      exec'd as is (e.g. `sh`)
#
# Also installed as /usr/local/bin/flow-server, so `docker exec <container>
# flow-server keys add phone` finds the config without --config. Called by
# that name it only runs CLI commands.
#
# Zero-config start (D77, FLOW_CONFIG unset):
#   /models   (FLOW_MODELS_DIR) the models folder, mounted read-only: one
#             folder per model, each tagged by its model.toml (D40), e.g. the
#             colleague bundle's unzipped library/. The server uses them and
#             serves them to the apps (the library).
#   /data     (FLOW_DATA) the volume: admin.token (the first admin key,
#             token_file, imported as key "owner"), keys.json,
#             key-usage.json, library-hashes.json, and optionally your own
#             config.toml.
# Without /data/config.toml the config is generated on every start from the
# tagged folders in /models (pick_model below; FLOW_STT / FLOW_LLM /
# FLOW_HEBREW / FLOW_LID name a folder or `none`). The key is never printed:
# the log says how to read it (`docker exec … cat /data/admin.token`).
#
# FLOW_CONFIG set (e.g. the Helm chart's /etc/flow-server/config.toml): that
# config as is. Then, with no keys yet, the entrypoint waits until
# `flow-server keys add <name> --admin` creates keys.json (a crash loop
# would make `exec` impossible).
#
# Binding (docs/container.md "Binding"). The server refuses a wildcard bind
# (0.0.0.0) without [tls] or --insecure-bind-any (config.rs check_bind), so:
# * FLOW_BIND_CONTAINER_IP=1 (default): replace the config's `bind` with
#   127.0.0.1 and this container's own addresses (private on Docker and
#   Kubernetes networks), port FLOW_PORT (default 7313). Published/forwarded
#   ports reach those addresses; the peer filter (loopback/LAN/VPN only)
#   stays on.
# * FLOW_BIND_CONTAINER_IP=0: use the config's `bind` as written (e.g.
#   0.0.0.0 with [tls], or with FLOW_INSECURE_BIND_ANY=1).
# * FLOW_INSECURE_BIND_ANY=1 adds --insecure-bind-any (warns).
set -eu

BIN="${FLOW_BIN:-/opt/flow-server/bin/flow-server}"
DATA="${FLOW_DATA:-/data}"
MODELS="${FLOW_MODELS_DIR:-/models}"
RUN_DIR="${FLOW_RUN_DIR:-/tmp}"
PORT="${FLOW_PORT:-7313}"
# Container name in the hints printed to the log (the quick start's --name).
NAME="${FLOW_CONTAINER_NAME:-flow-server}"
GENERATED="$RUN_DIR/flow-server.generated.toml"
PREPARING="$RUN_DIR/flow-server.preparing"
STARTED="$RUN_DIR/flow-server.started"

say() { echo "flow-server: $*" >&2; }
die() { say "error: $*"; exit 1; }

has_config_flag() {
    for x in "$@"; do [ "$x" = "--config" ] && return 0; done
    return 1
}

# ---- models folder (D77) ------------------------------------------------------

# tag DIR KEY: a string value from DIR/model.toml (empty when absent).
tag() {
    sed -n -E "s/^[[:space:]]*$2[[:space:]]*=[[:space:]]*\"([^\"]*)\".*/\1/p" \
        "$1/model.toml" 2>/dev/null | head -n 1
}

# model_file DIR: the model's single weights file (whisper .bin, GGUF).
model_file() {
    for f in "$1"/*.gguf "$1"/*.bin; do
        [ -f "$f" ] && { echo "$f"; return 0; }
    done
    return 1
}

# engine ID: model.toml's engine, else whisper / gguf from the files.
engine() {
    e="$(tag "$MODELS/$1" engine)"
    if [ -z "$e" ]; then
        case "$(model_file "$MODELS/$1" || true)" in
            *.gguf) e=gguf ;;
            *.bin) e=whisper ;;
            *) die "$MODELS/$1/model.toml has no engine (parakeet, sherpa_online, whisper, gguf)" ;;
        esac
    fi
    echo "$e"
}

# model_path ID: what the config names: the folder (sherpa-onnx models) or
# the weights file (whisper, GGUF).
model_path() {
    case "$(engine "$1")" in
        whisper|gguf) model_file "$MODELS/$1" || die "$MODELS/$1: no .bin or .gguf file" ;;
        *) echo "$MODELS/$1" ;;
    esac
}

# pick_model KIND OVERRIDE PREFERRED...: the folder id the config uses for
# KIND: OVERRIDE (a folder id, or `none`), else the first PREFERRED folder
# that is there, else the first folder of that kind by name; empty if none.
pick_model() {
    kind="$1" want="$2"
    shift 2
    [ "$want" = none ] && return 0
    if [ -n "$want" ]; then
        [ "$(tag "$MODELS/$want" kind)" = "$kind" ] ||
            die "$want is not a \"$kind\" model in $MODELS (no folder, or its model.toml says another kind)"
        echo "$want"
        return 0
    fi
    for id in "$@"; do
        [ "$(tag "$MODELS/$id" kind)" = "$kind" ] && { echo "$id"; return 0; }
    done
    for d in "$MODELS"/*/; do
        [ "$(tag "$d" kind)" = "$kind" ] && { basename "$d"; return 0; }
    done
    return 0
}

# generate_config FILE: a config for the tagged models in $MODELS.
generate_config() {
    [ -d "$MODELS" ] ||
        die "no models folder at $MODELS. Mount one: -v /path/to/models:$MODELS:ro (docs/server-setup.md)"
    stt="$(pick_model stt "${FLOW_STT:-}" parakeet-tdt-0.6b-v2-int8 parakeet-unified-en-0.6b-int8)"
    [ -n "$stt" ] ||
        die "no speech model in $MODELS: it needs a folder per model with a model.toml (kind = \"stt\"), e.g. the colleague bundle's library/ (docs/server-setup.md)"
    lid="$(pick_model lid "${FLOW_LID:-}" whisper-tiny-lid)"
    llm="$(pick_model llm "${FLOW_LLM:-}" qwen2.5-1.5b-instruct-q4_k_m)"
    he="$(pick_model hebrew "${FLOW_HEBREW:-}" zipformer-he-int8)"
    stt_engine="$(engine "$stt")" stt_path="$(model_path "$stt")"
    lid_path="" llm_path="" he_engine="" he_path=""
    [ -z "$lid" ] || lid_path="$(model_path "$lid")"
    [ -z "$llm" ] || llm_path="$(model_path "$llm")"
    [ -z "$he" ] || { he_engine="$(engine "$he")"; he_path="$(model_path "$he")"; }
    threads="${FLOW_THREADS:-$(nproc 2>/dev/null || echo 4)}"
    [ "$threads" -le 4 ] 2>/dev/null || threads=4
    {
        echo "# Generated by the entrypoint from the models in $MODELS. To change"
        echo "# it, copy it to $DATA/config.toml, edit, restart (docs/container.md)."
        echo "bind = [\"0.0.0.0:$PORT\"]   # replaced by the container's address"
        echo "token_file = \"$DATA/admin.token\""
        echo "state_dir = \"$DATA\""
        echo "workers = 1"
        echo "max_queue = 2"
        echo
        echo "[model]"
        echo "asr_engine = \"$stt_engine\""
        echo "asr_path = \"$stt_path\""
        echo "threads = $threads"
        [ -z "$lid_path" ] || echo "lid_path = \"$lid_path\""
        [ -z "$llm_path" ] || echo "llm_path = \"$llm_path\""
        if [ -n "$he_path" ]; then
            echo
            echo "[model.languages.he]"
            echo "asr_backend = \"$he_engine\""
            echo "asr_path = \"$he_path\""
        fi
        echo
        echo "[library]"
        echo "dir = \"$MODELS\""
        echo
        echo "[log]"
        echo "level = \"info\""
        echo "log_text = false"
    } >"$1.tmp"
    mv -f "$1.tmp" "$1"
    say "models from $MODELS: speech $stt${lid:+, language detection $lid}${llm:+, AI cleanup $llm}${he:+, Hebrew $he}"
}

# The config file: FLOW_CONFIG, else $DATA/config.toml, else generated.
config_file() {
    if [ -n "${FLOW_CONFIG:-}" ]; then
        echo "$FLOW_CONFIG"
    elif [ -f "$DATA/config.toml" ]; then
        echo "$DATA/config.toml"
    else
        generate_config "$GENERATED"
        echo "$GENERATED"
    fi
}

# The config to run with: $1, or a copy in $RUN_DIR with `bind` replaced by
# loopback + the container's addresses.
effective_config() {
    if [ "${FLOW_BIND_CONTAINER_IP:-1}" != 1 ]; then
        echo "$1"
        return
    fi
    host="$(cat /etc/hostname 2>/dev/null || hostname)"
    binds="\"127.0.0.1:$PORT\""
    for ip in $(getent hosts "$host" | awk '{print $1}' | sort -u); do
        case "$ip" in
            127.*|::1) ;;
            *:*) binds="$binds, \"[$ip]:$PORT\"" ;;
            *) binds="$binds, \"$ip:$PORT\"" ;;
        esac
    done
    out="$RUN_DIR/flow-server.config.toml"
    {
        echo "# $1 with bind rewritten by the entrypoint (FLOW_BIND_CONTAINER_IP=1)"
        echo "bind = [$binds]"
        # Drop the original top-level bind (single line, before any [table]).
        awk 'BEGIN { top = 1 } /^[[:space:]]*\[/ { top = 0 }
             top && /^[[:space:]]*bind[[:space:]]*=/ { next } { print }' "$1"
    } >"$out"
    echo "$out"
}

# admin_token CONFIG: create $DATA/admin.token when the config names it and
# it is missing. The key itself never goes to the log (CODING_STANDARDS
# §Privacy), only how to read it.
admin_token() {
    grep -q "^token_file = \"$DATA/admin.token\"" "$1" || return 0
    if [ ! -f "$DATA/admin.token" ]; then
        { [ -d "$DATA" ] && [ -w "$DATA" ]; } ||
            die "$DATA is not writable by uid $(id -u). Use a Docker volume (-v flow_data:$DATA), or for a host folder: sudo chown -R $(id -u):$(id -g) <folder>"
        "$BIN" gen-token --out "$DATA/admin.token" >/dev/null 2>&1 ||
            die "could not write the admin key to $DATA/admin.token"
        cat >&2 <<EOF

=====================================================================
 flow-server created its admin key. Show it (treat it like a password):

     docker exec $NAME cat $DATA/admin.token

 In each app, Settings > Your server: this machine's LAN or VPN
 address (\`hostname -I\` on this machine), port $PORT, and the key.
 One key per device:  docker exec $NAME flow-server keys add phone
=====================================================================

EOF
    else
        say "admin key: docker exec $NAME cat $DATA/admin.token"
    fi
}

# ---- commands -----------------------------------------------------------------

cli() {
    case "${1:-}" in
        keys|library)
            if has_config_flag "$@"; then exec "$BIN" "$@"; fi
            c="$(config_file)"
            exec "$BIN" "$@" --config "$c" ;;
        check-config)
            if has_config_flag "$@"; then exec "$BIN" "$@"; fi
            shift
            c="$(config_file)"
            set -- "$@" --config "$(effective_config "$c")"
            [ "${FLOW_INSECURE_BIND_ANY:-0}" = 1 ] && set -- "$@" --insecure-bind-any
            exec "$BIN" check-config "$@" ;;
    esac
    exec "$BIN" "$@"
}

# Called as `flow-server` (docker exec): CLI only.
[ "$(basename "$0")" = flow-server ] && cli "$@"

case "${1:-serve}" in
    keys|library|check-config|gen-token) cli "$@" ;;
    serve|--*) ;;
    *) exec "$@" ;;
esac
[ "${1:-}" = serve ] && shift

if ! has_config_flag "$@"; then
    CONFIG="$(config_file)"
    [ -n "${FLOW_CONFIG:-}" ] || admin_token "$CONFIG"
    EFFECTIVE="$(effective_config "$CONFIG")"
    [ "$EFFECTIVE" = "$CONFIG" ] || say "bind = $(sed -n 2p "$EFFECTIVE" | cut -d= -f2-)"
    set -- --config "$EFFECTIVE" "$@"
fi
if [ "${FLOW_INSECURE_BIND_ANY:-0}" = 1 ]; then
    say "WARNING: FLOW_INSECURE_BIND_ANY=1: wildcard/public bind allowed without TLS and no peer filter. Publish the port on a loopback, LAN or VPN host address only, or configure [tls] (docs/container.md)."
    set -- "$@" --insecure-bind-any
fi

# Wait for the first key (a config without token_file). check-config is
# cheap (no model load).
announced=0
while :; do
    if out="$("$BIN" check-config "$@" 2>&1)"; then
        break
    fi
    case "$out" in
        *"no keys"*)
            : >"$PREPARING" 2>/dev/null || true
            if [ "$announced" = 0 ]; then
                say "no keys in the state dir yet. Add the admin key, e.g."
                say "  docker exec $NAME flow-server keys add owner --admin"
                say "  kubectl exec deploy/<release>-flow-server -- flow-server keys add owner --admin"
                say "waiting for keys.json ..."
                announced=1
            fi
            sleep "${FLOW_KEY_POLL_S:-3}" ;;
        *)
            echo "$out" >&2
            exit 1 ;;
    esac
done
[ "$announced" = 1 ] && say "keys found, starting"
rm -f "$PREPARING" 2>/dev/null || true
# Models load before the listener binds (Parakeet + LLM: up to minutes on a
# loaded box); the healthcheck allows FLOW_START_GRACE_S from here.
date +%s >"$STARTED" 2>/dev/null || true
say "loading models (about a minute), then \"ready\""
exec "$BIN" "$@"
