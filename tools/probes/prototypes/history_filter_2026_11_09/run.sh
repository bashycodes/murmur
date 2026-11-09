#!/usr/bin/env bash
# Public B: rewrite the history for the public repo in a scratch mirror.
#
#   run.sh [SOURCE_GITDIR]     default: this checkout's common git dir
#
# Makes RUN=~/.cache/murmur-filter/work/<UTC timestamp>/ (never reused, so a
# run never overwrites another) with:
#   orig/       untouched local mirror, integration + tags only (reference)
#   repo/       the same, rewritten by git filter-repo
#   inputs/     per-run filter inputs (mode 600: they hold private values)
#   verify.txt  verify.py output     scan.txt / scan.json  scan.py output
# Private replacements: ~/.cache/murmur-filter/private/replacements.txt
# (uncommitted, filter-repo replace-text format). The mirror has no remote;
# nothing here pushes. See history-filter-report.md for the push step (C).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SRC=${1:-$(cd "$HERE" && cd "$(git rev-parse --git-common-dir)" && pwd)}
BASE=$HOME/.cache/murmur-filter
PRIVATE=$BASE/private/replacements.txt
HOSTS=$BASE/private/.hosts
RUN=$BASE/work/$(date -u +%Y%m%dT%H%M%SZ)
[[ -f $PRIVATE ]] || { echo "run.sh: missing $PRIVATE" >&2; exit 2; }
avail=$(free -m | awk '/^Mem:/ {print $7}')
(( avail >= 2000 )) || { echo "run.sh: only ${avail} MB available" >&2; exit 2; }

mkdir -m 700 "$RUN"   # fails if it exists: runs never overwrite each other
mkdir -m 700 "$RUN/inputs"
echo "run: $RUN"
nice -n 10 "$HERE/mirror.sh" "$SRC" "$RUN/orig"
nice -n 10 "$HERE/mirror.sh" "$RUN/orig" "$RUN/repo"

python3 "$HERE/prepare.py" --repo "$RUN/repo" --out "$RUN/inputs" --private "$PRIVATE"

cd "$RUN/repo"
nice -n 10 git filter-repo --force --quiet \
  --paths-from-file "$HERE/paths-drop.txt" --invert-paths \
  --strip-blobs-with-ids "$RUN/inputs/strip-blobs.txt" \
  --replace-text "$RUN/inputs/replace-text.txt" \
  --replace-message "$RUN/inputs/replace-message.txt" \
  --mailmap "$RUN/inputs/mailmap"
cd "$HERE"
# filter-repo leaves no remote; make sure, and drop its replace refs.
[[ -z $(git -C "$RUN/repo" remote) ]] || { echo "run.sh: remote present" >&2; exit 1; }
git -C "$RUN/repo" for-each-ref --format='delete %(refname)' refs/replace/ | git -C "$RUN/repo" update-ref --stdin

set +e
python3 "$HERE/verify.py" --orig "$RUN/orig" --new "$RUN/repo" | tee "$RUN/verify.txt"
v=${PIPESTATUS[0]}
nice -n 10 python3 "$HERE/scan.py" --repo "$RUN/repo" --private "$PRIVATE" --private-literals "$HOSTS" \
  --accepted "$HERE/accepted.txt" --json "$RUN/scan.json" > "$RUN/scan.txt"
s=$?
set -e
grep -v '^    ' "$RUN/scan.txt"
echo "run: $RUN verify=$v scan=$s"
exit $(( v || s ))
