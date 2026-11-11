#!/usr/bin/env bash
# Tests of tools/public/sync.sh's publish checks on throwaway local repos (no
# filter run, no network): a dry run pushes nothing, --push fast-forwards
# public main and nothing else, and every refusal (history not reproduced,
# not a fast-forward, failed or foreign run, leak in the new range, changed
# or misplaced tags) exits 1 with public main untouched. Seconds; needs git
# and python3. Run by CI (repo-checks).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SYNC="$ROOT/tools/public/sync.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

# Runners have no git identity; every commit here is a noreply one.
export GIT_AUTHOR_NAME=tester GIT_COMMITTER_NAME=tester
export GIT_AUTHOR_EMAIL=0+tester@users.noreply.github.com GIT_COMMITTER_EMAIL=0+tester@users.noreply.github.com
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export MURMUR_PUBLIC_URL="$WORK/public.git" MURMUR_PUBLIC_PUSH_URL="$WORK/public.git"
export MURMUR_SOURCE_GITDIR="$WORK/priv/.git"

g() { git -C "$@"; }
# commit REPO MSG: one commit touching a file named after MSG.
commit() {
  echo "$2" > "$1/${2// /_}.txt"
  g "$1" add -A && g "$1" commit -q -m "$2"
  g "$1" rev-parse HEAD
}

# The private side: two commits, origin/integration on the second.
git init -q -b integration "$WORK/priv"
commit "$WORK/priv" "private one" >/dev/null
P2=$(commit "$WORK/priv" "private two")
g "$WORK/priv" update-ref refs/remotes/origin/integration "$P2"

# The "filtered" history a filter run would produce: A (published) then B.
git init -q -b integration "$WORK/filt"
A=$(commit "$WORK/filt" "release notes")
g "$WORK/filt" tag -a v1.0.0 -m "Release 1.0.0"
B=$(commit "$WORK/filt" "fix a typo")

# mk_run NAME SRC_REPO [SOURCE_SHA]: a passed filter run whose repo/ mirrors SRC_REPO.
mk_run() {
  local d="$WORK/runs/$1"
  mkdir -p "$d"
  git clone -q --mirror "$2" "$d/repo"
  g "$d/repo" remote remove origin >/dev/null 2>&1
  echo "${3:-$P2}" > "$d/source"
  echo ok > "$d/ok"
  echo "$d"
}
# reset_public SHA: a public repo whose main is SHA, with the v1.0.0 tag.
reset_public() {
  rm -rf "$WORK/public.git"
  git init -q --bare "$WORK/public.git"
  g "$WORK/filt" push -q "$WORK/public.git" "$1:refs/heads/main" refs/tags/v1.0.0
}
pub_main() { g "$WORK/public.git" rev-parse refs/heads/main; }
# refused WHY PATTERN ARGS...: sync.sh exits 1, says PATTERN, leaves main as it was.
refused() {
  local why="$1" pat="$2" before out rc=0
  shift 2
  before=$(pub_main)
  out=$("$SYNC" "$@" 2>&1) || rc=$?
  [ "$rc" = 1 ] || fail "$why: exit $rc, want 1: $out"
  grep -q "REFUSED: .*$pat" <<<"$out" || fail "$why: no '$pat' in: $out"
  [ "$(pub_main)" = "$before" ] || fail "$why: public main moved"
  ok "refuses: $why"
}

RUN=$(mk_run good "$WORK/filt")
reset_public "$A"

out=$("$SYNC" --run "$RUN" 2>&1) || fail "dry run: $out"
grep -q "1 new commit" <<<"$out" || fail "dry run count: $out"
grep -q "dry run: nothing pushed" <<<"$out" || fail "dry run message: $out"
[ "$(pub_main)" = "$A" ] || fail "dry run moved public main"
ok "dry run prints the range and pushes nothing"

out=$("$SYNC" --run "$RUN" --push 2>&1) || fail "push: $out"
[ "$(pub_main)" = "$B" ] || fail "push: public main is not B"
[ "$(g "$WORK/public.git" for-each-ref --format='%(refname)' | sort | tr '\n' ' ')" = "refs/heads/main refs/tags/v1.0.0 " ] \
  || fail "push: unexpected refs on public"
ok "--push fast-forwards main only"
out=$("$SYNC" --run "$RUN" --push 2>&1) || fail "second push: $out"
grep -q "up to date" <<<"$out" || fail "second push: $out"
ok "second push is a no-op"

# The filter no longer reproduces the published SHAs: same content, other dates.
git init -q -b integration "$WORK/drift"
GIT_COMMITTER_DATE="2001-01-01T00:00:00Z" commit "$WORK/drift" "release notes" >/dev/null
commit "$WORK/drift" "fix a typo" >/dev/null
reset_public "$A"
refused "history not reproduced" "not in the filtered history" --run "$(mk_run drift "$WORK/drift")"

# Public main is a filtered commit, but not below the tip.
g "$WORK/filt" checkout -q -b side "$A"
S=$(commit "$WORK/filt" "side change")
g "$WORK/filt" checkout -q integration
reset_public "$S"
refused "not a fast-forward" "not a fast-forward" --run "$(mk_run side "$WORK/filt")"

reset_public "$A"
NOOK=$(mk_run nook "$WORK/filt"); rm "$NOOK/ok"
refused "run without the ok marker" "did not pass" --run "$NOOK"
refused "run of a commit not on origin/integration" "not on refs/remotes/origin/integration" \
  --run "$(mk_run foreign "$WORK/filt" "$(g "$WORK/filt" rev-parse "$S")")"

# A new commit with a non-noreply identity: leak-scan stops the sync.
git clone -q "$WORK/filt" "$WORK/leaky"
GIT_AUTHOR_EMAIL=someone@example.invalid commit "$WORK/leaky" "leaky change" >/dev/null
refused "leak in the new range" "leak-scan" --run "$(mk_run leaky "$WORK/leaky")"

# Tags: a new one on the new main is pushed with it; others are refused.
git clone -q "$WORK/filt" "$WORK/tagged"
g "$WORK/tagged" tag -a v1.0.1 -m "Release 1.0.1" "$B"
g "$WORK/tagged" tag -a v1.0.2 -m "Release 1.0.2" "$S"
TAGGED=$(mk_run tagged "$WORK/tagged")
refused "tag not on the new main" "not on the new main" --run "$TAGGED" --push --tags v1.0.2
refused "tag missing from the run" "not in the filtered repo" --run "$TAGGED" --tags v1.0.9
out=$("$SYNC" --run "$TAGGED" --push --tags v1.0.1 2>&1) || fail "tag push: $out"
[ "$(pub_main)" = "$B" ] || fail "tag push: main"
g "$WORK/public.git" rev-parse --verify -q refs/tags/v1.0.1 >/dev/null || fail "tag push: no v1.0.1"
g "$WORK/public.git" rev-parse --verify -q refs/tags/v1.0.2 >/dev/null && fail "tag push: v1.0.2 pushed"
ok "--tags pushes a new tag on main with the commits"

# A published tag the filtered repo has as another object.
reset_public "$A"
git clone -q "$WORK/filt" "$WORK/retag"
g "$WORK/retag" tag -f -a v1.0.0 -m "Release 1.0.0, again" "$A" >/dev/null
refused "published tag changed" "published tag v1.0.0" --run "$(mk_run retag "$WORK/retag")"

rc=0; "$SYNC" --run "$RUN" --tags 1.2 >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || fail "bad tag name: exit $rc, want 2"
ok "exit 2: bad tag name"
echo "sync_test: all passed"
