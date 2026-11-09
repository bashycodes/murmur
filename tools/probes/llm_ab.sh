#!/usr/bin/env bash
# A/B the LLM cleanup decode settings on the 7 messy-dictation probes.
# usage: tools/probes/llm_ab.sh model.gguf [draft values...]   (default: 0 4 8)
# Needs: cargo build --release -p murmur-cli -p murmur-llm. Prints per-call llm ms
# and plugin counters; compare the medians, the box is shared and noisy.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
model="$1"; shift
drafts=("${@:-0 4 8}")
texts=(
  "um so i was thinking uh we should meet tomorrow at like three thirty pm"
  "send the report to john on friday scratch that send it to sarah on monday"
  "can you grab milk comma eggs comma and bread period thanks"
  "first we need to update the docs second fix the login bug and third ship the release"
  "the meeting is on tuesday no wait wednesday at ten am"
  "what time does the grocery store close on sundays"
  "i i think the the problem is is in the cache layer you know"
)
sess="$(mktemp)"; echo '{"llm_timeout_ms":30000}' > "$sess"
for d in ${drafts[@]}; do
  echo "== MURMUR_LLM_DRAFT=$d"
  MURMUR_LLM_DRAFT=$d "$root/target/release/murmur-cli" clean --llm "$model" --session "$sess" "${texts[@]}" 2>&1 >/dev/null \
    | grep '^llm' | awk '{print $2}' | sort -n | tr '\n' ' '; echo
done
rm -f "$sess"
