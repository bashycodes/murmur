#!/usr/bin/env bash
# Tests of tools/checks/leak-scan.py: each pattern catches a fake secret of its
# shape, the marked fakes the test suites use stay allowed, and the range mode
# checks added lines, commit messages and commit identities. Positive values
# are assembled at runtime (`$P` joins) so this file never matches itself.
# Seconds; needs python3 and git. Run by CI (repo-checks).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCAN="$ROOT/tools/checks/leak-scan.py"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

# hit NAME LINE: the scanner flags LINE with pattern NAME.
hit() {
  local out
  if out="$(printf '%s\n' "$2" | python3 "$SCAN" --stdin 2>&1)"; then
    fail "$1 not caught"
  fi
  grep -q ": $1 " <<<"$out" || fail "$1: wrong pattern: $out"
  ok "catches $1"
}
# clean WHY LINE: the scanner lets LINE through.
clean() {
  local out
  out="$(printf '%s\n' "$2" | python3 "$SCAN" --stdin 2>&1)" || fail "$1 flagged: $out"
  ok "allows $1"
}

P=""  # joins prefix and body so the literal never appears in this file
hit openai-key "key = \"sk-${P}or-v1-9f3Kq2Lm8Zx1Vb7Nc4Hd6Tr0\""
hit openai-key "OPENAI_API_KEY=sk-${P}proj-q8W2e4R6t8Y0u2I4o6P8a0"
hit github-token "token: gh${P}p_Zq8Wm3Xn5Rb2Tk7Lp9Vc4Hd1Jf6Gs0"
hit github-token "gh${P}o_Zq8Wm3Xn5Rb2Tk7Lp9Vc4Hd1Jf6Gs0"
hit github-pat "github${P}_pat_11AZQ8WM3_xn5Rb2Tk7Lp9Vc4Hd1Jf6"
hit hf-token "HF_TOKEN=hf${P}_Zq8Wm3Xn5Rb2Tk7Lp9Vc4Hd1Jf"
hit aws-key-id "aws_access_key_id = AK${P}IAZQ8WM3XN5RB2TK7L"
hit private-key "-----BEGIN ${P}OPENSSH PRIVATE KEY-----"
hit private-key "-----BEGIN ${P}PRIVATE KEY-----"
hit jwt "Authorization: Bearer ey${P}JhbGciOiJIUzI1NiJ9.ey${P}JzdWIiOiJ1c2VyMSJ9.q8W2e4R6t8Y0u2I4"
hit tailnet-ip "server = \"http://100.${P}87.143.22:8178\""
hit tailnet-ip "ssh 100.${P}127.3.4"
hit home-path "model at /ho${P}me/alice/models/stt.onnx"
hit windows-home-path "C:\\Us${P}ers\\alice\\AppData\\Roaming"
hit windows-home-path "C:\\\\Us${P}ers\\\\alice\\\\AppData"
hit windows-home-path "c:/us${P}ers/alice/flow"

# The fake values the existing suites use carry a marker word.
clean "LEAKCHECK fake" 'let key = "sk-or-v1-LEAKCHECK-0123456789abcdef";'
clean "FAKE fake" 'const KEY = "sk-or-v1-FAKELEAKEDKEY0000777788889999"'
clean "PROBE fake" 'sk-or-v1-PROBE-0123456789abcd'
clean "fake hf token" 'hf_FAKEtokenZq8Wm3Xn5Rb2Tk7Lp9'
clean "fake github token" 'ghp_FAKE0000000000000000000000000000'
clean "AWS docs example" 'AKIAIOSFODNN7EXAMPLE'
clean "short sk- value" 'sk-form-key'
clean "task- word" 'subtask-with-a-very-long-kebab-name-here'
clean "doc tailnet address" 'http://100.64.0.10:8178'
clean "range boundary" '100.127.255.254'
clean "public 100.x" 'http://100.200.3.4/'
clean "temp HOME in tests" 'HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config"'
clean "placeholder home" 'cd /home/user/flow-clone'
clean "placeholder windows home" 'C:\Users\you\AppData'
clean "PEM public key" '-----BEGIN PUBLIC KEY-----'

# Range mode: added lines, commit messages and identities.
cd "$T"
git init -q -b main r && cd r
commit() {  # commit EMAIL MESSAGE FILE TEXT
  printf '%s\n' "$4" >"$3"
  git add "$3"
  GIT_AUTHOR_NAME=dev GIT_AUTHOR_EMAIL="$1" GIT_COMMITTER_NAME=dev GIT_COMMITTER_EMAIL="$1" \
    git commit -q -m "$2"
}
NR="1+dev@users.noreply.github.com"
commit "$NR" "base" a.txt "base"
BASE="$(git rev-parse HEAD)"
commit "$NR" "clean change" b.txt "sk-or-v1-LEAKCHECK-0123456789abcdef"
python3 "$SCAN" "$BASE..HEAD" >/dev/null 2>&1 || fail "range: clean change flagged"
ok "range: clean change passes"

commit "$NR" "add key" c.txt "key=sk-${P}or-v1-9f3Kq2Lm8Zx1Vb7Nc4Hd6Tr0"
out="$(python3 "$SCAN" "$BASE..HEAD" 2>&1)" && fail "range: added key missed"
grep -q "c.txt:1: openai-key" <<<"$out" || fail "range: wrong location: $out"
grep -q "9f3Kq2" <<<"$out" && fail "range: secret printed unmasked"
ok "range: added key caught, masked"
git reset -q --hard HEAD~1

commit "$NR" "server at 100.${P}87.143.22" d.txt "x"
out="$(python3 "$SCAN" "$BASE..HEAD" 2>&1)" && fail "range: message leak missed"
grep -q "message:1: tailnet-ip" <<<"$out" || fail "range: message: $out"
ok "range: commit message scanned"
git reset -q --hard HEAD~1

commit "dev@example.com" "web merge" e.txt "x"
out="$(python3 "$SCAN" "$BASE..HEAD" 2>&1)" && fail "range: real email missed"
grep -q "author email is not a noreply address" <<<"$out" || fail "range: identity: $out"
grep -q "example.com" <<<"$out" && fail "range: email printed"
ok "range: non-noreply identity caught"

# The repo itself is clean.
cd "$ROOT"
python3 "$SCAN" --tree >/dev/null || fail "tree scan of the repo has hits"
ok "repo tree clean"
