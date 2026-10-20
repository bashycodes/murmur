#!/usr/bin/env bash
# Static checks on the arm64-v8a output of tools/build-android-parakeet.sh
# (no device): 16 KB LOAD alignment of every .so, NEEDED closure (bionic or
# shipped alongside), JNI contract symbols, and that whisper + sherpa are
# both linked into libflow_ffi.so.
# Usage: tools/probes/check_parakeet_so.sh [jniLibs/arm64-v8a dir]
set -euo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"
dir="${1:-$repo/target/android-parakeet/jniLibs/arm64-v8a}"
bin="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}/toolchains/llvm/prebuilt/linux-x86_64/bin"
so="$dir/libflow_ffi.so"; fail=0
for f in "$dir"/*.so; do
  bad=$("$bin/llvm-readelf" -lW "$f" | awk '$1=="LOAD"{a=strtonum($NF); if (a < 16384) print $NF}')
  if [[ -n "$bad" ]]; then echo "$(basename "$f"): LOAD align < 16 KB: $bad"; fail=1
  else echo "$(basename "$f"): $(stat -c %s "$f") bytes, LOAD segments 16 KB aligned"; fi
  for n in $("$bin/llvm-readelf" -dW "$f" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p'); do
    case "$n" in libc.so|libm.so|libdl.so|liblog.so|libandroid.so) ;;
      *) [[ -f "$dir/$n" ]] || { echo "  NEEDED $n not shipped"; fail=1; } ;;
    esac
  done
done
want=$(grep -o 'Java_dev_flowclone_core_NativeFlowCore_native[A-Za-z]*' "$repo/apps/flow-android/JNI_CONTRACT.md" | sort -u)
have=$("$bin/llvm-nm" -D --defined-only "$so" | awk '$2=="T"{print $3}' | grep '^Java_' | sort -u)
missing=$(comm -23 <(echo "$want") <(echo "$have"))
echo "JNI contract: $(echo "$want" | wc -l) wanted, $(echo "$have" | wc -l) exported"
[[ -z "$missing" ]] || { echo "MISSING: $missing"; fail=1; }
echo "libflow_ffi NEEDED: $("$bin/llvm-readelf" -dW "$so" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p' | tr '\n' ' ')"
echo "sherpa imports: $("$bin/llvm-nm" -D --undefined-only "$so" | grep -c SherpaOnnx)"
echo "whisper linked: $("$bin/llvm-nm" -D --defined-only "$so" 2>/dev/null | grep -c ' whisper_' || true) exported whisper_* (0 is fine if hidden)"
exit $fail
