#!/usr/bin/env bash
# Run a PowerShell snippet on the Windows PC over SSH without cmd.exe
# quoting: pcps.sh 'Get-Process llama-server'. Host from $PC (user@host).
set -euo pipefail
: "${PC:?set PC=user@host}"
enc=$(printf '$ProgressPreference="SilentlyContinue"; %s' "$1" | iconv -t UTF-16LE | base64 -w0)
exec ssh -o ConnectTimeout=15 "$PC" "powershell -NoProfile -NonInteractive -EncodedCommand $enc"
