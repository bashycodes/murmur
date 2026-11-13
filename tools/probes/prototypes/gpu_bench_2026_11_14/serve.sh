#!/usr/bin/env bash
# Start llama-server for one model on the Windows PC (pc/serve.ps1 through
# the e2e interactive-task launcher) and wait until it listens, or stop it.
#
#   serve.sh start TAG MODEL.gguf [ngram|none] [extra,flags,comma,separated]
#   serve.sh stop TAG
#   serve.sh pull            # copy the PC's logs into pc-logs/
#
# env: PC (user@host). The bench folder on the PC is %USERPROFILE%\murmur-bench
# with the llama.cpp release unpacked in llama-b12368\ and models\.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
: "${PC:?set PC=user@host}"
repo="$here/../../../.."
case $1 in
  start)
    TAG=$2; MODEL=$3; SPEC=${4:-none}; EXTRA=${5:-}
    scp -q "$here/pc/serve.ps1" "$here/pc/stop.ps1" "$repo/tools/e2e-windows/ps/itask.ps1" "$PC:murmur-bench/"
    ssh "$PC" "cd murmur-bench && powershell -NoProfile -ExecutionPolicy Bypass -File itask.ps1 -Script serve.ps1 -Name MurmurBench-serve -Arguments \"-Model $MODEL -Tag $TAG -Spec $SPEC${EXTRA:+ -Extra $EXTRA}\""
    for _ in $(seq 120); do
      if "$here/pcps.sh" "Select-String -Quiet -Pattern 'listening on' -Path \$env:USERPROFILE\\murmur-bench\\logs\\$TAG-server.err" 2>/dev/null | grep -q True; then
        echo "listening $TAG"; exit 0
      fi
      sleep 5
    done
    echo "timeout waiting for $TAG" >&2; exit 1 ;;
  stop)
    scp -q "$here/pc/stop.ps1" "$PC:murmur-bench/"
    ssh "$PC" "cd murmur-bench && powershell -NoProfile -ExecutionPolicy Bypass -File stop.ps1 -Tag $2" ;;
  pull)
    mkdir -p "$here/pc-logs"
    scp -q "$PC:murmur-bench/logs/*" "$here/pc-logs/" ;;
esac
