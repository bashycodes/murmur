#!/usr/bin/env bash
# Build and install the self-hosted flow-server (Parakeet ASR) with its
# systemd unit. Setup guide: docs/server-setup.md.
#
#   tools/server/install.sh [--system | --user] [options]
#
# Modes:
#   --system (default)  /opt/flow-server/{bin,lib,models}, config + token in
#                       /etc/flow-server, the hardened system unit from
#                       crates/flow-server/dist/flow-server.service
#                       (DynamicUser, IP allowlist). Needs sudo; every root
#                       command is printed and confirmed first.
#   --user              ~/.local/{bin,lib/flow-server,share/flow-server/models},
#                       ~/.config/flow-server, a systemd *user* unit. No sudo.
#
# Options:
#   --prefix DIR        install root (default /opt/flow-server or ~/.local)
#   --model DIR         Parakeet model dir (default $FLOW_PARAKEET_DIR, else
#                       models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8)
#   --sherpa-lib DIR    dir with libsherpa-onnx-c-api.so + libonnxruntime.so
#                       (default $SHERPA_ONNX_LIB_DIR)
#   --bind ADDR         listen address, repeatable (default 127.0.0.1:7313 plus
#                       this machine's Tailscale IPv4 on the same port, if any)
#   --port N            port for the default bind addresses (default 7313)
#   --threads N         ASR threads (default 4)
#   --no-build          install the already built target/release/flow-server
#   --start             enable and start the service at the end (otherwise
#                       the commands are printed)
#   --dry-run           print every step, change nothing (no build either)
#   --yes               do not ask before running the sudo commands
#
# Never downloads anything: fetch the model first (tools/fetch-models.sh
# --parakeet) and build sherpa-onnx (or point --sherpa-lib at its libs).
# Existing config and token files are never overwritten.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE=system
PREFIX=""
MODEL="${FLOW_PARAKEET_DIR:-$ROOT/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8}"
SHERPA_LIB="${SHERPA_ONNX_LIB_DIR:-}"
BINDS=()
PORT=7313
THREADS=4
BUILD=1
START=0
DRY=0
YES=0

die() { echo "install.sh: $*" >&2; exit 1; }
usage() { sed -n '2,34p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --system) MODE=system ;;
        --user) MODE=user ;;
        --prefix) PREFIX="${2:?--prefix DIR}"; shift ;;
        --model) MODEL="${2:?--model DIR}"; shift ;;
        --sherpa-lib) SHERPA_LIB="${2:?--sherpa-lib DIR}"; shift ;;
        --bind) BINDS+=("${2:?--bind ADDR}"); shift ;;
        --port) PORT="${2:?--port N}"; shift ;;
        --threads) THREADS="${2:?--threads N}"; shift ;;
        --no-build) BUILD=0 ;;
        --start) START=1 ;;
        --dry-run) DRY=1 ;;
        --yes) YES=1 ;;
        -h|--help) usage 0 ;;
        *) echo "unknown option: $1" >&2; usage 2 ;;
    esac
    shift
done

if [ "$MODE" = system ]; then
    PREFIX="${PREFIX:-/opt/flow-server}"
    BIN_DIR="$PREFIX/bin"
    LIB_DIR="$PREFIX/lib"
    MODEL_ROOT="$PREFIX/models"
    CONF_DIR=/etc/flow-server
    UNIT_PATH=/etc/systemd/system/flow-server.service
    SYSTEMCTL=(systemctl)
    JOURNAL="journalctl -u flow-server -f"
else
    PREFIX="${PREFIX:-$HOME/.local}"
    BIN_DIR="$PREFIX/bin"
    LIB_DIR="$PREFIX/lib/flow-server"
    MODEL_ROOT="$PREFIX/share/flow-server/models"
    CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/flow-server"
    UNIT_PATH="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/flow-server.service"
    SYSTEMCTL=(systemctl --user)
    JOURNAL="journalctl --user -u flow-server -f"
fi
MODEL_NAME="$(basename "$MODEL")"
MODEL_DST="$MODEL_ROOT/$MODEL_NAME"
CONF="$CONF_DIR/config.toml"
TOKEN="$CONF_DIR/token"
BIN="$BIN_DIR/flow-server"

# ---- preflight (read-only) ---------------------------------------------------
[ -f "$MODEL/tokens.txt" ] || die "no Parakeet model in $MODEL (tools/fetch-models.sh --parakeet, or --model DIR)"
[ -n "$SHERPA_LIB" ] || die "set SHERPA_ONNX_LIB_DIR or pass --sherpa-lib DIR (the sherpa-onnx shared libraries)"
for l in libsherpa-onnx-c-api.so libonnxruntime.so; do
    [ -e "$SHERPA_LIB/$l" ] || die "$SHERPA_LIB/$l not found"
done
# --user: `systemctl --user` needs a running user manager and XDG_RUNTIME_DIR
# pointing at its runtime dir. Shells without a login session (ssh without
# pam_systemd, cron, agents) or with XDG_RUNTIME_DIR redirected elsewhere
# (e.g. rootless docker) get "Failed to connect to bus" only at daemon-reload,
# after everything is built and copied. Check here, fall back to the standard
# runtime dir, else stop with the fix. FLOW_INSTALL_RUN_USER: tests only.
if [ "$MODE" = user ] && ! systemctl --user show-environment >/dev/null 2>&1; then
    RUN_DIR="${FLOW_INSTALL_RUN_USER:-/run/user}/$(id -u)"
    if [ -S "$RUN_DIR/bus" ] && XDG_RUNTIME_DIR="$RUN_DIR" DBUS_SESSION_BUS_ADDRESS="unix:path=$RUN_DIR/bus" \
        systemctl --user show-environment >/dev/null 2>&1; then
        echo "note: no user bus at XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-<unset>}; using $RUN_DIR" >&2
        export XDG_RUNTIME_DIR="$RUN_DIR" DBUS_SESSION_BUS_ADDRESS="unix:path=$RUN_DIR/bus"
        JOURNAL="XDG_RUNTIME_DIR=$RUN_DIR $JOURNAL"
    else
        msg="no systemd user manager for ${USER:-$(id -un)} (systemctl --user cannot connect). Run 'loginctl enable-linger ${USER:-$(id -un)}' (no sudo needed for yourself on most distros) or log in on a console/desktop session, then run this again"
        if [ "$DRY" = 1 ]; then echo "warning: $msg" >&2; else die "$msg"; fi
    fi
fi
if [ ${#BINDS[@]} -eq 0 ]; then
    BINDS=("127.0.0.1:$PORT")
    TS_IP=""
    if command -v tailscale >/dev/null 2>&1; then
        TS_IP="$(tailscale ip -4 2>/dev/null | head -n 1 || true)"
    fi
    if [ -n "$TS_IP" ]; then
        BINDS+=("$TS_IP:$PORT")
    else
        echo "note: no Tailscale IPv4 found; binding loopback only (add --bind <tailscale-ip>:$PORT)" >&2
    fi
fi
BIND_TOML="$(printf '"%s", ' "${BINDS[@]}")"
BIND_TOML="[${BIND_TOML%, }]"

# ---- helpers -------------------------------------------------------------------
step() { printf '\n==> %s\n' "$*"; }
# run CMD...: as the current user.
run() {
    printf '    $ %s\n' "$*"
    [ "$DRY" = 1 ] || "$@"
}
# priv CMD...: as root in --system mode (via sudo), as the user in --user mode.
priv() {
    if [ "$MODE" = system ]; then
        printf '    [sudo] %s\n' "$*"
        [ "$DRY" = 1 ] || sudo "$@"
    else
        run "$@"
    fi
}
# priv_write PATH MODE: write stdin to PATH (root-owned in --system mode).
priv_write() {
    local path="$1" mode="$2" tmp
    tmp="$(mktemp)"
    cat >"$tmp"
    if [ "$DRY" = 1 ]; then
        printf '    (would write %s, mode %s:)\n' "$path" "$mode"
        sed 's/^/      | /' "$tmp"
    else
        priv install -m "$mode" "$tmp" "$path"
    fi
    rm -f "$tmp"
}
exists() {
    if [ "$MODE" = system ]; then sudo test -e "$1"; else test -e "$1"; fi
}

cat <<EOF
flow-server install ($MODE mode)$( [ "$DRY" = 1 ] && echo ", DRY RUN: nothing is changed" )
  binary   $BIN
  libs     $LIB_DIR (libsherpa-onnx-c-api.so, libonnxruntime.so)
  model    $MODEL_DST
  config   $CONF
  token    $TOKEN
  unit     $UNIT_PATH
  bind     ${BINDS[*]}
EOF

if [ "$MODE" = system ] && [ "$DRY" = 0 ] && [ "$YES" = 0 ]; then
    cat <<EOF

The system install runs these as root (sudo): create $PREFIX and
$CONF_DIR, copy the binary, libraries and model there, write the config,
token and $UNIT_PATH, and run systemctl daemon-reload.
EOF
    read -r -p "Continue? [y/N] " ok
    [ "$ok" = y ] || [ "$ok" = Y ] || die "aborted"
fi

# ---- 1. build ------------------------------------------------------------------
step "1. build flow-server (release, Parakeet + TLS)"
if [ "$BUILD" = 1 ]; then
    # Never downloads: SHERPA_ONNX_LIB_DIR points sherpa-onnx-sys at the local libs.
    run env SHERPA_ONNX_LIB_DIR="$SHERPA_LIB" cargo build --release --offline \
        --manifest-path "$ROOT/Cargo.toml" -p flow-server --no-default-features --features parakeet,tls
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BUILT="$TARGET_DIR/release/flow-server"
[ "$DRY" = 1 ] || [ -x "$BUILT" ] || die "$BUILT not built"

# ---- 2. files ------------------------------------------------------------------
step "2. install binary, libraries and model"
priv mkdir -p "$BIN_DIR" "$LIB_DIR" "$MODEL_ROOT" "$CONF_DIR"
priv install -m 0755 "$BUILT" "$BIN"
for l in libsherpa-onnx-c-api.so libonnxruntime.so; do
    priv install -m 0644 "$(readlink -f "$SHERPA_LIB/$l")" "$LIB_DIR/$l"
done
priv cp -rT "$MODEL" "$MODEL_DST"
[ "$MODE" = system ] && priv chmod -R a+rX "$PREFIX"
[ "$MODE" = system ] && priv chmod 0755 "$CONF_DIR"
[ "$MODE" = user ] && run chmod 0700 "$CONF_DIR"

# ---- 3. token ------------------------------------------------------------------
step "3. token"
if [ "$DRY" = 0 ] && exists "$TOKEN"; then
    echo "    $TOKEN exists, kept"
else
    priv env LD_LIBRARY_PATH="$LIB_DIR" "$BIN" gen-token --out "$TOKEN"
fi

# ---- 4. config -----------------------------------------------------------------
step "4. config"
if [ "$DRY" = 0 ] && exists "$CONF"; then
    echo "    $CONF exists, kept (compare with crates/flow-server/dist/config.example.toml)"
else
    priv_write "$CONF" 0644 <<EOF
# flow-server config, written by tools/server/install.sh. Reference:
# crates/flow-server/dist/config.example.toml, docs/server-setup.md.
# Loopback / LAN / Tailscale addresses only: 0.0.0.0 or a public address is
# refused unless [tls] is set.
bind = $BIND_TOML
token_file = "$TOKEN"
workers = 1
max_queue = 2

[model]
asr_engine = "parakeet"
asr_path = "$MODEL_DST"
threads = $THREADS

# [tls]                # optional wss:// (docs/server-setup.md "TLS")
# cert = "$CONF_DIR/cert.pem"
# key = "$CONF_DIR/key.pem"

[log]
level = "info"
log_text = false
EOF
fi
priv env LD_LIBRARY_PATH="$LIB_DIR" "$BIN" check-config --config "$CONF"

# ---- 5. systemd unit -----------------------------------------------------------
step "5. systemd unit"
SRC_UNIT="$ROOT/crates/flow-server/dist/flow-server.service"
if [ "$MODE" = system ]; then
    # The packaged hardened unit, with this install's paths.
    sed -e '/^# Install (manually/,/enable --now flow-server`\.$/c\# Installed by tools/server/install.sh (see docs/server-setup.md).' \
        -e "s|/usr/local/bin/flow-server|$BIN|" \
        -e "s|^LoadCredential=token:.*|LoadCredential=token:$TOKEN|" \
        -e "s|^# Environment=LD_LIBRARY_PATH=.*|Environment=LD_LIBRARY_PATH=$LIB_DIR|" \
        -e "s|--config /etc/flow-server/config.toml|--config $CONF|" \
        "$SRC_UNIT" | priv_write "$UNIT_PATH" 0644
else
    priv mkdir -p "$(dirname "$UNIT_PATH")"
    priv_write "$UNIT_PATH" 0644 <<EOF
# flow-server as a systemd user unit (tools/server/install.sh --user).
# The system unit (crates/flow-server/dist/flow-server.service) adds
# DynamicUser and a kernel IP allowlist; here the server's own bind policy
# and peer filter (loopback / LAN / Tailscale only) are the boundary.
[Unit]
Description=flow-server (self-hosted dictation ASR; LAN/Tailscale only)
After=network-online.target

[Service]
Type=simple
ExecStart=$BIN --config $CONF
Environment=LD_LIBRARY_PATH=$LIB_DIR
Restart=on-failure
RestartSec=3
Nice=5
MemoryMax=8G
NoNewPrivileges=yes
PrivateTmp=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX

[Install]
WantedBy=default.target
EOF
fi
priv "${SYSTEMCTL[@]}" daemon-reload

# ---- 6. start ------------------------------------------------------------------
step "6. start"
SUDO=""
[ "$MODE" = system ] && SUDO="sudo "
if [ "$START" = 1 ]; then
    priv "${SYSTEMCTL[@]}" enable --now flow-server
else
    echo "    not started (pass --start). To start now and at boot:"
    echo "      ${SUDO}${SYSTEMCTL[*]} enable --now flow-server"
    [ "$MODE" = user ] && echo "      sudo loginctl enable-linger $USER   # keep it running while logged out"
fi

cat <<EOF

Next steps (docs/server-setup.md):
  * logs (expect "ready workers=1 asr=parakeet"):  ${SUDO}$JOURNAL
  * the token for the apps:                        ${SUDO}cat $TOKEN
  * apps, Developer settings: Allow server on, Server host = one of
    ${BINDS[*]} (host part; the Tailscale 100.x address from other devices),
    port $PORT, TLS off on Tailscale, token as above, then "Test connection".
EOF
