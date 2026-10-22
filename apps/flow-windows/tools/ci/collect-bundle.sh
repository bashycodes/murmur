#!/usr/bin/env bash
# CI helper: copy the MSI + NSIS installers Tauri just built into
# dist-ci/<flavour>/ with the flavour in the file name (both flavours share
# target/release/bundle). Usage: tools/ci/collect-bundle.sh <flavour>
set -euo pipefail
flavour=$1
out=dist-ci/$flavour
mkdir -p "$out"
shopt -s nullglob
n=0
for f in target/release/bundle/msi/*.msi target/release/bundle/nsis/*.exe; do
  base=$(basename "$f")
  cp "$f" "$out/${base// /-}"
  n=$((n + 1))
done
# Rename to carry the flavour: Flow-Clone_0.1.0_x64_en-US.msi -> ..._hybrid.msi
for f in "$out"/*; do
  ext=${f##*.}
  stem=${f%.*}
  case "$stem" in *"_$flavour") ;; *) mv "$f" "${stem}_${flavour}.${ext}" ;; esac
done
ls -la "$out"
[ "$n" -ge 2 ] || { echo "expected an MSI and an NSIS installer, found $n" >&2; exit 1; }
rm -rf target/release/bundle/msi target/release/bundle/nsis
