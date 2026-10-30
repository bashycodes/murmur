#!/usr/bin/env bash
# Static checks on the packaged native libs (no device needed):
#  1. every C symbol in JNI_CONTRACT.md is exported by libflow_ffi.so;
#  2. every LOAD segment of every .so is >= 16 KB aligned (Android 15 16 KB pages);
#  3. NEEDED libs are bionic or shipped next to it (libc++_shared.so);
#  4. optional APK: zipalign -P 16 says the .so entries are page-aligned + stored.
#  5. per flavour (D17): remote ships no ASR/LLM libs and libflow_ffi.so has no whisper_/llama_
#     symbols; local's libflow_ffi.so has no WebSocket/TLS code and no cloud STT client (D18 C2);
#     hybrid/remote's carries the cloud client (feature `cloud`).
# Usage: tools/probes/check_native_so.sh [--flavor local|hybrid|remote] [app.apk]
set -euo pipefail
app="$(cd "$(dirname "$0")/../.." && pwd)"
ndk="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
bin="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
flavor=local
if [[ "${1:-}" == "--flavor" ]]; then flavor="$2"; shift 2; fi
dir="$app/app/src/$flavor/jniLibs/arm64-v8a"
echo "flavor $flavor: $dir"
so="$dir/libflow_ffi.so"
fail=0
want=$(grep -o 'Java_dev_flowclone_core_NativeFlowCore_native[A-Za-z]*' "$app/JNI_CONTRACT.md" | sort -u)
have=$("$bin/llvm-nm" -D --defined-only "$so" | awk '$2=="T"{print $3}' | grep '^Java_' | sort -u)
missing=$(comm -23 <(echo "$want") <(echo "$have"))
extra=$(comm -13 <(echo "$want") <(echo "$have"))
echo "contract symbols: $(echo "$want" | wc -l) wanted, $(echo "$have" | wc -l) exported"
[[ -z "$missing" ]] || { echo "MISSING: $missing"; fail=1; }
[[ -z "$extra" ]] || { echo "EXTRA (not in contract): $extra"; fail=1; }
for f in "$dir"/*.so; do
  bad=$("$bin/llvm-readelf" -lW "$f" | awk '$1=="LOAD"{a=strtonum($NF); if (a < 16384) print $NF}')
  if [[ -n "$bad" ]]; then echo "$(basename "$f"): LOAD align < 16 KB: $bad"; fail=1
  else echo "$(basename "$f"): LOAD segments 16 KB aligned"; fi
done
for f in "$dir"/*.so; do
  for n in $("$bin/llvm-readelf" -dW "$f" | sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p'); do
    case "$n" in libc.so|libm.so|libdl.so|liblog.so|libandroid.so) ;;
      *) [[ -f "$dir/$n" ]] || { echo "$(basename "$f"): NEEDED $n not shipped"; fail=1; } ;;
    esac
  done
done
llm="$dir/libflow_llm.so"
if [[ -f "$llm" ]]; then
  # The plugin must export only the flow_llm_* C ABI (flow-core/src/llama.rs); exporting its
  # ggml would let the dynamic linker interpose it with whisper's ggml in libflow_ffi.so.
  llm_have=$("$bin/llvm-nm" -D --defined-only "$llm" | awk '$2=="T"{print $3}' | sort -u)
  llm_want=$(printf '%s\n' flow_llm_abi_version flow_llm_backends flow_llm_clean flow_llm_free_string flow_llm_info flow_llm_load flow_llm_prefill flow_llm_prefill_cancel flow_llm_unload)
  if [[ "$llm_have" == "$llm_want" ]]; then echo "libflow_llm.so: exports exactly the 9 flow_llm_* symbols"
  else echo "libflow_llm.so: unexpected exports: $(comm -3 <(echo "$llm_have") <(echo "$llm_want") | tr -d '\t' | tr '\n' ' ')"; fail=1; fi
  if "$bin/llvm-nm" -D --defined-only "$so" | grep -q ' T ggml_'; then
    echo "note: libflow_ffi.so exports ggml_* (whisper's); the plugin keeps its own ggml private"
  fi
else
  echo "libflow_llm.so: not packaged (AI cleanup unavailable; build with --llm)"
fi
echo "whisper linked: $("$bin/llvm-nm" "$so" 2>/dev/null | grep -c ' whisper_full' || true) whisper_full* symbols"
# D33/H5: the Hebrew zipformer runs on sherpa's *online* recognizer: libflow_ffi.so must import
# it and the shipped libsherpa-onnx-c-api.so must export it (offline-only builds lack it).
sherpa="$dir/libsherpa-onnx-c-api.so"
if [[ -f "$sherpa" ]]; then
  for sym in SherpaOnnxCreateOnlineRecognizer SherpaOnnxCreateOnlineStream SherpaOnnxOnlineStreamAcceptWaveform SherpaOnnxDecodeOnlineStream SherpaOnnxOnlineStreamInputFinished SherpaOnnxGetOnlineStreamResultAsJson; do
    "$bin/llvm-nm" -D --undefined-only "$so" | grep -qw "$sym" || { echo "libflow_ffi.so: does not import $sym (feature parakeet)"; fail=1; }
    "$bin/llvm-nm" -D --defined-only "$sherpa" | grep -qw "$sym" || { echo "libsherpa-onnx-c-api.so: does not export $sym"; fail=1; }
  done
  echo "sherpa online (zipformer): $("$bin/llvm-nm" -D --undefined-only "$so" | grep -c 'SherpaOnnx.*Online' || true) online symbols imported"
fi
case "$flavor" in
  remote)
    bad=$(ls "$dir" | grep -E '^lib(whisper|sherpa|onnxruntime|flow_llm|c\+\+_shared|llama|ggml)' || true)
    [[ -z "$bad" ]] && echo "remote: no ASR/LLM libs" || { echo "remote: ships $bad"; fail=1; }
    n=$("$bin/llvm-nm" -D --defined-only "$so" | grep -cE ' (whisper_|llama_|ggml_)' || true)
    [[ "$n" == 0 ]] && echo "remote: libflow_ffi.so exports no whisper_/llama_/ggml_" || { echo "remote: $n ASR symbols exported"; fail=1; } ;;
  local)
    if grep -qaE 'tungstenite|rustls' "$so"; then echo "local: network code in libflow_ffi.so"; fail=1; else echo "local: no tungstenite/rustls in libflow_ffi.so"; fi
    if grep -qaE 'reqwest|webpki|flow-cloud' "$so"; then echo "local: cloud STT client in libflow_ffi.so"; fail=1; else echo "local: no reqwest/webpki/flow-cloud in libflow_ffi.so"; fi ;;
esac
case "$flavor" in
  hybrid|remote)
    if grep -qaE 'flow-cloud' "$so" && grep -qaE 'reqwest' "$so"; then echo "$flavor: cloud STT client linked (feature cloud)"
    else echo "$flavor: libflow_ffi.so lacks the cloud client (build-native.sh --flavor $flavor adds feature cloud)"; fail=1; fi ;;
esac
if [[ -n "${1:-}" ]]; then
  za=$(ls -d "${ANDROID_HOME:-$HOME/Android/Sdk}"/build-tools/* | sort -V | tail -1)/zipalign
  if "$za" -c -P 16 -v 4 "$1" | grep -E 'lib/.*\.so' | grep -v '(OK' ; then echo "APK: .so not 16 KB aligned"; fail=1
  else echo "APK: lib/*.so entries 16 KB aligned (zipalign -P 16)"; fi
  unzip -lv "$1" | awk '/lib\/.*\.so/{print "  " $8, $1, "bytes", $2}'
fi
exit $fail
