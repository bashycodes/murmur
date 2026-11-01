#!/usr/bin/env bash
# Tests for install.sh --user: the preflight of the systemd user manager,
# LLM cleanup (--llm, --llm-model, auto-on from an existing config's
# llm_path, config and token never overwritten), the whisper feature
# (--no-whisper, forced back on by lid_path & co.) and an in-place upgrade.
# No systemd, model or build needed: a stub `systemctl` on PATH answers only
# when $XDG_RUNTIME_DIR/bus is a socket, a fake /run/user root is passed
# through FLOW_INSTALL_RUN_USER, and real (non-dry) runs use --no-build with
# a stub flow-server and plugin in a fake CARGO_TARGET_DIR.
#
#   tools/server/test-install.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

mkdir -p "$T/bin" "$T/model" "$T/lib" "$T/run/$(id -u)" "$T/elsewhere" "$T/home" "$T/target/release"
export CARGO_TARGET_DIR="$T/target"
touch "$T/model/tokens.txt" "$T/lib/libsherpa-onnx-c-api.so" "$T/lib/libonnxruntime.so"
cat >"$T/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
[ -S "${XDG_RUNTIME_DIR:-/nonexistent}/bus" ] && exit 0
echo "Failed to connect to bus: No such file or directory" >&2
exit 1
EOF
chmod +x "$T/bin/systemctl"
mkbus() { python3 -c 'import socket,sys; socket.socket(socket.AF_UNIX).bind(sys.argv[1])' "$1"; }

# install.sh with a sandboxed PATH/HOME; prints combined output, exit code last.
inst() {
    local rc=0 out
    out="$(env -u DBUS_SESSION_BUS_ADDRESS PATH="$T/bin:$PATH" HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
        FLOW_INSTALL_RUN_USER="$T/run" XDG_RUNTIME_DIR="$1" \
        bash "$HERE/install.sh" --user --model "$T/model" --sherpa-lib "$T/lib" \
        --bind 127.0.0.1:7313 "${@:2}" 2>&1)" || rc=$?
    printf '%s\nrc=%s\n' "$out" "$rc"
}
fails=0
check() { # NAME OUTPUT PATTERN [!]
    if { [ "${4:-}" = "!" ] && ! grep -q -- "$3" <<<"$2"; } || { [ "${4:-}" != "!" ] && grep -q -- "$3" <<<"$2"; }; then
        echo "ok   $1"
    else
        echo "FAIL $1: expected ${4:-}'$3' in:"; printf '    | %s\n' "${2//$'\n'/$'\n'    | }"; fails=$((fails + 1))
    fi
}

# 1. No user manager anywhere: a real run stops before building, with the fix.
o="$(inst "$T/elsewhere")"
check "no manager: exits 1" "$o" "rc=1"
check "no manager: names enable-linger" "$o" "loginctl enable-linger"
check "no manager: nothing built" "$o" "1. build" "!"
# ... and a dry run only warns.
o="$(inst "$T/elsewhere" --dry-run)"
check "no manager, dry run: warns" "$o" "warning: no systemd user manager"
check "no manager, dry run: completes" "$o" "rc=0"

# 2. XDG_RUNTIME_DIR redirected, but /run/user/UID/bus is up: falls back.
mkbus "$T/run/$(id -u)/bus"
o="$(inst "$T/elsewhere" --dry-run)"
check "fallback: note" "$o" "using $T/run/$(id -u)"
check "fallback: no warning" "$o" "warning:" "!"
check "fallback: completes" "$o" "rc=0"

# 3. XDG_RUNTIME_DIR already right: silent.
o="$(inst "$T/run/$(id -u)" --dry-run)"
check "good env: no note" "$o" "note: no user bus" "!"
check "good env: completes" "$o" "rc=0"

# 4. --llm: llm feature, the flow-llm plugin build and install, bindgen's
# include path (from gcc, unless the caller set it).
R="$T/run/$(id -u)"
CONF="$T/home/.config/flow-server/config.toml"
o="$(inst "$R" --dry-run)"
check "no llm: whisper on by default" "$o" "--features whisper,parakeet,tls\$"
check "no llm: no plugin build" "$o" "-p flow-llm" "!"
check "no llm: summary says off" "$o" "llm      off"
o="$(inst "$R" --dry-run --llm)"
check "--llm: llm feature" "$o" "--features whisper,parakeet,tls,llm"
check "--llm: plugin build" "$o" "cargo build --release --offline .* -p flow-llm"
check "--llm: plugin installed" "$o" "install -m 0644 $T/target/release/libflow_llm.so $T/home/.local/lib/flow-server/libflow_llm.so.new"
check "--llm: new config has commented llm_path" "$o" "| # llm_path = "
if command -v gcc >/dev/null 2>&1 && [ -f "$(gcc -print-file-name=include)/stdbool.h" ]; then
    check "--llm: bindgen gets gcc include" "$o" "BINDGEN_EXTRA_CLANG_ARGS=-I$(gcc -print-file-name=include) "
fi
o="$(BINDGEN_EXTRA_CLANG_ARGS="-I/custom" inst "$R" --dry-run --llm)"
check "--llm: caller's bindgen args kept" "$o" "BINDGEN_EXTRA_CLANG_ARGS=-I/custom "

# 5. An existing config decides: llm_path set turns --llm on by itself (a
# build without the feature would not start); commented out it does not.
mkdir -p "$(dirname "$CONF")"
printf '[model]\n# llm_path = "/x.gguf"\n' >"$CONF"
o="$(inst "$R" --dry-run)"
check "commented llm_path: stays off" "$o" "--features whisper,parakeet,tls\$"
printf '[model]\nllm_path = "/m/q.gguf"\nllm_lib = "%s"\n' "$T/home/.local/lib/flow-server/libflow_llm.so" >"$CONF"
o="$(inst "$R" --dry-run)"
check "config llm_path: note" "$o" "sets llm_path; building with LLM cleanup"
check "config llm_path: llm feature" "$o" "--features whisper,parakeet,tls,llm"
check "config llm_path: matching llm_lib, no warning" "$o" "warning:" "!"
printf '[model]\n  llm_path = "/m/q.gguf"\nllm_lib = "/elsewhere/libflow_llm.so"\n' >"$CONF"
o="$(inst "$R" --dry-run)"
check "config llm_path (indented): llm feature" "$o" "--features whisper,parakeet,tls,llm"
check "other llm_lib: warning" "$o" "warning: .*llm_lib = \"/elsewhere/libflow_llm.so\""

# 6. Real runs (--no-build, stub binary + plugin): existing config and token
# are kept byte for byte, the plugin is installed; a missing plugin stops.
cat >"$T/target/release/flow-server" <<'EOF'
#!/usr/bin/env bash
case "$1" in
    gen-token) echo "stub-token" >"$3" ;;
    check-config) echo "config ok" ;;
esac
EOF
chmod +x "$T/target/release/flow-server"
printf '[model]\nllm_path = "/m/q.gguf"\n# user edit\n' >"$CONF"
echo "user-token" >"$T/home/.config/flow-server/token"
cp "$CONF" "$T/conf.before"
o="$(inst "$R" --no-build)"
check "real, no plugin built: exits 1" "$o" "rc=1"
check "real, no plugin built: names it" "$o" "libflow_llm.so not built"
echo plugin >"$T/target/release/libflow_llm.so"
o="$(inst "$R" --no-build)"
check "real: completes" "$o" "rc=0"
check "real: config kept" "$o" "config.toml exists, kept"
check "real: token kept" "$o" "token exists, kept"
check "real: config unchanged" "$(cmp "$T/conf.before" "$CONF" && echo same)" "same"
check "real: token unchanged" "$(cat "$T/home/.config/flow-server/token")" "^user-token$"
check "real: plugin installed" "$(cat "$T/home/.local/lib/flow-server/libflow_llm.so" 2>&1)" "^plugin$"

# 7. Fresh install with --llm-model: the GGUF is copied next to the ASR
# model and the new config points at it and at the plugin.
rm -rf "${T:?}/home"
echo gguf >"$T/q.gguf"
o="$(inst "$R" --no-build --llm-model "$T/q.gguf")"
M="$T/home/.local/share/flow-server/models"
check "--llm-model: completes" "$o" "rc=0"
check "--llm-model: gguf copied" "$(cat "$M/q.gguf" 2>&1)" "^gguf$"
check "--llm-model: config llm_path" "$(cat "$CONF")" "^llm_path = \"$M/q.gguf\"$"
check "--llm-model: config llm_lib" "$(cat "$CONF")" "^llm_lib = \"$T/home/.local/lib/flow-server/libflow_llm.so\"$"
check "--llm-model: token generated" "$(cat "$T/home/.config/flow-server/token")" "^stub-token$"
o="$(inst "$R" --dry-run --llm-model "$T/nope.gguf")"
check "--llm-model missing file: exits 1" "$o" "rc=1"

# 8. No --bind: loopback plus the first VPN (100.64/10) IPv4 that a stub
# `ip -4 -o addr` reports; 100.128.x (public) and LAN addresses are skipped.
cat >"$T/bin/ip" <<'EOF'
#!/usr/bin/env bash
cat "$IP_ADDRS"
EOF
chmod +x "$T/bin/ip"
nobind() {
    local rc=0 out
    out="$(env -u DBUS_SESSION_BUS_ADDRESS PATH="$T/bin:$PATH" HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
        FLOW_INSTALL_RUN_USER="$T/run" XDG_RUNTIME_DIR="$R" IP_ADDRS="$1" \
        bash "$HERE/install.sh" --user --model "$T/model" --sherpa-lib "$T/lib" --dry-run 2>&1)" || rc=$?
    printf '%s\nrc=%s\n' "$out" "$rc"
}
cat >"$T/ip-vpn" <<'EOF'
2: eno1    inet 192.168.1.5/24 brd 192.168.1.255 scope global eno1\       valid_lft forever preferred_lft forever
3: wan0    inet 100.128.0.9/24 scope global wan0\       valid_lft forever preferred_lft forever
5: wg0    inet 100.101.102.103/32 scope global wg0\       valid_lft forever preferred_lft forever
EOF
o="$(nobind "$T/ip-vpn")"
check "no --bind: VPN address bound" "$o" "127.0.0.1:7313 100.101.102.103:7313"
check "no --bind: public 100.128 skipped" "$o" "100.128.0.9:7313" "!"
check "no --bind: completes" "$o" "rc=0"
head -n 1 "$T/ip-vpn" >"$T/ip-lan"
o="$(nobind "$T/ip-lan")"
check "no --bind, no VPN: loopback only" "$o" "note: no VPN (100.64.0.0/10) IPv4 found; binding loopback only"
check "no --bind, no VPN: LAN not bound" "$o" "192.168.1.5:7313" "!"

# 9. whisper: on by default; --no-whisper leaves it out unless the config
# uses whisper.cpp (lid_path, vad_model_path, a whisper engine).
rm -rf "${T:?}/home"
o="$(inst "$R" --dry-run --no-whisper)"
check "--no-whisper: plain features" "$o" "--features parakeet,tls\$"
check "--no-whisper: summary says off" "$o" "whisper  off"
mkdir -p "$(dirname "$CONF")"
for k in lid_path vad_model_path engine; do
    case "$k" in
        engine) line='[model.languages.xx]
asr_engine = "whisper"' ;;
        *) line="$k = \"/m/x.bin\"" ;;
    esac
    printf '[model]\nasr_engine = "parakeet"\n%s\n' "$line" >"$CONF"
    o="$(inst "$R" --dry-run --no-whisper)"
    check "--no-whisper, config $k: whisper forced" "$o" "--features whisper,parakeet,tls\$"
    check "--no-whisper, config $k: note" "$o" "note: .* building with whisper despite --no-whisper"
done
printf '[model]\nasr_engine = "parakeet"\n# lid_path = "/m/x.bin"\n' >"$CONF"
o="$(inst "$R" --dry-run --no-whisper)"
check "--no-whisper, commented lid_path: stays off" "$o" "--features parakeet,tls\$"

# 10. Upgrade in place of the home-server layout: lid_path + llm_path +
# per-language model, installed model, keys. No --model: the config's
# asr_path is reused; config, token, keys, models kept; whisper and llm
# built; the service restarted with --start.
rm -rf "${T:?}/home"
M="$T/home/.local/share/flow-server/models"
P="$M/parakeet-installed"
mkdir -p "$P" "$M/zipformer-he" "$(dirname "$CONF")" "$T/home/.local/state/flow-server"
echo tokens >"$P/tokens.txt"
echo he >"$M/zipformer-he/tokens.txt"
echo lid >"$M/ggml-tiny-q5_1.bin"
echo gguf >"$M/q.gguf"
cat >"$CONF" <<EOC
bind = ["127.0.0.1:7313"]
token_file = "$T/home/.config/flow-server/token"
[model]
asr_engine = "parakeet"
asr_path = "$P"
llm_path = "$M/q.gguf"
llm_lib = "$T/home/.local/lib/flow-server/libflow_llm.so"
lid_path = "$M/ggml-tiny-q5_1.bin"
[model.languages.he]
asr_backend = "sherpa_online"
asr_path = "$M/zipformer-he"
EOC
echo "user-token" >"$T/home/.config/flow-server/token"
echo '{"keys":[]}' >"$T/home/.local/state/flow-server/keys.json"
cp "$CONF" "$T/conf.before"
(cd "$M" && find . -type f -exec sha256sum {} + | sort) >"$T/models.before"
upgrade() {
    local rc=0 out
    out="$(env -u DBUS_SESSION_BUS_ADDRESS PATH="$T/bin:$PATH" HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
        FLOW_INSTALL_RUN_USER="$T/run" XDG_RUNTIME_DIR="$R" FLOW_PARAKEET_DIR= \
        bash "$HERE/install.sh" --user --sherpa-lib "$T/lib" --bind 127.0.0.1:7313 "$@" 2>&1)" || rc=$?
    printf '%s\nrc=%s\n' "$out" "$rc"
}
o="$(upgrade --dry-run)"
check "upgrade: in-place note" "$o" "note: upgrading in place; model from .*: $P"
check "upgrade: whisper + llm build" "$o" "--features whisper,parakeet,tls,llm\$"
check "upgrade: plugin build" "$o" "-p flow-llm"
check "upgrade: summary whisper on" "$o" "whisper  on"
o="$(upgrade --no-build --start)"
check "upgrade: completes" "$o" "rc=0"
check "upgrade: model kept in place" "$o" "$P is the source model, kept"
check "upgrade: config kept" "$o" "config.toml exists, kept"
check "upgrade: token kept" "$o" "token exists, kept"
check "upgrade: new config checked" "$o" "check-config --config $CONF"
check "upgrade: restarted" "$o" "systemctl --user restart flow-server"
check "upgrade: config unchanged" "$(cmp "$T/conf.before" "$CONF" && echo same)" "same"
check "upgrade: token unchanged" "$(cat "$T/home/.config/flow-server/token")" "^user-token$"
check "upgrade: keys unchanged" "$(cat "$T/home/.local/state/flow-server/keys.json")" '^{"keys":\[\]}$'
check "upgrade: models unchanged" "$( (cd "$M" && find . -type f -exec sha256sum {} + | sort) | cmp - "$T/models.before" && echo same)" "same"
check "upgrade: binary replaced" "$(cmp "$T/target/release/flow-server" "$T/home/.local/bin/flow-server" && echo same)" "same"
# An asr_path without a model is not reused: the usual error.
sed -i "s|^asr_path = \"$P\"|asr_path = \"$T/nowhere\"|" "$CONF"
o="$(upgrade --dry-run)"
check "upgrade, config model gone: no in-place note" "$o" "upgrading in place" "!"
check "upgrade, config model gone: exits 1" "$o" "rc=1"
sed -i "s|^asr_path = \"$T/nowhere\"|asr_path = \"$P\"|" "$CONF"

# 11. Upgrade with --sherpa-lib naming the install's own lib dir: the libs
# are the installed files, kept (install(1) refuses "are the same file");
# the binary is still replaced. Without --sherpa-lib the installed libs are
# the default.
L="$T/home/.local/lib/flow-server"
echo sherpa-v1 >"$L/libsherpa-onnx-c-api.so"
echo ort-v1 >"$L/libonnxruntime.so"
echo '#!/usr/bin/env bash' >"$T/home/.local/bin/flow-server"
o="$(upgrade --no-build --sherpa-lib "$L")"
check "same lib dir: completes" "$o" "rc=0"
check "same lib dir: no same-file error" "$o" "are the same file" "!"
check "same lib dir: libs kept" "$o" "$L/libonnxruntime.so is the source, kept"
check "same lib dir: lib content kept" "$(cat "$L/libsherpa-onnx-c-api.so")" "^sherpa-v1$"
check "same lib dir: binary replaced" "$(cmp "$T/target/release/flow-server" "$T/home/.local/bin/flow-server" && echo same)" "same"
check "same lib dir: nothing staged left" "$(ls "$L" "$T/home/.local/bin")" "\.new" "!"
o="$(env -u DBUS_SESSION_BUS_ADDRESS -u SHERPA_ONNX_LIB_DIR PATH="$T/bin:$PATH" HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" \
    FLOW_INSTALL_RUN_USER="$T/run" XDG_RUNTIME_DIR="$R" FLOW_PARAKEET_DIR= \
    bash "$HERE/install.sh" --user --bind 127.0.0.1:7313 --no-build 2>&1; echo "rc=$?")"
check "no --sherpa-lib: installed libs used" "$o" "note: upgrading in place; sherpa libs from $L"
check "no --sherpa-lib: completes" "$o" "rc=0"

# 12. A copy that fails mid-install (an unreadable second lib) leaves the
# previous binary and libs in place, with no staged leftovers.
if [ "$(id -u)" != 0 ]; then
    mkdir -p "$T/badlib"
    echo sherpa-v2 >"$T/badlib/libsherpa-onnx-c-api.so"
    echo ort-v2 >"$T/badlib/libonnxruntime.so"
    chmod 000 "$T/badlib/libonnxruntime.so"
    cp "$T/home/.local/bin/flow-server" "$T/bin.before"
    printf '#!/usr/bin/env bash\n# v2\n' >"$T/target/release/flow-server"
    o="$(upgrade --no-build --sherpa-lib "$T/badlib")"
    check "failed copy: exits 1" "$o" "rc=1"
    check "failed copy: says unchanged" "$o" "installed binary and libraries are unchanged"
    check "failed copy: old binary kept" "$(cmp "$T/bin.before" "$T/home/.local/bin/flow-server" && echo same)" "same"
    check "failed copy: old libs kept" "$(cat "$L/libsherpa-onnx-c-api.so")" "^sherpa-v1$"
    check "failed copy: nothing staged left" "$(ls "$L" "$T/home/.local/bin")" "\.new" "!"
    chmod 644 "$T/badlib/libonnxruntime.so"
fi

if [ "$fails" != 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
