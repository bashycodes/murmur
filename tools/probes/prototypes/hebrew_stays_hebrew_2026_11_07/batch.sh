#!/usr/bin/env bash
# usage: batch.sh dev|holdout  (env as run.sh). The prompt-variant rows of
# the findings doc with a 20 s budget (the model's language, not the shared
# box's speed). dev = he_cases.jsonl; holdout = he_holdout.jsonl, written
# after the dev rows picked the variants.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
L1="Write the text in Hebrew, the language of the TRANSCRIPT. Do not translate it into English."
L2="Keep the text in Hebrew."
shot="$here/variants/shot_he.json"
case "$1" in
dev)
  c="$here/he_cases.jsonl"
  "$here/run.sh" he_nohint_t20 "$c" en --timeout-ms 20000
  "$here/run.sh" he_sfx1_t20 "$c" he --timeout-ms 20000 --user-suffix "$L1"
  "$here/run.sh" he_shot_t20 "$c" he --timeout-ms 20000 --extra-shots "$shot"
  "$here/run.sh" he_sfx1_shot_t20 "$c" he --timeout-ms 20000 --user-suffix "$L1" --extra-shots "$shot"
  ;;
holdout)
  h="$here/he_holdout.jsonl"
  "$here/run.sh" ho_before_t20 "$h" he --timeout-ms 20000
  "$here/run.sh" ho_sfx1_t20 "$h" he --timeout-ms 20000 --user-suffix "$L1"
  "$here/run.sh" ho_shot_t20 "$h" he --timeout-ms 20000 --extra-shots "$shot"
  "$here/run.sh" ho_sfx1_shot_t20 "$h" he --timeout-ms 20000 --user-suffix "$L1" --extra-shots "$shot"
  "$here/run.sh" ho_sfx2_shot_t20 "$h" he --timeout-ms 20000 --user-suffix "$L2" --extra-shots "$shot"
  ;;
product)
  # BIN = the runner built against this branch's flow-core (no prompt
  # overrides: the shipped prompt and the script check). TAG = before|after.
  "$here/run.sh" "he_${TAG}_t20" "$here/he_cases.jsonl" he --timeout-ms 20000
  "$here/run.sh" "ho_${TAG}_t20" "$here/he_holdout.jsonl" he --timeout-ms 20000
  # A run whose last call timed out can exit non-zero after writing every
  # row (the model thread is still decoding at exit); the rows are kept.
  "$here/run.sh" "he_${TAG}_3s" "$here/he_cases.jsonl" he || echo "he_${TAG}_3s exit $?" >&2
  "$0" english
  ;;
english)
  # The numbers probe's 61 English cases, same TAG / BIN.
  "$here/run.sh" "en_${TAG}_t20" "$here/../numbers_2026_11_06/cases.jsonl" en --timeout-ms 20000 \
    || echo "en_${TAG}_t20 exit $?" >&2
  "$here/run.sh" "en_${TAG}_3s" "$here/../numbers_2026_11_06/cases.jsonl" en || echo "en_${TAG}_3s exit $?" >&2
  ;;
guard)
  # The script check alone: this branch's runner (BIN) with the base
  # prompt (results/prompt_he_before.json, the base commit's system prompt
  # and English few-shots, so no Hebrew turn).
  "$here/run.sh" he_guard_only_t20 "$here/he_cases.jsonl" he --timeout-ms 20000 \
    --override "$here/results/prompt_he_before.json"
  ;;
esac
