#!/bin/sh
# flow-server container entrypoint (docs/container.md).
#
#   (no args) | serve [flags]          run the server
#   check-config [flags]               validate, with the same bind rewrite
#   keys … | library … | gen-token …   run that CLI command
#   anything else                      exec'd as is (e.g. `sh`)
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
#
# No keys yet (fresh state volume, no token_file): instead of exiting, wait
# until `flow-server keys add <name> --admin` (docker compose exec / kubectl
# exec) creates keys.json, then start. A crash loop would make `exec`
# impossible. The healthcheck counts the wait as alive.
set -eu

BIN=/opt/flow-server/bin/flow-server
CONFIG="${FLOW_CONFIG:-/etc/flow-server/config.toml}"
RUN_DIR="${FLOW_RUN_DIR:-/tmp}"
PORT="${FLOW_PORT:-7313}"
WAITING="$RUN_DIR/flow-server.waiting-for-keys"
STARTED="$RUN_DIR/flow-server.started"

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

case "${1:-serve}" in
    keys|library)
        if has_config_flag "$@"; then exec "$BIN" "$@"; fi
        exec "$BIN" "$@" --config "$CONFIG" ;;
    gen-token)
        exec "$BIN" "$@" ;;
    check-config)
        if has_config_flag "$@"; then exec "$BIN" "$@"; fi
        shift
        set -- "$@" --config "$(effective_config)"
        [ "${FLOW_INSECURE_BIND_ANY:-0}" = 1 ] && set -- "$@" --insecure-bind-any
        exec "$BIN" check-config "$@" ;;
    serve|--*) ;;
    *) exec "$@" ;;
esac
[ "${1:-}" = serve ] && shift

if ! has_config_flag "$@"; then
    EFFECTIVE="$(effective_config)"
    [ "$EFFECTIVE" = "$CONFIG" ] || echo "entrypoint: bind = $(sed -n 2p "$EFFECTIVE" | cut -d= -f2-)" >&2
    set -- --config "$EFFECTIVE" "$@"
fi
if [ "${FLOW_INSECURE_BIND_ANY:-0}" = 1 ]; then
    echo "entrypoint: WARNING: FLOW_INSECURE_BIND_ANY=1: wildcard/public bind allowed without TLS and no peer filter. Publish the port on a loopback, LAN or VPN host address only, or configure [tls] (docs/container.md)." >&2
    set -- "$@" --insecure-bind-any
fi

# Wait for the first key. check-config is cheap (no model load).
announced=0
while :; do
    if out="$("$BIN" check-config "$@" 2>&1)"; then
        break
    fi
    case "$out" in
        *"no keys"*)
            : >"$WAITING" 2>/dev/null || true
            if [ "$announced" = 0 ]; then
                echo "entrypoint: no keys in the state dir yet. Add the admin key, e.g." >&2
                echo "entrypoint:   docker compose exec flow-server flow-server keys add owner --admin" >&2
                echo "entrypoint:   kubectl exec deploy/<release>-flow-server -- flow-server keys add owner --admin" >&2
                echo "entrypoint: waiting for keys.json ..." >&2
                announced=1
            fi
            sleep "${FLOW_KEY_POLL_S:-3}" ;;
        *)
            echo "$out" >&2
            exit 1 ;;
    esac
done
rm -f "$WAITING" 2>/dev/null || true
[ "$announced" = 1 ] && echo "entrypoint: keys found, starting" >&2
# Models load before the listener binds (Parakeet + LLM: up to minutes on a
# loaded box); the healthcheck allows FLOW_START_GRACE_S from here.
date +%s >"$STARTED" 2>/dev/null || true
exec "$BIN" "$@"
