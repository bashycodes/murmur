#!/bin/sh
# murmur-server launcher for the colleague bundle: installed as
# /usr/local/bin/murmur-server (the path the systemd unit runs), it finds the
# speech-engine libraries next to the binary. MURMUR_SERVER_HOME overrides
# the install folder (default /opt/murmur-server: bin/ and lib/).
home="${MURMUR_SERVER_HOME:-/opt/murmur-server}"
LD_LIBRARY_PATH="$home/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" exec "$home/bin/murmur-server" "$@"
