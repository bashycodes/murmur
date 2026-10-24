#!/usr/bin/env bash
# Cut a release (D26, docs/versioning.md): bump the single product version,
# date the CHANGELOG's [Unreleased] notes, verify every derived copy, commit
# "Release X.Y.Z" and create the annotated tag vX.Y.Z. LOCAL ONLY: it never
# pushes; it prints the push command at the end.
#
# Usage: tools/release.sh X.Y.Z            dry run (default): shows the diff, changes nothing
#        tools/release.sh X.Y.Z --apply    stamp, commit, tag
#
# Refuses: a version that is not MAJOR.MINOR.PATCH or not newer than VERSION,
# an existing tag, an empty [Unreleased], a tree whose copies already
# disagree, and (with --apply) uncommitted changes to tracked files or a
# branch other than `integration` (FLOW_RELEASE_ANY_BRANCH=1 allows one).
# Env: FLOW_RELEASE_DATE=YYYY-MM-DD (default today, UTC);
#      FLOW_REPO_URL=https://… adds Keep-a-Changelog compare links.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

die() { echo "release: $*" >&2; exit 1; }
warn() { echo "release: warning: $*" >&2; }
py() { python3 -I "$root/tools/versioning/versioning.py" "$@"; }

[[ $# -ge 1 && $# -le 2 ]] || die "usage: tools/release.sh X.Y.Z [--apply]"
new=$1
apply=0
if [[ $# -eq 2 ]]; then
  [[ $2 == --apply ]] || die "unknown option $2 (only --apply)"
  apply=1
fi

py code "$new" >/dev/null || die "invalid version $new"
cur=$(py current)
py is-newer "$new" "$cur" || die "$new is not newer than the current VERSION $cur"
tag="v$new"
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then die "tag $tag already exists"; fi

branch=$(git symbolic-ref --short -q HEAD || echo "(detached)")
if [[ $branch != integration ]]; then
  msg="on branch $branch, releases are tagged on integration"
  if [[ $apply == 1 && -z ${FLOW_RELEASE_ANY_BRANCH:-} ]]; then die "$msg (FLOW_RELEASE_ANY_BRANCH=1 to override)"; fi
  warn "$msg"
fi
if [[ -n $(git status --porcelain --untracked-files=no) ]]; then
  if [[ $apply == 1 ]]; then die "uncommitted changes to tracked files; commit or stash them first"; fi
  warn "uncommitted changes to tracked files (--apply will refuse)"
fi

tools/check-version.sh >/dev/null || die "the version copies already disagree (tools/check-version.sh)"
[[ -n $(py unreleased) ]] || die "CHANGELOG.md [Unreleased] is empty: write the release notes first"
date=${FLOW_RELEASE_DATE:-$(date -u +%F)}
[[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die "FLOW_RELEASE_DATE must be YYYY-MM-DD"
cl_args=()
[[ -n ${FLOW_REPO_URL:-} ]] && cl_args=(--repo-url "$FLOW_REPO_URL")

# Every file the stamp/changelog step may touch (= tools/versioning/versioning.py collect()).
files=(VERSION Cargo.toml Cargo.lock CHANGELOG.md
  apps/flow-windows/Cargo.toml apps/flow-windows/Cargo.lock apps/flow-windows/tauri.conf.json
  apps/flow-windows/ui/package.json apps/flow-windows/ui/package-lock.json
  apps/flow-android/app/build.gradle.kts)

echo "release: $cur -> $new (tag $tag, $date, versionCode $(py code "$new"))"

if [[ $apply == 0 ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/a" "$tmp/b"
  for f in "${files[@]}"; do
    [[ -f $f ]] || continue
    mkdir -p "$tmp/a/$(dirname "$f")" "$tmp/b/$(dirname "$f")"
    cp "$f" "$tmp/a/$f"
    cp "$f" "$tmp/b/$f"
  done
  py stamp "$new" --root "$tmp/b"
  py changelog-release "$new" "$date" --root "$tmp/b" "${cl_args[@]}"
  py check --root "$tmp/b" --tag "$tag" >/dev/null || { py check --root "$tmp/b" --tag "$tag"; die "the stamped tree would fail the version check"; }
  (cd "$tmp" && git --no-pager diff --no-index --stat a b || true)
  (cd "$tmp" && git --no-pager diff --no-index -U1 a b || true) | sed -n '1,200p'
  echo
  echo "release notes for $tag:"
  py section "$new" --root "$tmp/b" | sed 's/^/  /'
  echo
  echo "dry run: nothing was changed. Re-run with --apply to commit and tag."
  exit 0
fi

py stamp "$new"
py changelog-release "$new" "$date" "${cl_args[@]}"
tools/check-version.sh --tag "$tag" >/dev/null || {
  tools/check-version.sh --tag "$tag" || true
  die "the stamped tree fails the version check; inspect, then 'git checkout -- .' to undo"
}
# The lockfiles must still be accepted as-is by cargo (CI builds --locked).
# --filter-platform: offline, only this host's crates have to be in the cache.
if command -v cargo >/dev/null; then
  host=$(rustc -vV | sed -n 's/^host: //p')
  for d in . apps/flow-windows; do
    (cd "$d" && cargo metadata --locked --offline --format-version 1 --filter-platform "$host" >/dev/null) \
      || die "cargo rejects $d/Cargo.lock after the stamp; 'git checkout -- .' to undo"
  done
fi
notes=$(py section "$new")
git add -- "${files[@]}"
git commit -q -m "Release $new" -m "$notes"
git tag -a "$tag" -m "Flow Clone $new" -m "$notes"
echo "release: committed $(git rev-parse --short HEAD) and tagged $tag (local only)."
echo "next: git push origin $branch $tag   # CI builds and attaches the $tag artifacts"
