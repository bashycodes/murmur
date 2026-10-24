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
# python3 on Linux/macOS; on Windows (Git Bash) `python`, since `python3` may be
# the Microsoft Store stub. Whichever is picked must actually run.
py=""
if [[ ${OS:-} == Windows_NT ]]; then cands=(python python3); else cands=(python3 python); fi
for c in "${cands[@]}"; do
  if command -v "$c" >/dev/null && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 9))' 2>/dev/null; then py=$c; break; fi
done
[[ -n $py ]] || { echo "check-version: no working python3 (>= 3.9) found" >&2; exit 1; }
if ! out=$("$py" -I -m unittest discover -s "$here/versioning" 2>&1); then
  echo "$out" >&2
  echo "check-version: the version rules' own tests fail (tools/versioning/test_versioning.py)" >&2
  exit 1
fi
echo "ok    version rules: $(grep -E '^Ran ' <<<"$out")"
exec "$py" -I "$here/versioning/versioning.py" check "$@"
