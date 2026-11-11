#!/usr/bin/env bash
# Tests of tools/checks/leak-scan.py: each pattern catches a fake secret of its
# shape, the marked fakes the test suites use stay allowed, errors exit 2, and
# the range mode checks added lines, commit messages and commit identities.
# Positive values are assembled at runtime (`$JOIN` splits each literal) so
# this file never matches itself. Seconds; needs python3 and git. Run by CI
# (repo-checks).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCAN="$ROOT/tools/checks/leak-scan.py"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

# hit NAME LINE: the scanner flags LINE with pattern NAME, masked.
hit() {
  local out
  if out="$(printf '%s\n' "$2" | python3 "$SCAN" --stdin 2>&1)"; then
    fail "$1 not caught: $2"
  fi
  grep -q ": $1 " <<<"$out" || fail "$1: wrong pattern: $out"
  # At most 4 value characters (or 100.x.x.x) after the pattern name.
  grep -qE ": $1 (.{0,4}…|100\.x\.x\.x)$" <<<"$out" || fail "$1: not masked: $out"
  ok "catches $1: ${3:-}"
}
# clean WHY LINE: the scanner lets LINE through.
clean() {
  local out
  out="$(printf '%s\n' "$2" | python3 "$SCAN" --stdin 2>&1)" || fail "$1 flagged: $out"
  ok "allows $1"
}
# usage_error WHY ARGS...: the scanner exits 2 with a message, no traceback.
usage_error() {
  local why="$1" out rc=0
  shift
  out="$("$@" 2>&1)" || rc=$?
  [ "$rc" = 2 ] || fail "$why: exit $rc, want 2: $out"
  grep -q "^leak-scan: " <<<"$out" || fail "$why: no message: $out"
  grep -q Traceback <<<"$out" && fail "$why: traceback"
  ok "exit 2: $why"
}

JOIN=""  # splits each positive literal so it never appears in this file
hit openai-key "key = \"sk-${JOIN}or-v1-9f3Kq2Lm8Zx1Vb7Nc4Hd6Tr0\"" openrouter
hit openai-key "OPENAI_API_KEY=sk-${JOIN}proj-q8W2e4R6t8Y0u2I4o6P8a0" openai
for kind in p o u s r; do
  hit github-token "token: gh${JOIN}${kind}_Zq8Wm3Xn5Rb2Tk7Lp9Vc4Hd1Jf6Gs0" "gh${kind}_"
done
hit github-pat "github${JOIN}_pat_11AZQ8WM3_xn5Rb2Tk7Lp9Vc4Hd1Jf6"
hit hf-token "HF_TOKEN=hf${JOIN}_Zq8Wm3Xn5Rb2Tk7Lp9Vc4Hd1Jf"
hit aws-key-id "aws_access_key_id = AK${JOIN}IAZQ8WM3XN5RB2TK7L" AKIA
hit aws-key-id "aws_access_key_id = AS${JOIN}IAZQ8WM3XN5RB2TK7L" ASIA
hit private-key "-----BEGIN ${JOIN}OPENSSH PRIVATE KEY-----" openssh
hit private-key "-----BEGIN ${JOIN}PRIVATE KEY-----" plain
hit jwt "Authorization: Bearer ey${JOIN}JhbGciOiJIUzI1NiJ9.ey${JOIN}JzdWIiOiJ1c2VyMSJ9.q8W2e4R6t8Y0u2I4"
hit tailnet-ip "server = \"http://100.${JOIN}87.143.22:8178\"" mid-range
hit tailnet-ip "100.${JOIN}64.0.1" "low boundary"
hit tailnet-ip "100.${JOIN}127.255.254" "high boundary"
hit home-path "model at /ho${JOIN}me/alice/models/stt.onnx" absolute
hit home-path "open file:///ho${JOIN}me/alice/notes.txt" file-url
hit windows-home-path "C:\\Us${JOIN}ers\\alice\\AppData\\Roaming" backslash
hit windows-home-path "C:\\\\Us${JOIN}ers\\\\alice\\\\AppData" escaped
hit windows-home-path "c:/us${JOIN}ers/alice/murmur" slash
hit windows-home-path "file:///C:/Us${JOIN}ers/alice/murmur" file-url
# A marker buried mid-segment is not a marker.
hit openai-key "sk-${JOIN}or-v1-9f3Kq2Lm8FAKEx1Vb7Nc4Hd6" "mid-segment FAKE"

# The fake values the existing suites use carry a marker token.
clean "LEAKCHECK fake" 'let key = "sk-or-v1-LEAKCHECK-0123456789abcdef";'
clean "FAKE fake" 'const KEY = "sk-or-v1-FAKELEAKEDKEY0000777788889999"'
clean "PROBE fake" 'sk-or-v1-PROBE-0123456789abcd'
clean "counting fake" 'sk-or-v1-0123456789abcdef0000'
clean "fake hf token" 'hf_FAKEtokenZq8Wm3Xn5Rb2Tk7Lp9'
clean "fake github token" 'ghs_FAKE0000000000000000000000000000'
clean "fake github pat" 'github_pat_FAKE_0000000000000000000000'
clean "fake jwt" 'eyJhbGciOiJub25lIn0.eyJzdWIiOiJ0ZXN0In0.LEAKCHECK-signature'
clean "fake private key" '-----BEGIN FAKE PRIVATE KEY-----'
clean "AWS docs example" 'AKIAIOSFODNN7EXAMPLE'
clean "short sk- value" 'sk-form-key'
clean "task- word" 'subtask-with-a-very-long-kebab-name-here'
clean "doc tailnet address" 'http://100.64.0.10:8178'
clean "tailnet CIDR" '100.64.0.0/10'
clean "below tailnet range" '100.63.255.255'
clean "above tailnet range" '100.128.0.1'
clean "temp HOME in tests" 'HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config"'
clean "placeholder home" 'cd /home/user/murmur'
clean "placeholder windows home" 'C:\Users\you\AppData'
clean "PEM public key" '-----BEGIN PUBLIC KEY-----'

# Errors exit 2 with a message.
usage_error "no argument" python3 "$SCAN"
usage_error "unknown flag" python3 "$SCAN" --nope
usage_error "bad range" python3 "$SCAN" no-such-rev..HEAD
mkdir "$WORK/bad"
cp "$SCAN" "$WORK/bad/leak-scan.py"
usage_error "missing pattern file" python3 "$WORK/bad/leak-scan.py" --tree
printf 'broken (unclosed\n' >"$WORK/bad/leak-patterns.txt"
cp "$ROOT/tools/checks/leak-allow.txt" "$WORK/bad/"
usage_error "bad regex" python3 "$WORK/bad/leak-scan.py" --stdin </dev/null
printf 'only-a-name\n' >"$WORK/bad/leak-patterns.txt"
usage_error "malformed rule" python3 "$WORK/bad/leak-scan.py" --stdin </dev/null

# Range mode: added lines, commit messages and identities.
# A throwaway identity, so commits work on a runner with no global git user.
NOREPLY="1+dev@users.noreply.github.com"
export GIT_AUTHOR_NAME=dev GIT_COMMITTER_NAME=dev
export GIT_AUTHOR_EMAIL="$NOREPLY" GIT_COMMITTER_EMAIL="$NOREPLY"
cd "$WORK"
git init -q -b main repo && cd repo
commit() {  # commit EMAIL MESSAGE FILE TEXT
  printf '%s\n' "$4" >"$3"
  git add "$3"
  GIT_AUTHOR_NAME=dev GIT_AUTHOR_EMAIL="$1" GIT_COMMITTER_NAME=dev GIT_COMMITTER_EMAIL="$1" \
    git commit -q -m "$2"
}
commit "$NOREPLY" "base" a.txt "base"
BASE="$(git rev-parse HEAD)"
commit "$NOREPLY" "clean change" b.txt "sk-or-v1-LEAKCHECK-0123456789abcdef"
python3 "$SCAN" "$BASE..HEAD" >/dev/null 2>&1 || fail "range: clean change flagged"
ok "range: clean change passes"

commit "$NOREPLY" "add key" c.txt "key=sk-${JOIN}or-v1-9f3Kq2Lm8Zx1Vb7Nc4Hd6Tr0"
out="$(python3 "$SCAN" "$BASE..HEAD" 2>&1)" && fail "range: added key missed"
grep -q "c.txt:1: openai-key" <<<"$out" || fail "range: wrong location: $out"
grep -q "9f3Kq2" <<<"$out" && fail "range: secret printed unmasked"
ok "range: added key caught, masked"
git reset -q --hard HEAD~1

# An added line starting with `++ ` reads as `+++ ` in the diff body.
commit "$NOREPLY" "plus line" p.txt "++ key=sk-${JOIN}or-v1-9f3Kq2Lm8Zx1Vb7Nc4Hd6Tr0"
out="$(python3 "$SCAN" "$BASE..HEAD" 2>&1)" && fail "range: '++ ' line missed"
grep -q "p.txt:1: openai-key" <<<"$out" || fail "range: '++ ' line: $out"
ok "range: added '++ ' line scanned"
git reset -q --hard HEAD~1

commit "$NOREPLY" "server at 100.${JOIN}87.143.22" d.txt "x"
out="$(python3 "$SCAN" "$BASE..HEAD" 2>&1)" && fail "range: message leak missed"
grep -q "message:1: tailnet-ip 100.x.x.x" <<<"$out" || fail "range: message: $out"
ok "range: commit message scanned"
git reset -q --hard HEAD~1

# Three dots: from the merge-base, so a line the base side deleted later is
# not reported as added on the branch.
git checkout -q -b old main
commit "$NOREPLY" "old adds key" m.txt "key=sk-${JOIN}or-v1-9f3Kq2Lm8Zx1Vb7Nc4Hd6Tr0"
git checkout -q -b side
commit "$NOREPLY" "side change" s.txt "x"
git checkout -q old
git rm -q m.txt
git commit -q -m "old drops it"
python3 "$SCAN" "old..side" >/dev/null 2>&1 && fail "range: setup: two-dot should see the line"
python3 "$SCAN" "old...side" >/dev/null 2>&1 || fail "range: three-dot range flagged base-side lines"
ok "range: three-dot range starts at merge-base"
git checkout -q main

commit "dev@example.com" "web merge" e.txt "x"
out="$(python3 "$SCAN" "$BASE..HEAD" 2>&1)" && fail "range: real email missed"
grep -q "author email is not a noreply address" <<<"$out" || fail "range: identity: $out"
grep -q "example.com" <<<"$out" && fail "range: email printed"
ok "range: non-noreply identity caught"

# The repo itself is clean, scanned from a subdirectory.
cd "$ROOT/tools"
python3 "$SCAN" --tree >/dev/null || fail "tree scan of the repo has hits"
ok "repo tree clean (from a subdirectory)"
