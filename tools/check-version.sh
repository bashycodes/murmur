#!/usr/bin/env bash
# Version gate (D26, docs/versioning.md): every copy of the product version
# equals the top-level VERSION file, Android/Tauri derive it instead of
# pinning a literal, and (with --tag) a release tag matches VERSION and has a
# CHANGELOG section. Runs the rules' unit tests first, so a broken checker
# cannot pass a broken tree.
#
# Usage: tools/check-version.sh [--tag vX.Y.Z | --tag refs/tags/vX.Y.Z] [--root DIR]
#        (from anywhere; exit 1 on a mismatch). Called by CI (both workflows),
#        tools/check-build-matrix.sh and tools/release.sh.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
if ! out=$(python3 -I -m unittest discover -s "$here/versioning" 2>&1); then
  echo "$out" >&2
  echo "check-version: the version rules' own tests fail (tools/versioning/test_versioning.py)" >&2
  exit 1
fi
echo "ok    version rules: $(grep -E '^Ran ' <<<"$out")"
exec python3 -I "$here/versioning/versioning.py" check "$@"
