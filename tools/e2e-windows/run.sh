#!/usr/bin/env bash
# Murmur Windows e2e harness: see docs/testing/windows-e2e.md.
#   tools/e2e-windows/run.sh --installer Murmur_1.3.0_x64-setup_hybrid.exe [--scenarios a,b,...]
#   tools/e2e-windows/run.sh restore      # put the owner's data back after --no-restore or a crash
# Exit (lib/verdict.mjs): 0 all scenarios passed and the owner's data is
# back; 3 a scenario failed / errored / aborted, a clean-up step failed, or
# the restore did not confirm; 1 the run itself failed; 2 bad arguments;
# 4 the laptop is busy (--no-wait, or the wait timed out); 5 --no-restore
# (data left in test state; see results.json "pass"); 130 interrupted
# (SIGINT/SIGTERM/SIGHUP: the clean-up ran first).
# The laptop host: $MURMUR_E2E_HOST, or MURMUR_E2E_HOST=user@host in
# tools/e2e-windows/local.env (gitignored).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"

# Runs and restores touch the laptop: one at a time from this server (the
# harness also takes a lock on the laptop, for other machines).
case "${1:-run}" in
  status | clips | selftest | -h | --help) exec node "$here/harness.mjs" "$@" ;;
esac
lock="${MURMUR_E2E_LOCK:-$HOME/.cache/murmur-e2e-laptop.lock}"
mkdir -p "$(dirname "$lock")"
exec 9>"$lock"
if ! flock -n 9; then
  if [[ " $* " == *" --no-wait "* ]]; then
    echo "another e2e run holds $lock" >&2
    exit 4
  fi
  echo "another e2e run holds $lock: waiting" >&2
  flock 9
fi
exec node "$here/harness.mjs" "$@"
