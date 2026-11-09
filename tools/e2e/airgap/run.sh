#!/usr/bin/env bash
# Air-gapped end-to-end check (docs/air-gapped.md): a murmur-server built from
# this checkout, on loopback, with a temp state dir and a temp model library,
# driven by the apps' own client code (crates/murmur-remote/examples/airgap_e2e.rs).
#
#   tools/e2e/airgap/run.sh [--models DIR] [--wav FILE] [--expect-text WORD]
#                           [--port N] [--no-build] [--keep]
#
#   --models DIR   folder with ggml-tiny.en.bin (the server's speech model and
#                  the library's first model) and, optionally, ggml-tiny.bin
#                  (a second library model). Read only. Default: $MURMUR_MODELS_DIR,
#                  else <repo>/models.
#   --wav FILE     16 kHz mono clip to dictate (public audio only).
#                  Default: <models>/jfk.wav
#   --expect-text  a word the transcript must contain (default: country)
#   --port N       loopback port (default 17399; never the real server's 7313)
#   --no-build     use the binaries already in the target dir
#   --keep         keep the temp dir (state, library, logs, strace) for a look
#
# Network: if unprivileged user + network namespaces work (`unshare -rn`),
# the server and the client run in a fresh namespace that has only loopback,
# so any outbound attempt fails by construction. Either way both processes
# run under strace and the check fails on any connect()/sendto() to an
# address that is not loopback.
#
# Builds honour CARGO_TARGET_DIR and BUILD_PREFIX (e.g.
# BUILD_PREFIX="flock $HOME/.cache/flow-clone-build.lock nice -n 10").
# Secrets (the admin key, the user key) stay in a mode-0600 temp file and in
# the client's memory; nothing here prints them.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
MODELS="${MURMUR_MODELS_DIR:-$ROOT/models}"
WAV="" EXPECT_TEXT="country" PORT=17399 BUILD=1 KEEP=0 INNER=""
die() { echo "airgap: $*" >&2; exit 2; }
while [ $# -gt 0 ]; do
    case "$1" in
        --models) MODELS="${2:?}"; shift ;;
        --wav) WAV="${2:?}"; shift ;;
        --expect-text) EXPECT_TEXT="${2:?}"; shift ;;
        --port) PORT="${2:?}"; shift ;;
        --no-build) BUILD=0 ;;
        --keep) KEEP=1 ;;
        --inner) INNER="${2:?}"; shift ;;  # internal: re-exec inside the namespace
        -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
    shift
done
MODELS="$(readlink -f "$MODELS")"
WAV="$(readlink -f "${WAV:-$MODELS/jfk.wav}")"
SERVER="$TARGET/release/murmur-server"
CLIENT="$TARGET/debug/examples/airgap_e2e"

# ---------------------------------------------------------------- inner part
# Runs in the namespace (or directly): CLI key, server, client, audit.
inner() {
    local T="$1"
    cd "$T"
    if [ -n "${AIRGAP_NETNS:-}" ]; then
        ip link set lo up
    fi
    local STRACE=()
    if command -v strace >/dev/null; then
        # -s 0: no payload bytes (headers carry the keys) in the logs.
        STRACE=(strace -f -qq -s 0 --seccomp-bpf -e "trace=connect,sendto,sendmsg" -o)
    fi
    umask 077
    echo "==> admin key via the CLI (keys add e2e-admin --admin)"
    "$SERVER" keys add e2e-admin --admin --config config.toml >admin.token 2>cli.log
    if grep -q 'added key "e2e-admin"' cli.log; then
        echo "PASS cli-admin-key: $(head -1 cli.log | cut -d. -f1)"
    else
        cat cli.log; echo "FAIL cli-admin-key"; return 1
    fi

    echo "==> start murmur-server on 127.0.0.1:$PORT"
    if [ ${#STRACE[@]} -gt 0 ]; then
        "${STRACE[@]}" strace.server "$SERVER" --config config.toml >server.log 2>&1 &
    else
        "$SERVER" --config config.toml >server.log 2>&1 &
    fi
    local SPID=$!
    local i
    for ((i = 0; i < 240; i++)); do
        grep -q '"ready"\|ready ' server.log && break
        kill -0 "$SPID" 2>/dev/null || { tail -20 server.log; echo "FAIL server-start"; return 1; }
        sleep 0.5
    done
    grep -q '"ready"\|ready ' server.log || { tail -20 server.log; echo "FAIL server-start (no ready line)"; kill "$SPID"; return 1; }
    echo "PASS server-start: $(grep -m1 'ready' server.log | sed 's/^.*ready/ready/' | cut -c1-160)"

    echo "==> client: admin API, library, dictation, usage, revoke"
    local EXPECTS=() f
    for f in library/*/*; do
        # Tagged folders only: the untagged one must not be listed.
        case "$f" in */model.toml) continue ;; esac
        [ -f "$(dirname "$f")/model.toml" ] || continue
        local id file
        id="$(basename "$(dirname "$f")")" file="$(basename "$f")"
        EXPECTS+=(--expect "$id/$file=$(sha256sum "$f" | cut -d' ' -f1)")
    done
    local CRC=0
    local CARGS=(--port "$PORT" --admin-token-file admin.token --wav "$WAV"
        --expect-text "$EXPECT_TEXT" --out downloads "${EXPECTS[@]}")
    if [ ${#STRACE[@]} -gt 0 ]; then
        "${STRACE[@]}" strace.client "$CLIENT" "${CARGS[@]}" | tee client.log || CRC=$?
    else
        "$CLIENT" "${CARGS[@]}" | tee client.log || CRC=$?
    fi

    echo "==> stop murmur-server (usage is flushed at shutdown)"
    local PID
    PID="$(pgrep -P "$SPID" -x murmur-server || echo "$SPID")"
    kill -TERM "$PID" 2>/dev/null || true
    wait "$SPID" 2>/dev/null || true

    echo "==> keys list via the CLI after shutdown"
    "$SERVER" keys list --config config.toml >keys.txt 2>>cli.log
    sed 's/^/    /' keys.txt
    if grep -E '^e2e-phone ' keys.txt | grep -q revoked && grep -E '^e2e-phone ' keys.txt | grep -q airgap-e2e; then
        echo "PASS cli-keys-list: e2e-phone revoked, usage kept on disk"
    else
        echo "FAIL cli-keys-list"; CRC=1
    fi

    echo "==> network audit"
    if [ ${#STRACE[@]} -gt 0 ]; then
        # Every inet address any traced process tried to reach.
        local addrs bad
        addrs="$(grep -hoE 'inet_addr\("[^"]+"\)|inet_pton\(AF_INET6, "[^"]+"' strace.server strace.client \
            | sed -E 's/.*"([^"]+)".*/\1/' | sort | uniq -c || true)"
        bad="$(echo "$addrs" | awk 'NF && $2 != "127.0.0.1" && $2 != "::1" && $2 != "::ffff:127.0.0.1"')"
        echo "    inet destinations seen: $(echo "$addrs" | awk 'NF{printf "%s x%s  ", $2, $1}')"
        if [ -n "$bad" ]; then
            echo "FAIL no-outbound: non-loopback destinations: $bad"; CRC=1
        else
            echo "PASS no-outbound: only loopback inet destinations ($(grep -c 'connect(' strace.server strace.client | paste -sd' '))"
        fi
    else
        echo "SKIP no-outbound: strace not installed"
    fi
    [ -n "${AIRGAP_NETNS:-}" ] && echo "PASS netns: ran in a network namespace with only loopback"
    return "$CRC"
}

if [ -n "$INNER" ]; then
    rc=0
    inner "$INNER" || rc=$?
    exit "$rc"
fi

# ---------------------------------------------------------------- outer part
[ -f "$MODELS/ggml-tiny.en.bin" ] || die "no ggml-tiny.en.bin in $MODELS (tools/fetch-models.sh, or --models DIR)"
[ -f "$WAV" ] || die "no clip at $WAV (--wav FILE)"

if [ "$BUILD" = 1 ]; then
    echo "==> build murmur-server (release, whisper + tls) and the client example"
    # shellcheck disable=SC2086
    ${BUILD_PREFIX:-} cargo build --release --offline -j 2 -p murmur-server --manifest-path "$ROOT/Cargo.toml"
    # shellcheck disable=SC2086
    ${BUILD_PREFIX:-} cargo build --offline -j 2 -p murmur-remote --features library \
        --example airgap_e2e --manifest-path "$ROOT/Cargo.toml"
fi
{ [ -x "$SERVER" ] && [ -x "$CLIENT" ]; } || die "binaries missing (run without --no-build)"

T="$(mktemp -d -t murmur-airgap.XXXXXX)"
if [ "$KEEP" = 1 ]; then
    trap 'echo "kept $T"' EXIT
else
    trap 'rm -rf "$T"' EXIT
fi

echo "==> temp state + library in $T"
mkdir -p "$T/state" "$T/library/whisper-tiny-en" "$T/downloads"
cp "$MODELS/ggml-tiny.en.bin" "$T/library/whisper-tiny-en/"
cat >"$T/library/whisper-tiny-en/model.toml" <<'EOF'
kind = "stt"
title = "Whisper tiny (English)"
language = "en"
engine = "whisper"
EOF
if [ -f "$MODELS/ggml-tiny.bin" ]; then
    mkdir -p "$T/library/whisper-tiny-lid"
    cp "$MODELS/ggml-tiny.bin" "$T/library/whisper-tiny-lid/"
    cat >"$T/library/whisper-tiny-lid/model.toml" <<'EOF'
kind = "lid"
title = "Whisper tiny (language detection)"
engine = "whisper"
EOF
fi
# An untagged folder: must be skipped, never served.
mkdir -p "$T/library/untagged" && echo x >"$T/library/untagged/weights.bin"

cat >"$T/config.toml" <<EOF
bind = ["127.0.0.1:$PORT"]
state_dir = "$T/state"
workers = 1

[model]
asr_engine = "whisper"
asr_path = "$MODELS/ggml-tiny.en.bin"
threads = 2

[library]
dir = "$T/library"
rescan_s = 2

[log]
level = "info"
EOF

echo "==> library list via the CLI"
"$SERVER" library list --config "$T/config.toml" | sed 's/^/    /'

rc=0
if unshare -rn true 2>/dev/null; then
    echo "==> network namespace: yes (unshare -rn), loopback only"
    AIRGAP_NETNS=1 unshare -rn "$0" --inner "$T" --models "$MODELS" --wav "$WAV" \
        --expect-text "$EXPECT_TEXT" --port "$PORT" || rc=$?
else
    echo "==> network namespace: not available here (unprivileged user namespaces are"
    echo "    restricted); running on loopback only, audited with strace"
    inner "$T" || rc=$?
fi
[ "$rc" = 0 ] && echo "==> RESULT: PASS" || echo "==> RESULT: FAIL ($rc)"
exit "$rc"
