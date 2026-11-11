#!/usr/bin/env bash
# mirror.sh SRC_GITDIR DEST: a local `git clone --mirror` that keeps only
# refs/heads/integration and refs/tags/*, with no remote. Never GitHub.
set -euo pipefail
src=$1 dest=$2
[[ -e $dest ]] && { echo "mirror.sh: $dest exists, refusing to overwrite" >&2; exit 2; }
git clone --quiet --mirror --no-local "$src" "$dest"
git -C "$dest" remote remove origin >/dev/null 2>&1
# SOURCE_REF stands in for integration: a branch name (refs/heads/ is
# implied; a feat/ branch gives a dry run of integration once it is merged)
# or a full ref such as refs/remotes/origin/integration (what sync.sh uses).
if [[ -n ${SOURCE_REF:-} ]]; then
  ref=$SOURCE_REF
  [[ $ref == refs/* ]] || ref=refs/heads/$ref
  # Fetched from SRC by name: a mirror clone does not copy refs/remotes/.
  git -C "$dest" fetch --quiet --no-tags "$src" "+$ref:refs/heads/integration"
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
