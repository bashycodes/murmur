#!/usr/bin/env bash
# ssh-shield.sh ssh|scp ARGS...: run ssh/scp so a terminal Ctrl+C does not
# cut a laptop step in half (an install or restore killed mid-way is worse
# than one that ends). Used by lib/remote.mjs for every ssh/scp call.
#
# The command runs in its own session (setsid), out of reach of the
# terminal's Ctrl+C. Here:
#   first Ctrl+C          the step goes on; the harness cleans up after it
#   second within 3 s     stop the ssh now and exit 86; the harness then
#                         stops the laptop step through its marker pid
#   SIGTERM (a timeout)   stop the ssh and exit 124
set -u
FORCED=86
setsid "$@" &
child=$!
last=""
stop_child() {
  kill -TERM -- "-$child" 2>/dev/null || kill -TERM "$child" 2>/dev/null
}
on_int() {
  local now=${EPOCHREALTIME/./}
  if [[ -n $last ]] && (( now - last <= 3000000 )); then
    echo "second Ctrl+C within 3 s: stopping the laptop step in progress" >&2
    stop_child
    exit "$FORCED"
  fi
  last=$now
  echo "Ctrl+C: letting the laptop step finish first (Ctrl+C again within 3 s to stop it)" >&2
}
trap on_int INT
trap 'stop_child; exit 124' TERM HUP
# `wait` returns early when a trap runs: wait until the child is gone, then
# once more for its real status (bash keeps it for a repeated wait).
while kill -0 "$child" 2>/dev/null; do wait "$child"; done
wait "$child"
exit $?
