#!/usr/bin/env bash
# Tests for pull-debug.sh. Never touches a phone: a stub platform tool on PATH
# (same name as the real one) serves the app's data dir from
# tools/phone/fixtures/phone/ (copied, so a test can change it) and logs every
# call. FLOW_PHONE is a fake serial, so even a real tool would find no device.
#
#   tools/phone/test-pull-debug.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

TOOLNAME="$(printf 'a%sb' d)"
mkdir -p "$T/bin"
cp -r "$HERE/fixtures/phone" "$T/phone"
cat >"$T/bin/$TOOLNAME" <<'EOF'
#!/usr/bin/env bash
# Fake platform tool: only `[transport] exec-out run-as <pkg> cat|ls|sha256sum …`.
printf '%s\n' "$*" >>"$FAKE_CALLS"
while [ $# -gt 0 ]; do
    case "$1" in
        -s|-t|-H|-P) shift 2 ;;
        -d|-e|-a) shift ;;
        *) break ;;
    esac
done
[ "${1:-}" = exec-out ] && [ "${2:-}" = run-as ] || { echo "fake: unsupported: $*" >&2; exit 2; }
[ "$3" = dev.flowclone ] || { echo "run-as: unknown package: $3" >&2; exit 1; }
cmd="$4"; shift 4
cd "$FAKE_PHONE"
case "$cmd" in
    cat)
        [ -f "$1" ] || { echo "cat: $1: No such file or directory" >&2; exit 1; }
        if [ -n "${FAKE_CORRUPT:-}" ] && [ "$(basename "$1")" = "$FAKE_CORRUPT" ]; then printf 'garbled'; else cat "$1"; fi ;;
    ls) [ "$1" = -1 ] && shift; [ -d "$1" ] || { echo "ls: $1: No such file or directory" >&2; exit 1; }; ls "$1" ;;
    sha256sum) sha256sum "$@" ;;
    *) echo "fake: unsupported command $cmd" >&2; exit 2 ;;
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
run() { local rc=0 out; out="$(bash "$HERE/pull-debug.sh" "$@" 2>&1)" || rc=$?; printf '%s\nrc=%s\n' "$out" "$rc"; }

SECRETS='FAKE-SEALED-SERVER-TOKEN|FAKE-SEALED-CLOUD-KEY|FAKE-PASSWORD|FAKE-AUTH|FAKE-SECRET|FAKE-NESTED-KEY|FAKELEAKEDKEY|FAKELEAKINLOG|FAKE-SET-TOKEN|31337'

# --- first pull ---
O="$T/out"
out="$(run --out "$O")"
check "first pull exits 0" "$out" "rc=0"
check "summary: 3 lines, 2 new wavs" "$out" "3 lines, 2 wavs (2 new, 0 already here)"
check "summary: bytes from phone" "$out" "[0-9]\+ bytes from phone"
check "transport passed to the tool" "$(cat "$T/calls")" "^-s fake-serial-0 exec-out run-as dev.flowclone"
cmp -s "$O/dictations.jsonl" /dev/null && bad "jsonl pulled" || ok "jsonl pulled"
for w in 2026-10-27T09-00-00.000Z.wav 2026-10-27T11-00-00.000Z.wav; do
    cmp -s "$O/audio/$w" "$T/phone/files/debug/audio/$w" && ok "wav $w identical" || bad "wav $w identical"
done
[ -f "$O/prefs/dev.xml" ] && [ -f "$O/prefs/flow_settings.xml" ] && ok "prefs pulled" || bad "prefs pulled"

# --- redaction ---
all="$(cat "$O"/prefs/*.xml "$O/dictations.jsonl"; printf '%s' "$out")"
check "no secret value in any output or stdout" "$all" "$(sed 's/|/\\|/g' <<<"$SECRETS")" "!"
leaked="$(grep -rlE "$SECRETS" "$O" || true)"; [ -z "$leaked" ] && ok "no secret on disk" || bad "secret on disk: $leaked"
dev="$(cat "$O/prefs/dev.xml")"
check "token entry kept, value redacted" "$dev" 'name="server.token_sealed">&lt;redacted&gt;</string>'
check "cloud key entry redacted" "$dev" 'name="cloud.key_sealed.openrouter">&lt;redacted&gt;</string>'
check "int auth value blanked" "$dev" 'name="AuthRetries" value="0"'
check "nested JSON key redacted, model kept" "$dev" 'llama-3.3-70b'
check "non-secret string kept" "$dev" 'name="server.host">192.0.2.10<'
check "non-secret bool kept" "$dev" 'name="debug_keep_audio" value="true"'
check "keep_local_warm not mistaken for a key" "$dev" 'name="keep_local_warm" value="false"'
fs="$(cat "$O/prefs/flow_settings.xml")"
check "set of tokens emptied" "$fs" 'FAKE-SET-TOKEN' "!"
check "denylist set kept" "$fs" 'com.example.bank'
check "leaked key in log line redacted" "$(cat "$O/dictations.jsonl")" '"text":"second <redacted>"'
python3 -c 'import sys,xml.etree.ElementTree as E; [E.parse(f) for f in sys.argv[1:]]' "$O"/prefs/*.xml && ok "redacted prefs are valid XML" || bad "redacted prefs are valid XML"

# --- incremental ---
: >"$T/calls"
out="$(run --out "$O")"
check "second pull: nothing new" "$out" "2 wavs (0 new, 2 already here)"
check "second pull: no wav cat" "$(cat "$T/calls")" "cat files/debug/audio" "!"
printf 'tampered' >"$O/audio/2026-10-27T09-00-00.000Z.wav"
out="$(run --out "$O")"
check "local copy with wrong sha256 pulled again" "$out" "2 wavs (1 new, 1 already here)"
cmp -s "$O/audio/2026-10-27T09-00-00.000Z.wav" "$T/phone/files/debug/audio/2026-10-27T09-00-00.000Z.wav" && ok "re-pulled copy identical" || bad "re-pulled copy identical"

# --- sha256 mismatch on transfer ---
O2="$T/out2"
out="$(FAKE_CORRUPT=2026-10-27T11-00-00.000Z.wav run --out "$O2")"
check "corrupt transfer fails" "$out" "rc=1"
check "corrupt transfer named" "$out" "sha256 mismatch: 2026-10-27T11-00-00.000Z.wav"
[ ! -e "$O2/audio/2026-10-27T11-00-00.000Z.wav" ] && ok "corrupt copy removed" || bad "corrupt copy removed"

# --- --since ---
out="$(run --out "$T/out3" --since 2026-10-27T10:00:00Z)"
check "--since: 2 lines, 1 wav" "$out" "2 lines, 1 wavs (1 new, 0 already here)"
[ ! -e "$T/out3/audio/2026-10-27T09-00-00.000Z.wav" ] && ok "--since skips older wav" || bad "--since skips older wav"
check "--since filters jsonl" "$(cat "$T/out3/dictations.jsonl")" '"text":"first"' "!"

# --- default out dir, repo guard, transports, errors ---
out="$(run)"
d="$(sed -n 's/.* → \(.*\) (.*/\1/p' <<<"$out" | head -1)"
case "$d" in "$REPO"/*|"") bad "default out dir outside repo ($d)" ;; *) [ -f "$d/dictations.jsonl" ] && ok "default out dir outside repo" || bad "default out dir has files ($d)" ;; esac
[ -n "$d" ] && case "$d" in /tmp/*|"${TMPDIR:-/tmp}"/*) rm -rf "$d" ;; esac
out="$(run --out "$REPO/tools/phone/should-not-exist")"
check "out dir inside repo refused" "$out" "rc=2"
[ ! -e "$REPO/tools/phone/should-not-exist" ] && ok "nothing written in repo" || { bad "nothing written in repo"; rm -rf "$REPO/tools/phone/should-not-exist"; }
: >"$T/calls"
FLOW_PHONE="-t 60" run --out "$T/out4" >/dev/null
check "FLOW_PHONE=-t 60 passed" "$(head -1 "$T/calls")" "^-t 60 exec-out"
: >"$T/calls"
FLOW_PHONE="emulator-5554" run --out "$T/out5" >/dev/null
check "bare serial becomes -s" "$(head -1 "$T/calls")" "^-s emulator-5554 exec-out"
out="$(FLOW_PKG=dev.other run --out "$T/out6")"
check "run-as failure exits 1" "$out" "rc=1"
check "run-as failure explained" "$out" "debug build"
out="$(run --bogus)"
check "unknown option exits 2" "$out" "rc=2"
rm -rf "$T/phone/files/debug/audio"
out="$(run --out "$T/out7")"
check "no audio dir: still ok" "$out" "rc=0"
check "no audio dir: 0 wavs" "$out" "0 wavs"

echo
[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
