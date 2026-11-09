#!/usr/bin/env bash
# Probe (stray "yeah", 2026-11-07, D72 parity check): run the parity sets
# through one flow-cli build (before or after the fix) with the PC and phone
# engine configs, via parity_ab.py. All data paths come from the
# environment; rows land in $OUT/<label>/<set>-<config>.jsonl.
#
#   CLI=<flow-cli, feature parakeet> PARITY=<wispr-flow eval dir: cfg/, clips/>
#   EN=<en-set WAV dir> PC=<PC recordings WAV dir> HE=<FLEURS he WAV dir>
#   UNI=<parakeet unified-en model> HEM=<parakeet he model> OUT=<private dir>
#   parity_sets.sh <label>
set -uo pipefail
: "${CLI:?}" "${PARITY:?}" "${EN:?}" "${PC:?}" "${HE:?}" "${UNI:?}" "${HEM:?}" "${OUT:?}"
here="$(cd "$(dirname "$0")" && pwd)"
label="$1"; o="$OUT/$label"; mkdir -p "$o"
cfg="$PARITY/cfg"
run() { # set clips session out-name args...
  local clips=$1 session=$2 name=$3; shift 3
  choom -n 1000 -- nice -n 10 python3 -I "$here/parity_ab.py" "$CLI" "$session" "$clips" "$o/$name.jsonl" \
    --engine parakeet --threads 2 "$@"
}
run "$PARITY/clips" "$cfg/session-pc.json" main-pc --model "$UNI" --parakeet-decoding auto --dict "$cfg/dict-owner-session.json"
run "$PARITY/clips" "$cfg/session-phone.json" main-phone --model "$UNI" --parakeet-decoding beam --dict "$cfg/dict-owner-session.json"
run "$EN" "$cfg/session-pc.json" en-pc --model "$UNI" --parakeet-decoding auto --dict "$cfg/dict-en-set.json"
run "$EN" "$cfg/session-phone.json" en-phone --model "$UNI" --parakeet-decoding beam --dict "$cfg/dict-en-set.json"
run "$PC" "$cfg/session-pc.json" pc-pc --model "$UNI" --parakeet-decoding auto
run "$HE" "$cfg/session-phone.json" he-phone --model "$HEM" --parakeet-decoding greedy --lang he
