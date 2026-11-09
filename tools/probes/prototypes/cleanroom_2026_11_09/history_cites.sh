#!/usr/bin/env bash
# Lists, per path, a few history lines that cite the other product
# (name or corpus line cites), to build docs/public-release/history-drop.md.
# Usage: bash history_cites.sh PATH...
set -u
for f in "$@"; do
  echo "== $f"
  git log --all -p -G 'Wispr|[A-Z]:[0-9]{4,}' --format= -- "$f" \
    | grep -E '^[-+].*(Wispr|[A-Z]:[0-9]{4,})' | sort -u | head -3 | cut -c1-170
done
