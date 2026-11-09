#!/usr/bin/env bash
# D46 Calls device check (debug builds only): decode a recording on the phone with the Calls
# decoder (MediaCodec for m4a/AAC, the WAV reader for WAV) to 16 kHz mono and print what came
# out; optionally transcribe it with the Calls job too. Pushes the file into the app's private
# storage (files/debug/calls-in/, staged in /data/local/tmp, copied in via run-as), checks it
# arrived intact (sha256), then sends the debug broadcast. The phone's shared storage is not
# touched.
#
#   tools/phone/call-decode.sh [--transcribe] [--lang he|en] [FILE]
#
#   FILE           audio file on this machine (default: the committed public-domain fixture
#                  apps/murmur-android/tools/fixtures/calls/jfk-call.m4a, 11 s, 44.1 kHz stereo AAC)
#   --transcribe   then transcribe it with the Calls job (its own engine); the transcript shows
#                  on the Calls page, progress in logcat (tag MurmurCallDecode)
#   --lang L       the call's language for --transcribe (default en)
#
# Expected for the fixture: "ok: in 44100 Hz 2 ch ...; out ~176000 samples 16000 Hz mono = ~11000 ms".
#
# Environment:
#   MURMUR_PHONE          transport: a serial or raw transport options ("-t 60", "-d"); empty = the only device
#   MURMUR_PKG            app package (default dev.murmur)
#   MURMUR_PLATFORM_TOOL  the Android platform tool to call (default: the one on PATH)
set -euo pipefail
TOOL="${MURMUR_PLATFORM_TOOL:-$(printf 'a%sb' d)}"
PKG="${MURMUR_PKG:-dev.murmur}"
RECEIVER="dev.murmur.debug.CallDecodeReceiver"
ACTION="dev.murmur.debug.DECODE_CALL"
HERE="$(cd "$(dirname "$0")" && pwd)"

usage() { sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2; exit 2; }
die() { echo "call-decode: $1" >&2; exit "${2:-1}"; }

TRANSCRIBE=0 LANG_CODE="en" FILE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --transcribe) TRANSCRIBE=1; shift ;;
        --lang) [ $# -ge 2 ] || usage; LANG_CODE="$2"; shift 2 ;;
        -h|--help) usage ;;
        -*) echo "call-decode: unknown option $1" >&2; usage ;;
        *) [ -z "$FILE" ] || usage; FILE="$1"; shift ;;
    esac
done
[ -n "$FILE" ] || FILE="$HERE/../../apps/murmur-android/tools/fixtures/calls/jfk-call.m4a"
[ -f "$FILE" ] || die "no such file: $FILE" 2
case "$LANG_CODE" in he|en) ;; *) die "--lang must be he or en" 2 ;; esac

TRANSPORT=()
case "${MURMUR_PHONE:-}" in
    "") ;;
    -*) read -r -a TRANSPORT <<<"$MURMUR_PHONE" ;;
    *) TRANSPORT=(-s "$MURMUR_PHONE") ;;
esac
phone() { "$TOOL" "${TRANSPORT[@]}" "$@"; }

NAME="$(basename "$FILE" | tr -c 'A-Za-z0-9._\n-' '_')"
DEST="files/debug/calls-in/$NAME"
# exec-in/exec-out skip the device shell on some phones (Android 17): inner quotes reach sh
# literally, the command fails, yet exit 0 comes back. So stage the file in /data/local/tmp
# with push and copy it in through `shell`, which always runs a shell.
TMP="/data/local/tmp/murmur-call-decode-$NAME"
phone push "$FILE" "$TMP" >/dev/null \
    || die "push failed (a connected phone and the right MURMUR_PHONE?)"
copied=0
phone shell "cat '$TMP' | run-as '$PKG' sh -c 'mkdir -p files/debug/calls-in && cat > $DEST'" || copied=$?
phone shell "rm -f '$TMP'" || true
[ "$copied" = 0 ] || die "copy into the app failed (run-as needs a debug build of the app)"
want="$(sha256sum <"$FILE" | cut -d' ' -f1)"
raw="$(phone shell "run-as '$PKG' sha256sum $DEST" 2>&1 | tr -d '\r')" || true
got="$(cut -d' ' -f1 <<<"$raw" | head -n 1)"
[[ "$got" =~ ^[0-9a-f]{64}$ ]] || die "sha256sum failed on the phone: ${raw:-no output}"
[ "$want" = "$got" ] || die "$DEST: sha256 mismatch after the push (got $got, want $want)"

args=(shell am broadcast -a "$ACTION" -n "$PKG/$RECEIVER" --es path "$DEST")
[ "$TRANSCRIBE" = 0 ] || args+=(--ez transcribe true --es lang "$LANG_CODE")
out="$(phone "${args[@]}" 2>&1)" || die "broadcast failed: $out"
data="$(sed -n 's/.*data="\(.*\)".*/\1/p' <<<"$out" | tail -n 1)"
[ -n "$data" ] || die "no answer from the app (not a debug build, or one without the Calls receiver?): $out"
echo "$data"
case "$data" in ok:*) ;; *) exit 1 ;; esac
