#!/usr/bin/env bash
# Tests for play-wav.sh. Never touches a phone: a stub platform tool on PATH (same
# name as the real one) keeps the app's data dir in a temp dir, answers the
# broadcast like the debug receiver, and logs every call. FLOW_PHONE is a fake
# serial, so even a real tool would find no device.
#
#   tools/phone/test-play-wav.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

TOOLNAME="$(printf 'a%sb' d)"
mkdir -p "$T/bin" "$T/phone"
cat >"$T/bin/$TOOLNAME" <<'EOF'
#!/usr/bin/env bash
# Fake platform tool: exec-in run-as <pkg> sh -c '…cat > F', exec-out run-as <pkg> sha256sum F,
# shell am broadcast … (answers like WavMicReceiver).
printf '%s\n' "$*" >>"$FAKE_CALLS"
while [ $# -gt 0 ]; do
    case "$1" in
        -s|-t|-H|-P) shift 2 ;;
        -d|-e|-a) shift ;;
        *) break ;;
    esac
done
cd "$FAKE_PHONE"
case "$1" in
    exec-in)
        # One remote command line, as the real tool sends it: run-as PKG sh -c '…'
        line="${*:2}"
        [[ "$line" == "run-as dev.flowclone sh -c '"*"'" ]] || { echo "fake: unsupported: $line" >&2; exit 2; }
        cmd="${line#run-as dev.flowclone sh -c \'}"; cmd="${cmd%\'}"
        if [ -n "${FAKE_GARBLE:-}" ]; then cmd="${cmd/cat >/head -c 10 >}"; fi
        sh -c "$cmd" ;;
    exec-out)
        [ "$2" = run-as ] && [ "$3" = dev.flowclone ] && [ "$4" = sha256sum ] || { echo "fake: unsupported: $*" >&2; exit 2; }
        sha256sum "$5" ;;
    shell)
        shift
        [ "$1 $2" = "am broadcast" ] || { echo "fake: unsupported: $*" >&2; exit 2; }
        echo "Broadcasting: Intent { act=dev.flowclone.debug.PLAY_WAV }"
        path=""; while [ $# -gt 0 ]; do [ "$1" = --es ] && [ "$2" = path ] && path="$3"; shift; done
        if [ -f "$path" ]; then echo "Broadcast completed: result=0, data=\"ok: $(basename "$path") queued\""
        else echo "Broadcast completed: result=0, data=\"error: no WAV file under files/debug/: $path\""; fi ;;
    *) echo "fake: unsupported $*" >&2; exit 2 ;;
esac
EOF
chmod +x "$T/bin/$TOOLNAME"
export PATH="$T/bin:$PATH" FAKE_PHONE="$T/phone" FAKE_CALLS="$T/calls" FLOW_PHONE="-s fake-serial-0"
unset FLOW_PLATFORM_TOOL
[ "$(command -v "$TOOLNAME")" = "$T/bin/$TOOLNAME" ] || { echo "stub not first on PATH; refusing to run"; exit 1; }

fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fails=$((fails + 1)); }
check() { # NAME TEXT PATTERN [!]
    if [ "${4:-}" = "!" ]; then grep -q -- "$3" <<<"$2" && bad "$1" || ok "$1"
    else grep -q -- "$3" <<<"$2" && ok "$1" || bad "$1"; fi
}
run() { local rc=0 out; out="$(bash "$HERE/play-wav.sh" "$@" 2>&1)" || rc=$?; printf '%s\nrc=%s\n' "$out" "$rc"; }
wav() { # FILE RATE CHANNELS
    python3 -I -c '
import sys, wave
w = wave.open(sys.argv[1], "wb"); w.setnchannels(int(sys.argv[3])); w.setsampwidth(2); w.setframerate(int(sys.argv[2]))
w.writeframes(bytes(range(256)) * 50); w.close()' "$1" "$2" "$3"
}

wav "$T/hello there.wav" 16000 1
wav "$T/stereo.wav" 16000 2
wav "$T/cd.wav" 44100 1

# --- push + broadcast ---
out="$(run "$T/hello there.wav")"
check "exits 0" "$out" "rc=0"
check "prints the app's answer" "$out" "^ok: hello_there.wav queued"
cmp -s "$T/hello there.wav" "$T/phone/files/debug/wav-in/hello_there.wav" && ok "wav pushed intact, name sanitised" || bad "wav pushed intact"
calls="$(cat "$T/calls")"
check "transport passed on every call" "$(grep -vc '^-s fake-serial-0 ' <<<"$calls" || true)" "^0$"
check "push via run-as" "$calls" "exec-in run-as dev.flowclone sh -c 'mkdir -p files/debug/wav-in && cat > files/debug/wav-in/hello_there.wav'"
check "explicit receiver, path extra" "$calls" "am broadcast -a dev.flowclone.debug.PLAY_WAV -n dev.flowclone/dev.flowclone.debug.WavMicReceiver --es path files/debug/wav-in/hello_there.wav$"
check "real time by default" "$calls" "--ez fast" "!"

# --- flags ---
: >"$T/calls"
out="$(run --fast --tap "$T/hello there.wav")"
check "--fast --tap exit 0" "$out" "rc=0"
check "--fast and --tap passed" "$(cat "$T/calls")" "--es path files/debug/wav-in/hello_there.wav --ez fast true --ez tap true"

# --- replay a WAV already on the phone ---
mkdir -p "$T/phone/files/debug/audio"; cp "$T/hello there.wav" "$T/phone/files/debug/audio/2026-10-27T10-00-00.000Z.wav"
: >"$T/calls"
out="$(run --on-phone files/debug/audio/2026-10-27T10-00-00.000Z.wav)"
check "--on-phone exit 0" "$out" "rc=0"
check "--on-phone does not push" "$(cat "$T/calls")" "exec-in" "!"
out="$(run --on-phone shared_prefs/dev.xml)"
check "--on-phone outside files/debug refused" "$out" "rc=2"

# --- refusals ---
: >"$T/calls"
out="$(run "$T/stereo.wav")"
check "stereo refused before any push" "$out" "needs 16000 Hz mono 16-bit"
check "stereo: exit 2" "$out" "rc=2"
out="$(run "$T/cd.wav")"
check "44.1 kHz refused" "$out" "44100 Hz"
check "nothing sent to the phone" "$(cat "$T/calls" 2>/dev/null)" "." "!"
out="$(run "$T/missing.wav")"
check "missing file: exit 2" "$out" "rc=2"
out="$(FAKE_GARBLE=1 run "$T/hello there.wav")"
check "short push detected" "$out" "sha256 mismatch"
check "short push: exit 1, no broadcast" "$out" "rc=1"
rm -f "$T/phone/files/debug/audio/"*
out="$(run --on-phone files/debug/audio/gone.wav)"
check "app error is shown" "$out" "^error: no WAV file"
check "app error: exit 1" "$out" "rc=1"

echo
[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
