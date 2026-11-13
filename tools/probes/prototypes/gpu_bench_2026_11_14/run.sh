#!/usr/bin/env bash
# One model through the four synthetic suites of gpu_cleanup_model_2026_11_12
# (bench, numbers, Hebrew dev + holdout), in `server` mode (prompt v2, never
# skipped), with the model on the Windows PC's GPU behind llama-server
# (pc/serve.ps1), reached through an SSH tunnel at $URL.
#
#   run.sh TAG [SUITES...]      -> results/TAG-{bench,numbers,he,ho}.jsonl
#
# env: BIN (gpu-http-clean), URL (default http://127.0.0.1:18091).
# Light on this box: the runner only runs the rules and waits on HTTP.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
TAG=$1; shift
SUITES=${*:-bench numbers he ho}
: "${BIN:?}"
URL=${URL:-http://127.0.0.1:18091}
O="$here/../gpu_cleanup_model_2026_11_12"
P="$here/.."
mkdir -p "$here/results"
for s in $SUITES; do
  case $s in
    bench) c="$O/bench_cases.jsonl"; ses="$O/session-formal.json"; lang=en ;;
    numbers) c="$P/numbers_2026_11_06/cases.jsonl"; ses="$O/session-casual.json"; lang=en ;;
    he) c="$P/hebrew_stays_hebrew_2026_11_07/he_cases.jsonl"; ses="$O/session-casual.json"; lang=he ;;
    ho) c="$P/hebrew_stays_hebrew_2026_11_07/he_holdout.jsonl"; ses="$O/session-casual.json"; lang=he ;;
    he5) c="$O/control_he5.jsonl"; ses="$O/session-casual.json"; lang=he ;;
    *) echo "unknown suite $s" >&2; exit 2 ;;
  esac
  o="$here/results/$TAG-$s.jsonl"
  if [ -s "$o" ]; then echo "have $TAG-$s" >&2; continue; fi
  echo "== $TAG-$s $(date +%T)" >&2
  timeout 7200 "$BIN" --cases "$c" --mode server --url "$URL" \
    --session "$ses" --language "$lang" --timeout-ms 120000 \
    > "$o.tmp" && mv "$o.tmp" "$o"
done
echo "done $TAG $(date +%T)" >&2
