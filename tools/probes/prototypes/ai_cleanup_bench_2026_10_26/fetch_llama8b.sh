#!/usr/bin/env bash
# Download Llama-3.1-8B-Instruct Q4_K_M (bartowski GGUF repo) into $MODELS and
# check it against the sha256 the Hugging Face API lists for the LFS object
# (fetched 2026-10-26). Resumable; the file is renamed only when the hash matches.
set -euo pipefail
: "${MODELS:?set MODELS to the gguf directory}"
name=Meta-Llama-3.1-8B-Instruct-Q4_K_M.gguf
sha=7b064f5842bf9532c91456deda288a1b672397a54fa729aa665952863033557c
url=https://huggingface.co/bartowski/Meta-Llama-3.1-8B-Instruct-GGUF/resolve/main/$name
cd "$MODELS"
if [[ -f $name ]]; then echo "$sha  $name" | sha256sum -c; exit; fi
curl -sSL --fail -C - -o "$name.part" "$url"
echo "$sha  $name.part" | sha256sum -c
mv "$name.part" "$name"
echo "ok $name $(stat -c %s "$name") bytes"
