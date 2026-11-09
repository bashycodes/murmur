#!/bin/sh
# flow-server launcher for the colleague bundle: installed as
# /usr/local/bin/flow-server (the path the systemd unit runs), it finds the
# speech-engine libraries next to the binary. FLOW_SERVER_HOME overrides
# the install folder (default /opt/flow-server: bin/ and lib/).
home="${FLOW_SERVER_HOME:-/opt/flow-server}"
LD_LIBRARY_PATH="$home/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" exec "$home/bin/flow-server" "$@"
