#!/usr/bin/env bash
# Dated probe (2026-11-07) for docs/findings/2026-11-07-phone-beam-hotwords.md:
# peak RSS of one `flow-cli transcribe` per Parakeet decoding mode (greedy /
# beam / auto = beam + a second, greedy recognizer for the fallback). Not a
# permanent tool.
#
#   CLI=<release flow-cli, feature parakeet> MODEL=<parakeet dir> CLIP=<wav>
#   DICT=<dictionary JSON> SESSION=<session JSON>  ram_modes.sh
set -euo pipefail
: "${CLI:?}" "${MODEL:?}" "${CLIP:?}" "${DICT:?}" "${SESSION:?}"
for mode in greedy beam auto; do
  rss=$( { /usr/bin/time -f '%M' "$CLI" transcribe --json --level none --session "$SESSION" \
      --engine parakeet --model "$MODEL" --threads 4 --parakeet-decoding "$mode" --dict "$DICT" \
      "$CLIP" >/dev/null; } 2>&1 | tail -1)
  echo "$mode max_rss_kb=$rss"
done
