#!/usr/bin/env bash
# CI helper (Windows, Git Bash; D80): every NSIS installer given unpacks
# COPYRIGHT.txt, NOTICE.txt and THIRD-PARTY-LICENSES.txt next to the app, and
# its sherpa-onnx / ONNX Runtime DLLs carry no espeak-ng (TTS) code.
#   tools/ci/check-installer-notices.sh INSTALLER.exe...
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
seven="$(command -v 7z || echo "/c/Program Files/7-Zip/7z.exe")"
[ $# -gt 0 ] || { echo "usage: check-installer-notices.sh INSTALLER.exe..." >&2; exit 2; }
for exe in "$@"; do
  dir="$(mktemp -d)"
  "$seven" x -y -o"$dir" "$exe" > /dev/null
  for f in COPYRIGHT.txt NOTICE.txt THIRD-PARTY-LICENSES.txt; do
    found="$(find "$dir" -name "$f" | head -1)"
    [ -n "$found" ] && [ -s "$found" ] || { echo "FAIL $exe: no $f inside" >&2; exit 1; }
  done
  grep -q 'All rights reserved' "$(find "$dir" -name COPYRIGHT.txt | head -1)" ||
    { echo "FAIL $exe: COPYRIGHT.txt does not reserve rights" >&2; exit 1; }
  mapfile -t dlls < <(find "$dir" -iname 'sherpa-onnx-c-api.dll' -o -iname 'onnxruntime*.dll')
  [ ${#dlls[@]} -gt 0 ] || { echo "FAIL $exe: no sherpa-onnx DLLs inside" >&2; exit 1; }
  "$here/../../../../tools/checks/no-tts-libs.sh" "${dlls[@]}"
  echo "ok   $exe: COPYRIGHT, NOTICE, THIRD-PARTY-LICENSES; ${#dlls[@]} DLLs without TTS"
  rm -rf "$dir"
done
