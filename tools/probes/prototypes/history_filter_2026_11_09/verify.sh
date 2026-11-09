#!/usr/bin/env bash
# verify.sh RUNDIR: rerun verify.py on a run made by run.sh.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
exec python3 "$HERE/verify.py" --orig "$1/orig" --new "$1/repo"
