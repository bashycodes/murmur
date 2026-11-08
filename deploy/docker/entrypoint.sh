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
# that name it only runs CLI commands (never the setup below).
#
# Zero-config start (D77). Everything lives in one volume, FLOW_DATA (/data):
#   config.toml   written on the first start, yours to edit afterwards
#   admin.token   the first admin key (token_file, imported as key "owner")
#   library/      the models, one folder per model (D40): the server uses
#                 them and the apps can download them from it
#   keys.json, key-usage.json, library-hashes.json   server state
# On every start the models named by FLOW_MODELS that are missing from
# library/ are downloaded from the pinned list (docs/air-gapped-models.txt,
# SHA-256 checked). FLOW_MODELS: `default` (English Parakeet, language
# detection, Hebrew zipformer, Qwen2.5 1.5B cleanup), `english` (Parakeet
# only), `none`, or model folder ids (comma or space separated; `default`
# and `english` may be mixed in). The config is written once, from the
# models present then.
#
# A config elsewhere (FLOW_CONFIG not $FLOW_DATA/config.toml, e.g. the Helm
# chart's /etc/flow-server/config.toml) is used as is: no download, no
# config, no admin token. Then, with no keys yet, the entrypoint waits until
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
CONFIG="${FLOW_CONFIG:-$DATA/config.toml}"
MANIFEST="${FLOW_MODEL_LIST:-/opt/flow-server/share/models.txt}"
RUN_DIR="${FLOW_RUN_DIR:-/tmp}"
PORT="${FLOW_PORT:-7313}"
# Container name in the hints printed to the log (the quick start's --name).
NAME="${FLOW_CONTAINER_NAME:-flow-server}"
PREPARING="$RUN_DIR/flow-server.preparing"
STARTED="$RUN_DIR/flow-server.started"
DEFAULT_MODELS="parakeet-tdt-0.6b-v2-int8 whisper-tiny-lid zipformer-he-int8 qwen2.5-1.5b-instruct-q4_k_m"
ENGLISH_MODELS="parakeet-tdt-0.6b-v2-int8"

say() { echo "flow-server: $*" >&2; }
die() { say "error: $*"; exit 1; }

has_config_flag() {
    for x in "$@"; do [ "$x" = "--config" ] && return 0; done
    return 1
}

# The config to run with: $CONFIG, or a copy in $RUN_DIR with `bind`
# replaced by loopback + the container's addresses.
effective_config() {
    if [ "${FLOW_BIND_CONTAINER_IP:-1}" != 1 ]; then
        echo "$CONFIG"
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
        echo "# $CONFIG with bind rewritten by the entrypoint (FLOW_BIND_CONTAINER_IP=1)"
        echo "bind = [$binds]"
        # Drop the original top-level bind (single line, before any [table]).
        awk 'BEGIN { top = 1 } /^[[:space:]]*\[/ { top = 0 }
             top && /^[[:space:]]*bind[[:space:]]*=/ { next } { print }' "$CONFIG"
    } >"$out"
    echo "$out"
}

# ---- models (D77) -------------------------------------------------------------

# The model folder ids FLOW_MODELS names, one per line, presets expanded,
# duplicates dropped, order kept.
model_ids() {
    for m in $(echo "${FLOW_MODELS:-default}" | tr ',' ' '); do
        case "$m" in
            default) echo "$DEFAULT_MODELS" ;;
            english) echo "$ENGLISH_MODELS" ;;
            none) ;;
            *) echo "$m" ;;
        esac
    done | tr ' ' '\n' | awk 'NF && !seen[$0]++'
}

# The pinned manifest lines (folder kind engine file bytes sha256 url title)
# of one model folder.
manifest_lines() {
    awk -v id="$1" '!/^#/ && $1 == id' "$MANIFEST"
}

sha_of() { sha256sum "$1" | cut -d' ' -f1; }
size_of() { stat -c %s "$1" 2>/dev/null || echo -1; }

# fetch_model ID: every file of the model in $DATA/library/ID, then its
# model.toml (written last: the library serves a folder only once tagged).
# A tagged folder whose files have the pinned sizes is not hashed again.
fetch_model() {
    id="$1"
    lines="$(manifest_lines "$id")"
    [ -n "$lines" ] || die "FLOW_MODELS: unknown model \"$id\"; known: $(awk '!/^#/ && NF { print $1 }' "$MANIFEST" | sort -u | tr '\n' ' ')"
    dir="$DATA/library/$id"
    mkdir -p "$dir"
    tagged=0
    [ -f "$dir/model.toml" ] && tagged=1
    echo "$lines" | while read -r _ kind engine file bytes sha url title; do
        f="$dir/$file"
        if [ "$(size_of "$f")" = "$bytes" ]; then
            [ "$tagged" = 1 ] && continue
            [ "$(sha_of "$f")" = "$sha" ] && continue
        fi
        command -v curl >/dev/null ||
            die "$id/$file is missing and this image has no curl (built with WITH_CURL=0): put the model folders into $DATA/library first (docs/container.md §3)"
        say "downloading $id/$file ($((bytes / 1000000)) MB)"
        t0=$(date +%s)
        curl -fL --retry 3 --retry-delay 5 -C - -sS -o "$f.part" "$url" </dev/null \
            || { rm -f "$f.part"; die "download failed: $url"; }
        if [ "$(sha_of "$f.part")" != "$sha" ]; then
            rm -f "$f.part"
            die "$id/$file does not match its pinned SHA-256 (download corrupted?)"
        fi
        mv -f "$f.part" "$f"
        say "  done in $(($(date +%s) - t0)) s"
    done
    if [ "$tagged" = 0 ]; then
        echo "$lines" | head -n 1 | while read -r _ kind engine _ _ _ _ title; do
            {
                echo "kind = \"$kind\""
                echo "title = \"$title\""
                echo "engine = \"$engine\""
                # Language detection is not a catalogue model (air-gapped-models.txt).
                [ "$kind" = lid ] || echo "catalogue_id = \"$id\""
            } >"$dir/model.toml"
        done
    fi
}

# What the config uses from one model folder: its directory (sherpa-onnx
# models) or its single file (whisper, GGUF).
model_path() {
    manifest_lines "$1" | {
        read -r _ _ engine file _
        case "$engine" in
            whisper|gguf) echo "$DATA/library/$1/$file" ;;
            *) echo "$DATA/library/$1" ;;
        esac
    }
}
model_field() { manifest_lines "$1" | head -n 1 | awk -v n="$2" '{ print $n }'; }

# write_config: config.toml for the models now in the library. The main
# speech model is the first `stt` model of FLOW_MODELS; the first `lid`,
# `llm` and `hebrew` ones add language detection, AI cleanup and Hebrew.
write_config() {
    stt="" lid="" llm="" he=""
    for id in $(model_ids); do
        case "$(model_field "$id" 2)" in
            stt) [ -n "$stt" ] || stt="$id" ;;
            lid) [ -n "$lid" ] || lid="$id" ;;
            llm) [ -n "$llm" ] || llm="$id" ;;
            hebrew) [ -n "$he" ] || he="$id" ;;
        esac
    done
    [ -n "$stt" ] || die "FLOW_MODELS has no speech model (kind stt), e.g. FLOW_MODELS=default; or write $CONFIG yourself"
    threads="${FLOW_THREADS:-$(nproc 2>/dev/null || echo 4)}"
    [ "$threads" -le 4 ] 2>/dev/null || threads=4
    tmp="$CONFIG.tmp"
    {
        echo "# flow-server config, written on the first start (FLOW_MODELS=${FLOW_MODELS:-default})."
        echo "# Edit and restart the container to change it. Every key:"
        echo "# crates/flow-server/dist/config.example.toml in the source repo."
        echo "#"
        echo "# bind: the entrypoint replaces it with the container's own address"
        echo "# (FLOW_BIND_CONTAINER_IP=1, the default); only loopback, LAN and VPN"
        echo "# peers are accepted (docs/container.md \"Binding\")."
        echo "bind = [\"0.0.0.0:$PORT\"]"
        echo "token_file = \"$DATA/admin.token\"   # the first admin key, \"owner\" in \`keys list\`"
        echo "state_dir = \"$DATA\""
        echo "workers = 1            # dictations decoded at once; each holds its models"
        echo "max_queue = 2"
        echo
        echo "[model]"
        echo "asr_engine = \"$(model_field "$stt" 3)\""
        echo "asr_path = \"$(model_path "$stt")\""
        echo "threads = $threads"
        [ -z "$lid" ] || echo "lid_path = \"$(model_path "$lid")\"   # spoken-language detection"
        [ -z "$llm" ] || echo "llm_path = \"$(model_path "$llm")\"   # AI cleanup"
        if [ -n "$he" ]; then
            echo
            echo "# Hebrew dictations (app language Hebrew, or detected by lid_path)."
            echo "[model.languages.he]"
            echo "asr_backend = \"$(model_field "$he" 3)\""
            echo "asr_path = \"$(model_path "$he")\""
        fi
        echo
        echo "[library]              # the models the apps can download from this server"
        echo "dir = \"$DATA/library\""
        echo
        echo "[log]"
        echo "level = \"info\""
        echo "log_text = false       # never log transcripts"
        echo
        echo "# [tls]                # wss:// (docs/container.md \"TLS\")"
        echo "# cert = \"$DATA/tls/cert.pem\""
        echo "# key = \"$DATA/tls/key.pem\""
    } >"$tmp"
    mv -f "$tmp" "$CONFIG"
    say "wrote $CONFIG (speech $stt${lid:+, detection $lid}${llm:+, cleanup $llm}${he:+, Hebrew $he})"
}

# setup: the zero-config part of a start (config in $DATA only).
setup() {
    [ "$CONFIG" = "$DATA/config.toml" ] || return 0
    { [ -d "$DATA" ] && [ -w "$DATA" ]; } ||
        die "$DATA is not writable by uid $(id -u). Use a Docker volume (-v flow_data:$DATA), or for a host folder: sudo chown -R $(id -u):$(id -g) <folder>"
    [ -f "$MANIFEST" ] || die "model list $MANIFEST missing from the image"
    : >"$PREPARING" 2>/dev/null || true
    mkdir -p "$DATA/library"
    for id in $(model_ids); do fetch_model "$id"; done
    [ -f "$CONFIG" ] || write_config
    if [ ! -f "$DATA/admin.token" ] && grep -q "^token_file = \"$DATA/admin.token\"" "$CONFIG"; then
        "$BIN" gen-token --out "$DATA/admin.token" 2>/dev/null
        cat >&2 <<EOF

=====================================================================
 flow-server is set up. The admin key, shown only this once:

     $(cat "$DATA/admin.token")

 In each app: Settings > Your server: this machine's LAN or VPN
 address, port $PORT, and the key above.
 Show the key again:  docker exec $NAME cat $DATA/admin.token
 One key per device:  docker exec $NAME flow-server keys add phone
=====================================================================

EOF
    elif [ -f "$DATA/admin.token" ]; then
        say "admin key: docker exec $NAME cat $DATA/admin.token"
    fi
}

# ---- commands -----------------------------------------------------------------

cli() {
    case "${1:-}" in
        keys|library)
            if has_config_flag "$@"; then exec "$BIN" "$@"; fi
            exec "$BIN" "$@" --config "$CONFIG" ;;
        check-config)
            if has_config_flag "$@"; then exec "$BIN" "$@"; fi
            shift
            set -- "$@" --config "$(effective_config)"
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
    setup
    EFFECTIVE="$(effective_config)"
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
