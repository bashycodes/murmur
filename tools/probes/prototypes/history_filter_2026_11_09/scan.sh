#!/usr/bin/env bash
# scan.sh REPO [scan.py args]: the three scans (secrets, personal,
# wispr-legal) over every blob, message and identity of REPO's history.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
P=$HOME/.cache/murmur-filter/private
repo=$1
shift
exec nice -n 10 python3 "$HERE/scan.py" --repo "$repo" --private "$P/replacements.txt" \
  --private-literals "$P/.hosts" --accepted "$HERE/accepted.txt" "$@"
