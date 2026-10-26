#!/usr/bin/env bash
# Phase 1d (2026-10-26): AI cleanup through OpenRouter with the product's
# OpenRouterCleaner (D28: provider zdr:true, data_collection deny, sort
# latency), prompt v4b, both guards, the same 102 cases.
# usage: run_or.sh LABEL MODEL_ID
# The key is read from ~/.config/flow-clone/openrouter.key into this process's
# env only (never an argument, never printed). Total spend over all or_*
# results is capped at $CAP (default 1.50): the runner gets the remainder and
# stops when it is used up.
#
# Prompt v4c rerun (2026-10-26): PROMPT_VERSION=v4c uses flow-core's built-in
# prompt v4c instead of prompt_v4b.json; SPENT_GLOB (default or_*) picks the
# runs the cap counts (v4c reruns: SPENT_GLOB='or_v4c*' CAP=0.50); CASES may
# point at a subset of results/all_cases.jsonl.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="${BENCH_BIN:-$here/../../../../target/ai-bench/release/ai-cleanup-bench}"
label="$1"; model="$2"; cap="${CAP:-1.50}"
[[ -s "$here/results/$label.jsonl" ]] && { echo "== have $label"; exit 0; }
if [[ -n "${PROMPT_VERSION:-}" ]]; then
  prompt_args=(--prompt-version "$PROMPT_VERSION")
else
  prompt_args=(--prompt "$here/prompt_v4b.json")
fi
spent=$(SPENT_GLOB="${SPENT_GLOB:-or_*}" python3 -I - "$here/results" <<'PY'
import glob, json, os, sys
t = 0.0
for p in glob.glob(os.path.join(sys.argv[1], os.environ["SPENT_GLOB"] + ".jsonl")):
    rows = [json.loads(l) for l in open(p) if l.strip()]
    t += sum((r.get("cloud") or {}).get("cost_usd") or 0 for r in rows)
print(f"{t:.6f}")
PY
)
left=$(python3 -c "print(max(0.0, $cap - $spent))")
echo "== $label $model $(date +%T) spent so far \$$spent, left \$$left" >&2
python3 -c "import sys; sys.exit(0 if $left > 0.01 else 1)" || { echo "cap reached" >&2; exit 4; }
cases="${CASES:-$here/results/all_cases.jsonl}"
cat "$here/../formatting_eval_2026_10_24/cases.jsonl" "$here/wispr_cases.jsonl" > "$here/results/all_cases.jsonl"
OPENROUTER_KEY="$(cat "$HOME/.config/flow-clone/openrouter.key")"
export OPENROUTER_KEY
nice -n 10 "$bin" --mode ai --cases "$cases" --session "$here/sessions/prod.json" \
  --openrouter "$model" "${prompt_args[@]}" --timeout-ms 8000 --max-spend "$left" \
  > "$here/results/$label.jsonl"
