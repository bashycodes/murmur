#!/usr/bin/env bash
# Every decision row in docs/DECISIONS.md (`| D<n> | ...`) has its own number.
# Parallel branches each take "the next D" and collide at merge; this fails
# the branch instead of leaving two D29s for a reviewer to spot.
set -euo pipefail
cd "$(dirname "$0")/../.."
dups=$(grep -oE '^\| *D[0-9]+ *\|' docs/DECISIONS.md | tr -d '| ' | sort | uniq -d)
if [ -n "$dups" ]; then
  echo "docs/DECISIONS.md: duplicate decision ids: $(echo $dups)" >&2
  echo "Renumber the newer one to the next free id and update references to it." >&2
  exit 1
fi
echo "decision ids unique ($(grep -cE '^\| *D[0-9]+ *\|' docs/DECISIONS.md) rows)"
