#!/usr/bin/env bash
# Fetch the three candidate cleanup models (pinned revision + SHA-256) into
# the bench cache, $CACHE/models (default ~/.cache/murmur/models). Idempotent.
# ~20.9 GB in total.
set -euo pipefail
CACHE="${CACHE:-$HOME/.cache/murmur}"
M="$CACHE/models"; mkdir -p "$M"

# repo revision file size sha256
PINS=(
  "bartowski/Qwen2.5-7B-Instruct-GGUF 8911e8a47f92bac19d6f5c64a2e2095bd2f7d031 Qwen2.5-7B-Instruct-Q4_K_M.gguf 4683074240 65b8fcd92af6b4fefa935c625d1ac27ea29dcb6ee14589c55a8f115ceaaa1423"
  "bartowski/google_gemma-3-12b-it-GGUF 648e3a36a77c8a9f12d86e741f9dcb9089c769c4 google_gemma-3-12b-it-Q4_K_M.gguf 7300575264 fc57f67efa46d711c346e587cbef7d049e95f3df8db2eb2271153343ef0acc7b"
  "bartowski/Qwen2.5-14B-Instruct-GGUF 05244aa5d871c661c80082a15d3bce44714d068d Qwen2.5-14B-Instruct-Q4_K_M.gguf 8988110976 e47ad95dad6ff848b431053b375adb5d39321290ea2c638682577dafca87c008"
)

for p in "${PINS[@]}"; do
  read -r repo rev f size sha <<<"$p"
  dest="$M/$f"
  if [ -s "$dest" ] && [ "$(stat -c %s "$dest")" = "$size" ]; then
    echo "have $f"
  else
    echo "fetch $f"
    curl -fsSL --retry 5 -C - -o "$dest.part" "https://huggingface.co/$repo/resolve/$rev/$f"
    mv "$dest.part" "$dest"
  fi
  got=$(sha256sum "$dest" | cut -d' ' -f1)
  if [ "$got" != "$sha" ]; then echo "SHA-256 mismatch: $f ($got)" >&2; exit 1; fi
  echo "ok $f $sha"
done
