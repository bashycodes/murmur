#!/usr/bin/env bash
# D80: no shipped native library may carry espeak-ng (GPL-3.0) or piper-phonemize, which
# sherpa-onnx links in for text-to-speech only. Checks built files (.so/.dll), not source.
#   tools/checks/no-tts-libs.sh FILE...     (exit 1 naming each file that has them)
# The markers are strings from espeak-ng's and piper-phonemize's own code; a no-TTS build's
# "TTS is not enabled" stubs have none of them. ("espeak" alone would match "wespeaker".)
set -euo pipefail
[ $# -gt 0 ] || { echo "usage: no-tts-libs.sh FILE..." >&2; exit 2; }
bad=0
for f in "$@"; do
  [ -f "$f" ] || { echo "no-tts-libs.sh: $f not found" >&2; exit 2; }
  n=$(LC_ALL=C grep -a -c -E 'espeak-ng|espeak_|piper_phonemize|phontab' "$f" || true)
  if [ "$n" != 0 ]; then
    echo "FAIL $f: $n espeak-ng/piper markers (a sherpa-onnx build with TTS; use the no-TTS build)" >&2
    bad=1
  else
    echo "ok   $f: no espeak-ng/piper code"
  fi
done
exit "$bad"
