#!/usr/bin/env bash
# FROZEN: promoted to tools/public/ (filter.sh, sync.sh; docs/public-release/sync.md). This copy is the audit trail of the 2026-11-09 first publish; change tools/public/ instead.
# verify.sh RUNDIR: rerun verify.py on a run made by run.sh.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
exec python3 "$HERE/verify.py" --orig "$1/orig" --new "$1/repo" \
  --strip-blobs "$1/inputs/strip-blobs.txt" --replace-message "$1/inputs/replace-message.txt"
