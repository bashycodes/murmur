#!/usr/bin/env bash
# FROZEN: promoted to tools/public/ (filter.sh, sync.sh; docs/public-release/sync.md). This copy is the audit trail of the 2026-11-09 first publish; change tools/public/ instead.
# scan.sh REPO [scan.py args]: the three scans (secrets, personal,
# wispr-legal) over every blob, message and identity of REPO's history.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/common.sh"
repo=$1
shift
exec nice -n 10 python3 "$HERE/scan.py" --repo "$repo" --private "$PRIVATE" \
  --private-literals "$HOSTS" --accepted "$HERE/accepted.txt" "$@"
