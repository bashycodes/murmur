#!/usr/bin/env bash
# usage: run_evals.sh LABEL
# Deterministic evals for the post-processing rewrite, no LLM, through the
# real MurmurCore commit path (probe.rs eval mode): the formatting-eval cases
# (our own set) with the shipped and the proposed styles. Optional parity
# set: PARITY_CASES (a JSONL outside the repo) with PARITY_SESSION and
# PARITY_OUT (an output dir outside the repo); its outputs never go into
# the repo (they hold real dictations). The two original runners no longer build
# (stale `guard` / `CleanPrompt` paths). Writes results/LABEL_*.jsonl and
# prints formatting_eval score.py's tables.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
label="$1"
fe="$root/tools/probes/prototypes/formatting_eval_2026_10_24"
mkdir -p "$here/results"
run() { PROBE_FE_CASES="$1" PROBE_FE_SESSION="$2" bash "$here/run_probe.sh" "$3"; }
run "$fe/cases.jsonl" "$fe/sessions/shipped.json" "$here/results/${label}_fe_shipped.jsonl"
run "$fe/cases.jsonl" "$fe/sessions/proposed_styles.json" "$here/results/${label}_fe_styles.jsonl"
if [ -n "${PARITY_CASES:-}" ]; then
  run "$PARITY_CASES" "$PARITY_SESSION" "$PARITY_OUT/${label}_parity.jsonl"
fi
rm -f "$here/results/${label}"_*.jsonl.log
cd "$fe"
python3 score.py cases.jsonl "shipped=$here/results/${label}_fe_shipped.jsonl" "styles=$here/results/${label}_fe_styles.jsonl"
if [ -n "${PARITY_CASES:-}" ]; then
  python3 score.py "$PARITY_CASES" "parity=$PARITY_OUT/${label}_parity.jsonl"
fi
