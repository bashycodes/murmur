#!/usr/bin/env bash
# Copy the phone's debug state to a temporary directory on this machine in one
# command: the debug log (files/debug/dictations.jsonl), the kept dictation
# WAVs (files/debug/audio/, only the ones not already here, sha256-checked) and
# the two settings files (shared_prefs/flow_settings.xml, dev.xml) with every
# token / key / secret / password / auth value removed before it touches disk.
# Read-only on the phone: recordings stay there. Needs a debug build (run-as).
#
#   tools/phone/pull-debug.sh [--since TS] [--out DIR]
#
#   --since TS  only log lines and WAVs from TS on (e.g. 2026-10-27T10:00Z)
#   --out DIR   where to put them (default: a new temp dir; never inside the repo).
#               Pull into the same DIR again to fetch only the new WAVs.
#
# Environment:
#   FLOW_PHONE          transport: a serial (emulator-5554, 100.64.0.5:5555) or
#                       raw transport options ("-t 60", "-d"); empty = the only device
#   FLOW_PKG            app package (default dev.flowclone)
#   FLOW_PLATFORM_TOOL  the Android platform tool to call (default: the one on PATH)
#
# Summary on stdout: "pulled N lines, N wavs (N new, N already here), N prefs → DIR (N bytes from phone)".
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd -P)"
REPO="$(cd "$HERE/../.." && pwd -P)"
TOOL="${FLOW_PLATFORM_TOOL:-$(printf 'a%sb' d)}"
PKG="${FLOW_PKG:-dev.flowclone}"

usage() { sed -n '2,/^set -euo/{/^set -euo/d;s/^# \{0,1\}//;p}' "$0" >&2; exit 2; }
die() { echo "pull-debug: $1" >&2; exit "${2:-1}"; }

SINCE="" OUT=""
while [ $# -gt 0 ]; do
    case "$1" in
        --since) [ $# -ge 2 ] || usage; SINCE="$2"; shift 2 ;;
        --out) [ $# -ge 2 ] || usage; OUT="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "pull-debug: unknown option $1" >&2; usage ;;
    esac
done

# Transport: "-t 60" / "-s X" / "-d" as given, a bare value is a serial.
TRANSPORT=()
case "${FLOW_PHONE:-}" in
    "") ;;
    -*) read -r -a TRANSPORT <<<"$FLOW_PHONE" ;;
    *) TRANSPORT=(-s "$FLOW_PHONE") ;;
esac

# --since → the log's own format (2026-10-27T10:00:00.000Z), so plain string order works.
if [ -n "$SINCE" ]; then
    SINCE="$(date -u -d "$SINCE" +%Y-%m-%dT%H:%M:%S.%3NZ 2>/dev/null)" || die "--since: not a time: $SINCE" 2
fi
SINCE_WAV="${SINCE//:/-}"

if [ -z "$OUT" ]; then
    OUT="$(mktemp -d "${TMPDIR:-/tmp}/flow-debug.XXXXXX")"
else
    real="$(realpath -m -- "$OUT")"
    case "$real/" in "$REPO"/*) die "--out $OUT is inside the repo; pick a directory outside it" 2 ;; esac
    OUT="$real"
fi

mkdir -p "$OUT"
BYTES=0
pull() { "$TOOL" "${TRANSPORT[@]}" exec-out run-as "$PKG" "$@"; }
# pull_to FILE ARGS…: stdout of the call into FILE, counted.
pull_to() { local f="$1"; shift; pull "$@" >"$f" || return 1; BYTES=$((BYTES + $(wc -c <"$f"))); }
# pull_var ARGS…: stdout of the call into $REPLY (kept in memory, never on disk), counted.
pull_var() {
    local LC_ALL=C
    REPLY="$(pull "$@" && printf x)" || return 1 # the x keeps trailing newlines
    REPLY="${REPLY%x}"; BYTES=$((BYTES + ${#REPLY}))
}

pull_var ls . 2>"$OUT/.err" || die "cannot read $PKG's data (run-as needs a debug build of the app, a connected phone and the right FLOW_PHONE): $(head -c 300 "$OUT/.err")"
rm -f "$OUT/.err"
mkdir -p "$OUT/audio" "$OUT/prefs"

# Redactor (stdin → stdout). Mode "xml": SharedPreferences XML; "jsonl": debug log lines.
# A name matching SECRET (or a known secret pref) has its value blanked: strings → <redacted>,
# numbers → 0, sets emptied. JSON inside string values is redacted by key the same way, and any
# value that looks like an API key (sk-…) or a bearer token is replaced wherever it appears.
REDACT_PY='
import json, re, sys
import xml.etree.ElementTree as ET
SECRET = re.compile(r"token|key|secret|passw|auth|sealed|cred|bearer|cookie", re.I)
# Known secret prefs (DevStore.kt): server.token_sealed, cloud.key_sealed.<id>. Covered by SECRET.
LOOKS = re.compile(r"sk-[A-Za-z0-9_-]{16,}|Bearer\s+[A-Za-z0-9._~+/=-]{8,}")
R = "<redacted>"
def j(v):
    if isinstance(v, dict): return {k: (R if SECRET.search(k) and v[k] not in (None, "") else j(x)) for k, x in v.items()}
    if isinstance(v, list): return [j(x) for x in v]
    if isinstance(v, str): return LOOKS.sub(R, v)
    return v
def s(text):
    t = text.strip()
    if t[:1] in "{[":
        try: return json.dumps(j(json.loads(t)), separators=(",", ":"), ensure_ascii=False)
        except ValueError: pass
    return LOOKS.sub(R, text)
mode = sys.argv[1]
data = sys.stdin.read()
if mode == "jsonl":
    for line in data.splitlines():
        try: o = json.loads(line)
        except ValueError: print(LOOKS.sub(R, line)); continue
        r = j(o)
        print(line if r == o else json.dumps(r, separators=(",", ":"), ensure_ascii=False))
    sys.exit(0)
root = ET.fromstring(data)
for e in root:
    name = e.get("name", "")
    if SECRET.search(name):
        if e.tag == "set":
            for c in list(e): e.remove(c)
        elif e.tag == "string":
            e.text = R if e.text else e.text
        elif e.tag in ("int", "long", "float"):
            e.set("value", "0")
    elif e.tag == "string" and e.text:
        e.text = s(e.text)
    elif e.tag == "set":
        for c in e:
            if c.text: c.text = LOOKS.sub(R, c.text)
sys.stdout.write("<?xml version=\x271.0\x27 encoding=\x27utf-8\x27 standalone=\x27yes\x27 ?>\n")
sys.stdout.write(ET.tostring(root, encoding="unicode") + "\n")
'
redact() { python3 -I -c "$REDACT_PY" "$1"; }

# 1. Debug log (missing when the debug log was never on).
LINES=0
if pull_var cat files/debug/dictations.jsonl 2>/dev/null; then
    if [ -n "$SINCE" ]; then
        REPLY="$(printf '%s' "$REPLY" | awk -v s="$SINCE" 'match($0, /"ts":"[^"]*"/) { if (substr($0, RSTART + 6, RLENGTH - 7) >= s) print }')"
    fi
    printf '%s' "$REPLY" | redact jsonl >"$OUT/dictations.jsonl"
    LINES=$(grep -c . "$OUT/dictations.jsonl" || true)
fi

# 2. WAVs: list + sha256 of all in two calls; pull only the missing / differing ones.
WAVS=0 NEW=0 HAVE=0 BAD=0
names=()
if pull_var ls files/debug/audio 2>/dev/null; then
    while read -r n; do
        case "$n" in *.wav) ;; *) continue ;; esac
        [ -n "$SINCE" ] && [[ "${n%.wav}" < "$SINCE_WAV" ]] && continue
        names+=("$n")
    done <<<"$REPLY"
fi
if [ ${#names[@]} -gt 0 ]; then
    pull_var sha256sum "${names[@]/#/files/debug/audio/}" || die "sha256sum failed on the phone"
    declare -A want=()
    while read -r h p; do [ -n "${p:-}" ] && want["$(basename "$p")"]="$h"; done <<<"$REPLY"
    for n in "${names[@]}"; do
        h="${want[$n]:-}"
        [ -n "$h" ] || { echo "pull-debug: no sha256 for $n (deleted meanwhile?)" >&2; continue; }
        WAVS=$((WAVS + 1))
        f="$OUT/audio/$n"
        if [ -f "$f" ] && [ "$(sha256sum <"$f" | cut -d' ' -f1)" = "$h" ]; then HAVE=$((HAVE + 1)); continue; fi
        if pull_to "$f.part" cat "files/debug/audio/$n" && [ "$(sha256sum <"$f.part" | cut -d' ' -f1)" = "$h" ]; then
            mv -f "$f.part" "$f"; NEW=$((NEW + 1))
        else
            rm -f "$f.part" "$f"; BAD=$((BAD + 1)); echo "pull-debug: sha256 mismatch: $n (not kept)" >&2
        fi
    done
fi

# 3. Settings, redacted in memory before writing.
PREFS=0
for p in flow_settings dev; do
    if pull_var cat "shared_prefs/$p.xml" 2>/dev/null; then
        printf '%s' "$REPLY" | redact xml >"$OUT/prefs/$p.xml" || { rm -f "$OUT/prefs/$p.xml"; die "could not redact $p.xml; not kept"; }
        PREFS=$((PREFS + 1))
    fi
done
REPLY=""

echo "pulled $LINES lines, $WAVS wavs ($NEW new, $HAVE already here), $PREFS prefs → $OUT ($BYTES bytes from phone)"
[ "$BAD" -eq 0 ] || exit 1
