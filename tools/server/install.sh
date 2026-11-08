#!/usr/bin/env bash
# Build and install the self-hosted flow-server (Parakeet ASR) with its
# systemd unit, or upgrade an existing install in place (binary and libs
# replaced; config, token, keys and models kept). Setup guide:
# docs/server-advanced.md.
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
#                       the existing config's asr_path when it holds a
#                       model (upgrade in place), else
#                       models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8)
#   --sherpa-lib DIR    dir with libsherpa-onnx-c-api.so + libonnxruntime.so
#                       (default $SHERPA_ONNX_LIB_DIR, else the install's
#                       lib dir when it holds both (upgrade in place))
#   --bind ADDR         listen address, repeatable (default 127.0.0.1:7313 plus
#                       this machine's first VPN IPv4 in 100.64.0.0/10 on the
#                       same port, if any)
#   --port N            port for the default bind addresses (default 7313)
#   --threads N         ASR threads (default 4)
#   --llm               also build LLM cleanup: flow-server with the `llm`
#                       feature plus the llama.cpp plugin libflow_llm.so,
#                       installed next to the sherpa libs. Turned on by
#                       itself when the existing config sets llm_path (a
#                       build without the feature refuses to start then)
#   --llm-model FILE    GGUF for LLM cleanup (implies --llm): copied next to
#                       the ASR model and written as llm_path into a *new*
#                       config (an existing config is never edited)
#   --no-whisper        leave out the `whisper` feature (whisper.cpp: the
#                       whisper engine, lid_path language ID, vad_model_path).
#                       Ignored when the existing config uses any of them (a
#                       build without it refuses to start then)
#   --no-build          install the already built target/release/flow-server
#   --start             enable and (re)start the service at the end, so an
#                       upgrade runs the new binary (otherwise the commands
#                       are printed)
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
MODEL="${FLOW_PARAKEET_DIR:-}"
SHERPA_LIB="${SHERPA_ONNX_LIB_DIR:-}"
BINDS=()
PORT=7313
THREADS=4
BUILD=1
LLM=0
LLM_MODEL=""
WHISPER=1
START=0
DRY=0
YES=0

die() { echo "install.sh: $*" >&2; exit 1; }
usage() { sed -n '2,53p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

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
        --llm) LLM=1 ;;
        --llm-model) LLM_MODEL="${2:?--llm-model FILE}"; LLM=1; shift ;;
        --no-whisper) WHISPER=0 ;;
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
CONF="$CONF_DIR/config.toml"
TOKEN="$CONF_DIR/token"
BIN="$BIN_DIR/flow-server"
PLUGIN="$LIB_DIR/libflow_llm.so"

# ---- preflight (read-only) ---------------------------------------------------
# An existing config (re-install / upgrade) decides what the build needs;
# it is never edited.
CONF_LLM_PATH=""
CONF_LLM_LIB=""
CONF_ASR_PATH=""
CONF_WHISPER=""
if [ -r "$CONF" ]; then
    toml_str() { sed -n -E "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\"([^\"]*)\".*/\1/p" "$CONF" | head -n 1; }
    CONF_LLM_PATH="$(toml_str llm_path)"
    CONF_LLM_LIB="$(toml_str llm_lib)"
    CONF_ASR_PATH="$(toml_str asr_path)"
    # whisper.cpp users: language ID (D34), the Silero VAD, a whisper engine.
    for k in lid_path vad_model_path; do
        [ -n "$(toml_str "$k")" ] && CONF_WHISPER="$CONF_WHISPER $k"
    done
    grep -q -E '^[[:space:]]*asr_(engine|backend)[[:space:]]*=[[:space:]]*"whisper"' "$CONF" \
        && CONF_WHISPER="$CONF_WHISPER asr_engine=whisper"
fi
# Upgrade in place: without --model, keep serving the installed model.
if [ -z "$MODEL" ] && [ -n "$CONF_ASR_PATH" ] && [ -f "$CONF_ASR_PATH/tokens.txt" ]; then
    echo "note: upgrading in place; model from $CONF: $CONF_ASR_PATH" >&2
    MODEL="$CONF_ASR_PATH"
fi
MODEL="${MODEL:-$ROOT/models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8}"
MODEL_NAME="$(basename "$MODEL")"
MODEL_DST="$MODEL_ROOT/$MODEL_NAME"
[ -f "$MODEL/tokens.txt" ] || die "no Parakeet model in $MODEL (tools/fetch-models.sh --parakeet, or --model DIR)"
# Upgrade in place: without --sherpa-lib, keep serving the installed libs.
if [ -z "$SHERPA_LIB" ] && [ -e "$LIB_DIR/libsherpa-onnx-c-api.so" ] && [ -e "$LIB_DIR/libonnxruntime.so" ]; then
    echo "note: upgrading in place; sherpa libs from $LIB_DIR" >&2
    SHERPA_LIB="$LIB_DIR"
fi
[ -n "$SHERPA_LIB" ] || die "set SHERPA_ONNX_LIB_DIR or pass --sherpa-lib DIR (the sherpa-onnx shared libraries)"
for l in libsherpa-onnx-c-api.so libonnxruntime.so; do
    [ -e "$SHERPA_LIB/$l" ] || die "$SHERPA_LIB/$l not found"
done
# A config that needs a feature the build leaves out stops flow-server at
# startup ("lid_path set but built without the `whisper` feature", same for
# llm_path), so a plain re-install would take the server down.
if [ -n "$CONF_WHISPER" ] && [ "$WHISPER" = 0 ]; then
    echo "note: $CONF uses${CONF_WHISPER}; building with whisper despite --no-whisper" >&2
    WHISPER=1
fi
if [ -n "$CONF_LLM_PATH" ] && [ "$LLM" = 0 ]; then
    echo "note: $CONF sets llm_path; building with LLM cleanup (as --llm)" >&2
    LLM=1
fi
if [ -n "$LLM_MODEL" ]; then
    [ -f "$LLM_MODEL" ] || die "--llm-model $LLM_MODEL: no such file"
    LLM_MODEL_DST="$MODEL_ROOT/$(basename "$LLM_MODEL")"
fi
if [ -n "$CONF_LLM_LIB" ] && [ "$CONF_LLM_LIB" != "$PLUGIN" ]; then
    echo "warning: $CONF has llm_lib = \"$CONF_LLM_LIB\"; this install puts the plugin at $PLUGIN, which that config does not load" >&2
fi
# bindgen (llama-cpp-sys-2) runs libclang, which does not find the compiler's
# own headers (stdbool.h) on some distros (Ubuntu with gcc 13). Point it at
# gcc's include dir unless the caller set BINDGEN_EXTRA_CLANG_ARGS already.
BINDGEN_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:-}"
if [ "$LLM" = 1 ] && [ -z "$BINDGEN_ARGS" ] && command -v gcc >/dev/null 2>&1; then
    GCC_INC="$(gcc -print-file-name=include 2>/dev/null || true)"
    [ -f "$GCC_INC/stdbool.h" ] && BINDGEN_ARGS="-I$GCC_INC"
fi
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
    # A VPN overlay address (100.64.0.0/10, the range mesh VPNs hand out)
    # on any interface; LAN and other addresses need an explicit --bind.
    VPN_IP=""
    if command -v ip >/dev/null 2>&1; then
        VPN_IP="$(ip -4 -o addr show scope global 2>/dev/null | awk '{
            split($4, a, "/"); split(a[1], o, ".")
            if (o[1] == 100 && o[2] >= 64 && o[2] <= 127) { print a[1]; exit }
        }' || true)"
    fi
    if [ -n "$VPN_IP" ]; then
        BINDS+=("$VPN_IP:$PORT")
    else
        echo "note: no VPN (100.64.0.0/10) IPv4 found; binding loopback only (add --bind <LAN-or-VPN-ip>:$PORT)" >&2
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
  whisper  $( if [ "$WHISPER" = 1 ]; then echo "on (language ID, VAD, whisper engine)"; else echo "off (--no-whisper)"; fi )
  llm      $( if [ "$LLM" = 1 ]; then echo "on: plugin $PLUGIN${LLM_MODEL:+, model ${LLM_MODEL_DST:-}}"; else echo "off (--llm to build cleanup)"; fi )
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
FEATURES=parakeet,tls
[ "$WHISPER" = 1 ] && FEATURES=whisper,$FEATURES
[ "$LLM" = 1 ] && FEATURES=$FEATURES,llm
step "1. build flow-server (release, features $FEATURES)"
if [ "$BUILD" = 1 ]; then
    # Never downloads: SHERPA_ONNX_LIB_DIR points sherpa-onnx-sys at the local libs.
    run env SHERPA_ONNX_LIB_DIR="$SHERPA_LIB" cargo build --release --offline \
        --manifest-path "$ROOT/Cargo.toml" -p flow-server --no-default-features --features "$FEATURES"
    # The `llm` feature only adds the loader; llama.cpp is the separate
    # flow-llm cdylib (its ggml clashes with whisper.cpp's in one binary).
    if [ "$LLM" = 1 ]; then
        run env BINDGEN_EXTRA_CLANG_ARGS="$BINDGEN_ARGS" cargo build --release --offline \
            --manifest-path "$ROOT/Cargo.toml" -p flow-llm
    fi
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BUILT="$TARGET_DIR/release/flow-server"
BUILT_PLUGIN="$TARGET_DIR/release/libflow_llm.so"
[ "$DRY" = 1 ] || [ -x "$BUILT" ] || die "$BUILT not built"
[ "$DRY" = 1 ] || [ "$LLM" = 0 ] || [ -f "$BUILT_PLUGIN" ] || die "$BUILT_PLUGIN not built"

# ---- 2. files ------------------------------------------------------------------
step "2. install binary, libraries and model"
priv mkdir -p "$BIN_DIR" "$LIB_DIR" "$MODEL_ROOT" "$CONF_DIR"
# Binary and libs go in as one set: each file is staged next to its target
# as <name>.new, and only once all are staged are they renamed over the old
# ones, libs first and the binary last. A failed copy leaves the previous set
# untouched (a restart would otherwise load a new binary against old libs).
# A source that is the installed file itself (--sherpa-lib naming the
# install's lib dir) is kept: `install` refuses to copy a file onto itself.
STAGED=()
stage() { # SRC DST MODE
    if [ "$1" -ef "$2" ]; then
        echo "    $2 is the source, kept"
        return 0
    fi
    if ! priv install -m "$3" "$1" "$2.new"; then
        [ ${#STAGED[@]} -eq 0 ] || priv rm -f "${STAGED[@]/%/.new}"
        priv rm -f "$2.new"
        die "copying $1 failed; the installed binary and libraries are unchanged"
    fi
    STAGED+=("$2")
}
for l in libsherpa-onnx-c-api.so libonnxruntime.so; do
    stage "$(readlink -f "$SHERPA_LIB/$l")" "$LIB_DIR/$l" 0644
done
if [ "$LLM" = 1 ]; then stage "$BUILT_PLUGIN" "$PLUGIN" 0644; fi
stage "$BUILT" "$BIN" 0755
for f in ${STAGED[@]+"${STAGED[@]}"}; do
    priv mv -f "$f.new" "$f"
done
# --model may name the installed copy itself (re-install in place).
if [ "$(readlink -f "$MODEL")" = "$(readlink -f "$MODEL_DST" 2>/dev/null || true)" ]; then
    echo "    $MODEL_DST is the source model, kept"
else
    priv cp -rT "$MODEL" "$MODEL_DST"
fi
if [ -n "$LLM_MODEL" ]; then
    if [ "$(readlink -f "$LLM_MODEL")" = "$(readlink -f "$LLM_MODEL_DST" 2>/dev/null || true)" ]; then
        echo "    $LLM_MODEL_DST is the source GGUF, kept"
    else
        priv install -m 0644 "$LLM_MODEL" "$LLM_MODEL_DST"
    fi
fi
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
# crates/flow-server/dist/config.example.toml, docs/server-advanced.md.
# Loopback / LAN / VPN addresses only: 0.0.0.0 or a public address is
# refused unless [tls] is set.
bind = $BIND_TOML
token_file = "$TOKEN"
workers = 1
max_queue = 2

[model]
asr_engine = "parakeet"
asr_path = "$MODEL_DST"
threads = $THREADS
$( if [ -n "$LLM_MODEL" ]; then
    printf 'llm_path = "%s"\nllm_lib = "%s"\n' "$LLM_MODEL_DST" "$PLUGIN"
else
    printf '# llm_path = "%s/<model>.gguf"   # LLM cleanup, needs --llm\n# llm_lib = "%s"\n' "$MODEL_ROOT" "$PLUGIN"
fi )

# [tls]                # optional wss:// (docs/server-advanced.md "TLS")
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
    sed -e '/^# Install (manually/,/enable --now flow-server`\.$/c\# Installed by tools/server/install.sh (see docs/server-advanced.md).' \
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
# and peer filter (loopback / LAN / VPN only) are the boundary.
[Unit]
Description=flow-server (self-hosted dictation ASR; LAN/VPN only)
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
    # restart, not `enable --now`: an upgrade must replace the running binary.
    priv "${SYSTEMCTL[@]}" enable flow-server
    priv "${SYSTEMCTL[@]}" restart flow-server
else
    echo "    not started (pass --start). To (re)start now and enable at boot:"
    echo "      ${SUDO}${SYSTEMCTL[*]} enable flow-server && ${SUDO}${SYSTEMCTL[*]} restart flow-server"
    [ "$MODE" = user ] && echo "      sudo loginctl enable-linger $USER   # keep it running while logged out"
fi

cat <<EOF

Next steps (docs/server-advanced.md):
  * logs (expect "ready workers=1 asr=parakeet"):  ${SUDO}$JOURNAL
  * LLM cleanup loaded: GET /v1/health with the token shows "llm":true
  * the token for the apps:                        ${SUDO}cat $TOKEN
    (that is the admin key "owner"; one key per device:
     ${SUDO}env LD_LIBRARY_PATH=$LIB_DIR $BIN keys add <name> --config $CONF)
  * apps, Developer settings: Allow server on, Server host = one of
    ${BINDS[*]} (host part; the LAN or VPN address from other devices),
    port $PORT, TLS off on a VPN or trusted LAN, token as above, then "Test connection".
EOF
