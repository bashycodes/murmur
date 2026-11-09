#!/usr/bin/env bash
# CI helper (Windows, Git Bash; D80): every installer given (NSIS .exe or
# WiX .msi) holds the repo's COPYRIGHT, NOTICE and THIRD-PARTY-LICENSES and
# its DLLs carry no espeak-ng (TTS) code.
#   tools/ci/check-installer-notices.sh INSTALLER.exe|.msi...
# 7-Zip unpacks both: an NSIS installer into its files by name; an MSI into
# its streams, whose cabinet holds the files under WiX file ids, not their
# names. So the notices are found by content (byte-equal to the repo root's
# files) and every PE file (MZ header) inside is checked for TTS code; that
# includes the MSI's Binary streams (the WiX UI extension's custom-action DLL
# and the WebView2 bootstrapper). SEVENZIP overrides the 7-Zip command (tests).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../../.." && pwd)"
seven="${SEVENZIP:-$(command -v 7z || echo "/c/Program Files/7-Zip/7z.exe")}"
[ $# -gt 0 ] || { echo "usage: check-installer-notices.sh INSTALLER.exe|.msi..." >&2; exit 2; }
sum() { sha256sum "$1" | cut -d' ' -f1; }
for inst in "$@"; do
  dir="$(mktemp -d)"
  "$seven" x -y -o"$dir/x" "$inst" > /dev/null
  # An MSI's files are in its cabinet(s): unpack those too.
  while IFS= read -r cab; do
    "$seven" x -y -o"$dir/cab-$(basename "$cab")" "$cab" > /dev/null
  done < <(find "$dir/x" -type f -iname '*.cab')
  find "$dir" -type f -exec sha256sum {} + | cut -d' ' -f1 | sort -u > "$dir/sums"
  for f in COPYRIGHT NOTICE THIRD-PARTY-LICENSES; do
    grep -qx "$(sum "$root/$f")" "$dir/sums" || { echo "FAIL $inst: no copy of $f inside" >&2; exit 1; }
  done
  pe=()
  while IFS= read -r p; do
    [ "$(head -c 2 "$p")" = MZ ] && pe+=("$p")
  done < <(find "$dir" -type f ! -name sums)
  sherpa=0
  for p in "${pe[@]}"; do LC_ALL=C grep -aq 'SherpaOnnx' "$p" && sherpa=$((sherpa + 1)); done
  [ "$sherpa" -gt 0 ] || { echo "FAIL $inst: no sherpa-onnx DLL inside" >&2; exit 1; }
  "$root/tools/checks/no-tts-libs.sh" "${pe[@]}" > /dev/null
  echo "ok   $inst: COPYRIGHT, NOTICE, THIRD-PARTY-LICENSES; ${#pe[@]} PE files (sherpa-onnx in $sherpa) without TTS"
  rm -rf "$dir"
done
