#!/bin/sh
# Run a local PowerShell script on the laptop over ssh (EncodedCommand, UTF-16LE).
# usage: LAPTOP_SSH=user@host rps.sh script.ps1 ; stderr (CLIXML) errors shown as "ERR: ..."
ENC=$(iconv -t UTF-16LE "$1" | base64 -w0)
ssh "$LAPTOP_SSH" "powershell -NoProfile -NonInteractive -EncodedCommand $ENC" 2>&1 | sed 's/_x000D__x000A_//g' | awk '
  /^#< CLIXML/ {next}
  /^<Objs/ { while (match($0, /<S S="Error">[^<]*/)) { print "ERR: " substr($0, RSTART+13, RLENGTH-13); $0 = substr($0, RSTART+RLENGTH) } next }
  {print}'
