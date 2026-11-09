#!/usr/bin/env bash
# Lists, per path, a few history lines that cite the other product
# (name or corpus line cites), to build docs/public-release/history-drop.md.
# Usage: OTHER_NAME=<the other product's name> bash history_cites.sh PATH...
set -u
pat="${OTHER_NAME:?set OTHER_NAME to the other product's name}|[A-Z]:[0-9]{4,}"
for f in "$@"; do
  echo "== $f"
  git log --all -p -G "$pat" --format= -- "$f" \
    | grep -E "^[-+].*($pat)" | sort -u | head -3 | cut -c1-170
done
