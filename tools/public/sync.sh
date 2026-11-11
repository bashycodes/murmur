#!/usr/bin/env bash
# sync.sh: publish the private integration branch to the public repo's main
# as a pure fast-forward of filtered commits (docs/public-release/sync.md).
#
#   sync.sh                       filter origin/integration, check, dry run
#   sync.sh --push                the same, then push the new commits
#   sync.sh --run DIR [--push]    reuse a passed filter.sh run, no refilter
#   sync.sh ... --tags v1.2.0[,v1.2.1]   also publish these new tags
#
# Every sync, dry run or not:
#   1. filter.sh reruns the history filter on refs/remotes/origin/integration
#      (fetched first) and must pass verify + the full-history scan.
#   2. The published main (ls-remote) must be a commit of the filtered history
#      and an ancestor of the filtered tip. The filter is deterministic, so a
#      missing commit means the rules, the private replacements or
#      filter-repo changed: refuse, nothing is rewritten on the public side.
#   3. Every published tag must be the same tag object in the filtered repo.
#   4. tools/checks/leak-scan.py over published..new (lines, messages,
#      identities) must be clean.
#   5. A requested tag must be new, and its commit on the new main.
# Then a dry run prints the range; --push runs exactly
#   git push --atomic public <tip>:refs/heads/main [refs/tags/vX.Y.Z ...]
# with no --force (the server refuses a non-fast-forward too).
#
# Env: MURMUR_PUBLIC_URL (read, default the https URL), MURMUR_PUBLIC_PUSH_URL
# (default the ssh URL), MURMUR_SOURCE_GITDIR (default this checkout's common
# git dir), MURMUR_FILTER_BASE (see common.sh).
# Exit 0 done or nothing to do, 1 refused, 2 usage or setup error.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
PUBLIC_URL=${MURMUR_PUBLIC_URL:-https://github.com/bashycodes/murmur.git}
PUSH_URL=${MURMUR_PUBLIC_PUSH_URL:-git@github.com:bashycodes/murmur.git}
SRC=${MURMUR_SOURCE_GITDIR:-$(cd "$HERE" && cd "$(git rev-parse --git-common-dir)" && pwd)}
SOURCE_REF=refs/remotes/origin/integration

usage() { sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }
refuse() { echo "sync.sh: REFUSED: $*" >&2; echo "sync.sh: nothing pushed" >&2; exit 1; }

push=0 run="" tags=()
while (( $# )); do
  case $1 in
    --push) push=1 ;;
    --run) run=${2:?--run needs a directory}; shift ;;
    --tags) IFS=, read -ra more <<<"${2:?--tags needs vX.Y.Z}"; tags+=("${more[@]}"); shift ;;
    -h|--help) usage ;;
    *) echo "sync.sh: unknown argument $1" >&2; usage ;;
  esac
  shift
done
for t in "${tags[@]}"; do
  [[ $t =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "sync.sh: bad tag $t (want vX.Y.Z)" >&2; exit 2; }
done

# 1. A passed filter run.
if [[ -z $run ]]; then
  git -C "$SRC" fetch --quiet origin integration
  log=$(SOURCE_REF=$SOURCE_REF "$HERE/filter.sh" "$SRC" | tee /dev/stderr) \
    || refuse "the filter run failed verify or scan (see its verify.txt / scan.txt)"
  run=$(sed -n 's/^run: \([^ ]*\) verify=.*/\1/p' <<<"$log" | tail -1)
fi
[[ -d $run/repo ]] || { echo "sync.sh: $run is not a filter run (no repo/)" >&2; exit 2; }
[[ -f $run/ok ]] || refuse "$run did not pass verify + scan (no ok marker)"
repo=$run/repo
src_sha=$(cat "$run/source")
# A run of a feat/ branch (SOURCE_REF=feat/x filter.sh) is a preview only.
git -C "$SRC" merge-base --is-ancestor "$src_sha" "$SOURCE_REF" 2>/dev/null \
  || refuse "the run filtered ${src_sha:0:10}, which is not on $SOURCE_REF"
new=$(git -C "$repo" rev-parse --verify 'refs/heads/integration^{commit}')
cur=$(git -C "$SRC" rev-parse "$SOURCE_REF")
[[ $cur == "$src_sha" ]] || echo "note: $SOURCE_REF is now ${cur:0:10}; this run filtered ${src_sha:0:10}"

# 2. Published main is in the filtered history, below the new tip.
remote=$(git ls-remote "$PUBLIC_URL" refs/heads/main 'refs/tags/*') || { echo "sync.sh: cannot read $PUBLIC_URL" >&2; exit 2; }
pub=$(awk '$2 == "refs/heads/main" {print $1}' <<<"$remote")
[[ -n $pub ]] || refuse "the public repo has no main; the first publish is manual (history-filter-report.md, Step C)"
git -C "$repo" cat-file -e "$pub^{commit}" 2>/dev/null \
  || refuse "published main ${pub:0:10} is not in the filtered history: the filter no longer reproduces the published SHAs (rules, private replacements or filter-repo changed). Recovery: docs/public-release/sync.md"
git -C "$repo" merge-base --is-ancestor "$pub" "$new" \
  || refuse "published main ${pub:0:10} is not an ancestor of the filtered tip ${new:0:10}: not a fast-forward"

# 3. Published tags are unchanged.
while read -r sha ref; do
  [[ $ref == refs/tags/* && $ref != *'^{}' ]] || continue
  mine=$(git -C "$repo" rev-parse --verify --quiet "$ref" || true)
  [[ $mine == "$sha" ]] || refuse "published tag ${ref#refs/tags/} is ${sha:0:10}, the filtered repo has ${mine:-nothing}: not reproduced"
done <<<"$remote"

# 4. Leak scan of what would be new.
count=$(git -C "$repo" rev-list --count "$pub..$new")
if (( count )); then
  (cd "$repo" && python3 "$ROOT/tools/checks/leak-scan.py" "$pub..$new") \
    || refuse "leak-scan found hits in ${pub:0:10}..${new:0:10}"
fi

# 5. Requested tags: new, and on the new main.
refspecs=()
(( count )) && refspecs+=("$new:refs/heads/main")
for t in "${tags[@]}"; do
  grep -q "[[:space:]]refs/tags/$t\$" <<<"$remote" && { echo "tag $t: already published, skipped"; continue; }
  tc=$(git -C "$repo" rev-parse --verify --quiet "refs/tags/$t^{commit}") || refuse "tag $t is not in the filtered repo"
  git -C "$repo" merge-base --is-ancestor "$tc" "$new" || refuse "tag $t (${tc:0:10}) is not on the new main"
  refspecs+=("refs/tags/$t:refs/tags/$t")
done

echo "public main: ${pub:0:10}   filtered tip: ${new:0:10} (from integration ${src_sha:0:10})"
echo "range ${pub:0:10}..${new:0:10}: $count new commit(s)"
(( count )) && git -C "$repo" log --oneline --first-parent "$pub..$new" | head -20
unpub=$(git -C "$repo" tag --merged "$new" --list 'v*' | while read -r t; do
  grep -q "[[:space:]]refs/tags/$t\$" <<<"$remote" || printf '%s ' "$t"; done)
[[ -z $unpub ]] || echo "unpublished tags on the new main: $unpub(publish with --tags)"
if (( ! ${#refspecs[@]} )); then
  echo "up to date: nothing to push"
  exit 0
fi
echo "would push: ${refspecs[*]}"
if (( ! push )); then
  echo "dry run: nothing pushed. To push this run: $0 --run $run --push${tags:+ --tags $(IFS=,; echo "${tags[*]}")}"
  exit 0
fi

git -C "$repo" remote remove public 2>/dev/null || true
git -C "$repo" remote add public "$PUSH_URL"
git -C "$repo" push --atomic public "${refspecs[@]}"
after=$(git ls-remote "$PUSH_URL" refs/heads/main | awk '{print $1}')
[[ $after == "$new" || $count == 0 ]] || { echo "sync.sh: pushed, but public main reads ${after:0:10}" >&2; exit 1; }
echo "pushed: public main is ${new:0:10}"
