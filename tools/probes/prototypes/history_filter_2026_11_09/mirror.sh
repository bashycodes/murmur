#!/usr/bin/env bash
# FROZEN: promoted to tools/public/ (filter.sh, sync.sh; docs/public-release/sync.md). This copy is the audit trail of the 2026-11-09 first publish; change tools/public/ instead.
# mirror.sh SRC_GITDIR DEST: a local `git clone --mirror` that keeps only
# refs/heads/integration and refs/tags/*, with no remote. Never GitHub.
set -euo pipefail
src=$1 dest=$2
[[ -e $dest ]] && { echo "mirror.sh: $dest exists, refusing to overwrite" >&2; exit 2; }
git clone --quiet --mirror --no-local "$src" "$dest"
git -C "$dest" remote remove origin >/dev/null 2>&1
# SOURCE_REF (e.g. a feat/ branch) stands in for integration: dry runs of
# what integration will be once that branch is merged.
if [[ -n ${SOURCE_REF:-} ]]; then
  git -C "$dest" update-ref refs/heads/integration "$(git -C "$dest" rev-parse --verify "refs/heads/$SOURCE_REF^{commit}")"
fi
git -C "$dest" for-each-ref --format='delete %(refname)' \
  | { grep -v -e ' refs/heads/integration$' -e ' refs/tags/' || true; } \
  | git -C "$dest" update-ref --stdin
left=$(git -C "$dest" for-each-ref --format='%(refname)' | grep -v -e '^refs/heads/integration$' -e '^refs/tags/' || true)
[[ -z $left ]] || { echo "mirror.sh: unexpected refs left: $left" >&2; exit 1; }
git -C "$dest" symbolic-ref HEAD refs/heads/integration
[[ -z $(git -C "$dest" remote) ]] || { echo "mirror.sh: remote still set" >&2; exit 1; }
# Unreachable objects from the deleted refs must not survive in the scratch repo.
git -C "$dest" reflog expire --expire=now --all
git -C "$dest" gc --quiet --prune=now
