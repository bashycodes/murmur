#!/usr/bin/env bash
# CI helper (Windows, Git Bash): fail unless every whisper.cpp/ggml build
# under the given cargo profile dirs was compiled optimised.
#
# Why: the `cmake` crate, with the Visual Studio generator, overwrites
# CMAKE_<LANG>_FLAGS_RELEASE with cc's flags ("-nologo -MD -Brepro"), and
# those carry no /O2 (cmake-rs#240). whisper-rs-sys does not put them back,
# so through 1.0.0 ggml was built at MSVC's default /Od and CPU whisper ran
# 6-18x slower than whisper-cli (docs/findings/2026-11-02-win-cpu-whisper-speed.md).
# The workflow sets CMAKE_C_FLAGS_RELEASE / CMAKE_CXX_FLAGS_RELEASE, which
# whisper-rs-sys forwards to CMake; this check proves they arrived.
#
# Usage: tools/ci/check-ggml-opt.sh <cargo-profile-dir>...   (e.g. target/release)
set -euo pipefail
[ $# -gt 0 ] || { echo "usage: $0 <cargo-profile-dir>..." >&2; exit 2; }
found=0
bad=0
for dir in "$@"; do
  for b in "$dir"/build/whisper-rs-sys-*/out/build; do
    [ -f "$b/CMakeCache.txt" ] || continue
    found=$((found + 1))
    echo "== $b"
    grep -E '^CMAKE_(C|CXX)_FLAGS_RELEASE:' "$b/CMakeCache.txt" || true
    for proj in ggml-cpu ggml-base whisper; do
      f=$(find "$b" -name "$proj.vcxproj" | head -n 1)
      if [ -z "$f" ]; then
        echo "  missing $proj.vcxproj" >&2
        bad=1
        continue
      fi
      # The Release|x64 ItemDefinitionGroup's <ClCompile><Optimization>.
      opt=$(awk '/<ItemDefinitionGroup Condition=.*Release[|]x64/{r=1} r&&/<Optimization>/{gsub(/.*<Optimization>|<\/Optimization>.*/,"");print;exit} r&&/<\/ItemDefinitionGroup>/{r=0}' "$f" | tr -d '\r')
      echo "  $proj Release Optimization: ${opt:-<none>}"
      [ "$opt" = MaxSpeed ] || bad=1
    done
  done
done
if [ "$found" -eq 0 ]; then
  echo "no whisper-rs-sys CMake build found under: $*" >&2
  exit 1
fi
if [ "$bad" -ne 0 ]; then
  echo "whisper.cpp/ggml is not built with /O2: CPU whisper would be several times slower" >&2
  exit 1
fi
echo "ok: $found whisper.cpp build(s) optimised (/O2)"
