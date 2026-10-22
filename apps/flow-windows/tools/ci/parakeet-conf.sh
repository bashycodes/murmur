#!/usr/bin/env bash
# CI helper (Windows, Git Bash): write tauri.parakeet.conf.json, a config
# overlay that bundles the Parakeet runtime DLLs (sherpa-onnx C API +
# onnxruntime, copied into target/release by sherpa-onnx-sys' build
# script) next to flow-windows.exe. Run after a release cargo build with
# the `parakeet` feature. Usage: tools/ci/parakeet-conf.sh [target-dir]
set -euo pipefail
target=${1:-target/release}
dlls=$(cd "$target" && ls *.dll 2>/dev/null | grep -Ei '^(sherpa-onnx|onnxruntime)' || true)
if [ -z "$dlls" ]; then
  echo "no sherpa-onnx/onnxruntime DLLs in $target" >&2
  exit 1
fi
{
  echo '{'
  echo '  "$schema": "https://schema.tauri.app/config/2",'
  echo '  "bundle": {'
  echo '    "resources": {'
  first=1
  for d in $dlls; do
    [ $first = 1 ] || echo ','
    printf '      "%s/%s": "%s"' "$target" "$d" "$d"
    first=0
  done
  echo
  echo '    }'
  echo '  }'
  echo '}'
} > tauri.parakeet.conf.json
cat tauri.parakeet.conf.json
