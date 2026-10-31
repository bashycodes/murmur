#!/bin/bash
# flow-server liveness (docs/container.md "Health checks"). /v1/health needs
# a key, so this only checks that the port accepts a TCP connection; nothing
# is sent. Also healthy while the entrypoint waits for the first key, and
# for FLOW_START_GRACE_S (default 600) after the server process started
# (models load before the listener binds).
RUN_DIR="${FLOW_RUN_DIR:-/tmp}"
PORT="${FLOW_PORT:-7313}"

tcp_ok() { (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; }

[ -e "$RUN_DIR/flow-server.waiting-for-keys" ] && exit 0
tcp_ok && exit 0
started="$(cat "$RUN_DIR/flow-server.started" 2>/dev/null)" || exit 1
[ $(( $(date +%s) - started )) -lt "${FLOW_START_GRACE_S:-600}" ]
