#!/usr/bin/env bash
# usage: run.sh [GPU_RESULTS_DIR]
#   Runs the D65 mixed-script check (script_lock::added_foreign) over the
#   stored Hebrew cleanup answers of hebrew_stays_hebrew_2026_11_07 and,
#   when given, the GPU bench's results/ (on feat/gpu-cleanup-model:
#   `git archive origin/feat/gpu-cleanup-model
#   tools/probes/prototypes/gpu_cleanup_model_2026_11_12/results | tar -x -C DIR`).
#   Builds under the shared build lock, low priority, into its own target dir.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
he="$here/../hebrew_stays_hebrew_2026_11_07"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$here/target}"
flock "$HOME/.cache/flow-clone-build.lock" choom -n 1000 -- nice -n 10 \
  cargo run -q -j 2 --offline --manifest-path "$here/Cargo.toml" -- \
  "$he" "$he/results" "$@"
