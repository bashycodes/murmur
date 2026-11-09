#!/usr/bin/env bash
# Probe (stray "yeah", 2026-11-07): run one WAV through murmur-cli with the
# PC's settings (Parakeet unified-en, Windows platform, segmented commit, no
# cleanup) and write the JSON result next to the WAV.
#
# Usage: run_cli.sh <murmur-cli dir> <parakeet model dir> <wav> [extra murmur-cli args...]
set -euo pipefail
bin="$1"; model="$2"; wav="$3"; shift 3
export LD_LIBRARY_PATH="$bin"
nice -n 10 "$bin/murmur-cli" transcribe --engine parakeet --model "$model" \
  --platform windows --level none --threads 2 --json "$@" "$wav" \
  >"${wav%.wav}.cli.json" 2>"${wav%.wav}.cli.err"
python3 -I - "${wav%.wav}.cli.json" <<'EOF'
import json, sys
j = json.load(open(sys.argv[1]))
t = j.get("asr_text") or j.get("text") or ""
meta = {k: v for k, v in j.items() if not isinstance(v, str) or len(v) < 40}
print(meta)
print("last words:", " ".join(t.split()[-3:]))
EOF
