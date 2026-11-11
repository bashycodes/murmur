#!/usr/bin/env bash
# filter.sh: rewrite the history for the public repo in a scratch mirror,
# then verify and rescan it. Promoted from the 2026-11-09 prototype
# (tools/probes/prototypes/history_filter_2026_11_09/run.sh); sync.sh runs it
# before every push (docs/public-release/sync.md).
#
#   filter.sh [SOURCE_GITDIR]     default: this checkout's common git dir
#   SOURCE_REF=feat/x filter.sh   filter that branch as if it were integration
#   SOURCE_REF=refs/remotes/origin/integration filter.sh   (what sync.sh does)
#
# Makes RUN=$MURMUR_FILTER_BASE/work/<UTC timestamp>/ (default base
# ~/.cache/murmur-filter; never reused, so a run never overwrites another):
#   orig/       untouched local mirror, integration + tags only (reference)
#   repo/       the same, rewritten by git filter-repo
#   inputs/     per-run filter inputs (mode 600: they hold private values)
#   verify.txt  verify.py output     scan.txt / scan.json  scan.py output
#   source      the integration commit that was filtered
#   ok          written last, only when verify and scan both passed
# Private replacements: <base>/private/replacements.txt (uncommitted,
# filter-repo replace-text format). The mirror has no remote; nothing here
# pushes. The last line on stdout is `run: <dir> verify=<rc> scan=<rc>`.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SRC=${1:-$(cd "$HERE" && cd "$(git rev-parse --git-common-dir)" && pwd)}
. "$HERE/common.sh"
RUN=$BASE/work/$(date -u +%Y%m%dT%H%M%SZ)
[[ -f $PRIVATE ]] || { echo "filter.sh: missing $PRIVATE" >&2; exit 2; }
# filter-repo holds the whole history's blobs in memory; ~2 GB keeps it clear
# of the OOM killer on this shared machine.
MIN_AVAIL_MB=2000
avail=$(free -m | awk '/^Mem:/ {print $7}')
(( avail >= MIN_AVAIL_MB )) || { echo "filter.sh: only ${avail} MB available, need $MIN_AVAIL_MB" >&2; exit 2; }

mkdir -p "$BASE/work"
mkdir -m 700 "$RUN"   # fails if it exists: runs never overwrite each other
mkdir -m 700 "$RUN/inputs"
echo "run: $RUN"
nice -n 10 "$HERE/mirror.sh" "$SRC" "$RUN/orig"
nice -n 10 env -u SOURCE_REF "$HERE/mirror.sh" "$RUN/orig" "$RUN/repo"
git -C "$RUN/orig" rev-parse refs/heads/integration > "$RUN/source"

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
[[ -z $(git -C "$RUN/repo" remote) ]] || { echo "filter.sh: remote present" >&2; exit 1; }
git -C "$RUN/repo" for-each-ref --format='delete %(refname)' refs/replace/ | git -C "$RUN/repo" update-ref --stdin

set +e
python3 "$HERE/verify.py" --orig "$RUN/orig" --new "$RUN/repo" \
  --strip-blobs "$RUN/inputs/strip-blobs.txt" --replace-message "$RUN/inputs/replace-message.txt" > "$RUN/verify.txt"
v=$?
grep -e '^FAIL' -e '^verify:' "$RUN/verify.txt"
nice -n 10 python3 "$HERE/scan.py" --repo "$RUN/repo" --private "$PRIVATE" --private-literals "$HOSTS" \
  --accepted "$HERE/accepted.txt" --json "$RUN/scan.json" > "$RUN/scan.txt"
s=$?
set -e
grep -v '^    ' "$RUN/scan.txt" | grep -v -e '^\[accepted\]' -e '^\[info\]' || true
(( v || s )) || echo ok > "$RUN/ok"
echo "run: $RUN verify=$v scan=$s"
exit $(( v || s ))
