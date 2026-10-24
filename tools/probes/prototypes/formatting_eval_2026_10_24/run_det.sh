#!/usr/bin/env bash
# usage: run_det.sh
# Scores the deterministic flow-core pipeline (no LLM) with the spoken-command
# rules and bug fixes now in crates/flow-core (spoken.rs + spoken_commands.rs),
# for the shipped styles (all formal) and the proposed chat=casual styles.
# Replaces the rules_proto.py pre-pass for the det columns: the inputs are the
# raw cases. Outputs are deterministic, so a rerun rewrites the same files.
# Fails when the styles column drops below the D27 floor (72 % pass, 16/16
# negatives).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target="$here/../../../../target/formatting-eval"
CARGO_TARGET_DIR="$target" nice cargo build -q --manifest-path "$here/runner/Cargo.toml"
bin="$target/debug/formatting-eval"
mkdir -p "$here/results"
"$bin" --cases "$here/cases.jsonl" --session "$here/sessions/shipped.json" > "$here/results/det_core_rules_shipped.jsonl"
"$bin" --cases "$here/cases.jsonl" --session "$here/sessions/proposed_styles.json" > "$here/results/det_core_rules_styles.jsonl"
cd "$here"
python3 score.py cases.jsonl \
  det=results/det_shipped.jsonl det+styles=results/det_proposed_styles.jsonl \
  proto+styles=results/det_rules_styles.jsonl \
  core_rules=results/det_core_rules_shipped.jsonl core_rules+styles=results/det_core_rules_styles.jsonl
python3 - <<'PY'
import score
cases = score.load("cases.jsonl")
r = score.score(cases, score.load("results/det_core_rules_styles.jsonl"))
neg_ok, neg_n = r["cats"]["negative"]
print(f"floor: pass {r['pass']:.0%} (>= 72%), negatives {neg_ok}/{neg_n}")
assert r["pass"] >= 0.72 and neg_ok == neg_n, "below the D27 floor"
PY
