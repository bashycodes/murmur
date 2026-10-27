#!/usr/bin/env bash
# Run one real dictation on the phone from a recorded WAV instead of the microphone
# (debug builds only). Pushes the WAV into the app's private storage
# (files/debug/wav-in/, via run-as), checks it arrived intact (sha256), then sends the
# debug broadcast; the app runs the normal pipeline (route order, cleanup, insert guard,
# insert into the focused field, debug log line with "source":"wav"). Focus a text field
# on the phone first. Protected fields (password, banking, denylisted apps) are refused.
#
#   tools/phone/play-wav.sh [--fast] [--tap] FILE.wav
#   tools/phone/play-wav.sh [--fast] [--tap] --on-phone files/debug/audio/NAME.wav
#
#   FILE.wav        16 kHz mono 16-bit PCM WAV on this machine (checked before pushing)
#   --on-phone P    replay a WAV already in the app's files/debug/ (e.g. a kept dictation); no push
#   --fast          feed the audio as fast as the pipeline takes it (default: real time)
#   --tap           a hands-free dictation (ends on silence or at the end); default: hold
#
# Environment:
#   FLOW_PHONE          transport: a serial (emulator-5554, 100.64.0.5:5555) or
#                       raw transport options ("-t 60", "-d"); empty = the only device
#   FLOW_PKG            app package (default dev.flowclone)
#   FLOW_PLATFORM_TOOL  the Android platform tool to call (default: the one on PATH)
#
# Prints the app's answer, e.g. "ok: hello.wav queued"; exit 1 when the app refused.
set -euo pipefail
TOOL="${FLOW_PLATFORM_TOOL:-$(printf 'a%sb' d)}"
PKG="${FLOW_PKG:-dev.flowclone}"
RECEIVER="dev.flowclone.debug.WavMicReceiver"
ACTION="dev.flowclone.debug.PLAY_WAV"

usage() { sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2; exit 2; }
die() { echo "play-wav: $1" >&2; exit "${2:-1}"; }

FAST=0 TAP=0 ONPHONE="" FILE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --fast) FAST=1; shift ;;
        --tap) TAP=1; shift ;;
        --on-phone) [ $# -ge 2 ] || usage; ONPHONE="$2"; shift 2 ;;
        -h|--help) usage ;;
        -*) echo "play-wav: unknown option $1" >&2; usage ;;
        *) [ -z "$FILE" ] || usage; FILE="$1"; shift ;;
    esac
done
[ -n "$FILE" ] || [ -n "$ONPHONE" ] || usage
[ -z "$FILE" ] || [ -z "$ONPHONE" ] || die "give FILE.wav or --on-phone, not both" 2

# Transport: "-t 60" / "-s X" / "-d" as given, a bare value is a serial.
TRANSPORT=()
case "${FLOW_PHONE:-}" in
    "") ;;
    -*) read -r -a TRANSPORT <<<"$FLOW_PHONE" ;;
    *) TRANSPORT=(-s "$FLOW_PHONE") ;;
esac
phone() { "$TOOL" "${TRANSPORT[@]}" "$@"; }

if [ -n "$FILE" ]; then
    [ -f "$FILE" ] || die "no such file: $FILE" 2
    # The app accepts only 16 kHz mono PCM16; say so here rather than after the push.
    python3 -I -c '
import sys, wave
try:
    w = wave.open(sys.argv[1], "rb")
except Exception as e:
    sys.exit(f"not a PCM WAV ({e})")
r, c, b = w.getframerate(), w.getnchannels(), w.getsampwidth()
if (r, c, b) != (16000, 1, 2):
    sys.exit(f"{r} Hz, {c} channel(s), {8 * b}-bit; needs 16000 Hz mono 16-bit (e.g. ffmpeg -i in -ar 16000 -ac 1 -c:a pcm_s16le out.wav)")
' "$FILE" || die "$FILE: refused" 2
    NAME="$(basename "$FILE" | tr -c 'A-Za-z0-9._\n-' '_')"
    case "$NAME" in *.wav) ;; *) NAME="$NAME.wav" ;; esac
    DEST="files/debug/wav-in/$NAME"
    phone exec-in run-as "$PKG" sh -c "'mkdir -p files/debug/wav-in && cat > $DEST'" <"$FILE" \
        || die "push failed (run-as needs a debug build of the app, a connected phone and the right FLOW_PHONE)"
    want="$(sha256sum <"$FILE" | cut -d' ' -f1)"
    got="$(phone exec-out run-as "$PKG" sha256sum "$DEST" | cut -d' ' -f1)" || die "sha256sum failed on the phone"
    [ "$want" = "$got" ] || die "$DEST: sha256 mismatch after the push (got ${got:-nothing})"
else
    case "$ONPHONE" in files/debug/*) DEST="$ONPHONE" ;; *) die "--on-phone must be under files/debug/: $ONPHONE" 2 ;; esac
fi

args=(shell am broadcast -a "$ACTION" -n "$PKG/$RECEIVER" --es path "$DEST")
[ "$FAST" = 0 ] || args+=(--ez fast true)
[ "$TAP" = 0 ] || args+=(--ez tap true)
out="$(phone "${args[@]}" 2>&1)" || die "broadcast failed: $out"
# am prints: Broadcast completed: result=0, data="ok: x.wav queued"
data="$(sed -n 's/.*data="\(.*\)".*/\1/p' <<<"$out" | tail -n 1)"
[ -n "$data" ] || die "no answer from the app (not a debug build, or an old one without the WAV receiver?): $out"
echo "$data"
case "$data" in ok:*) ;; *) exit 1 ;; esac
