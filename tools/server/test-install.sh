#!/usr/bin/env bash
# Tests for install.sh's --user preflight of the systemd user manager.
# No systemd, model or build needed: a stub `systemctl` on PATH answers only
# when $XDG_RUNTIME_DIR/bus is a socket, and a fake /run/user root is passed
# through FLOW_INSTALL_RUN_USER.
#
#   tools/server/test-install.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

mkdir -p "$T/bin" "$T/model" "$T/lib" "$T/run/$(id -u)" "$T/elsewhere" "$T/home"
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

if [ "$fails" != 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
