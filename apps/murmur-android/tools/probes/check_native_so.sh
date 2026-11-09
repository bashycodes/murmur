#!/usr/bin/env bash
# Static checks on the packaged native libs (no device needed):
#  1. every C symbol in JNI_CONTRACT.md is exported by libflow_ffi.so;
#  2. every LOAD segment of every .so is >= 16 KB aligned (Android 15 16 KB pages);
#  3. NEEDED libs are bionic or shipped next to it (libc++_shared.so);
#  4. optional APK: zipalign -P 16 says the .so entries are page-aligned + stored.
#  4b. no .so carries espeak-ng/piper (sherpa-onnx TTS, GPL-3.0; D80).
#  5. per flavour (D17): remote ships no ASR/LLM libs and libflow_ffi.so has no whisper_/llama_
#     symbols; local's libflow_ffi.so has no WebSocket/TLS code and no cloud STT client (D18 C2);
#     hybrid/remote's carries the cloud client (feature `cloud`) and the D39 admin natives (`admin`)
#     and the D41 model-library natives (`library`).
# Every failed check prints "FAIL [<check>]: <why>", and the last line names the failed checks.
# No tool's output is piped into a reader that can exit early (grep -q): under pipefail the
# tool's SIGPIPE (141) failed the pipeline at random, which CI once reported as "libflow_ffi.so:
# does not import SherpaOnnxCreateOnlineRecognizer" while the same run counted 11 imports.
# Each tool runs once into a variable, and the checks match on that.
# Usage: tools/probes/check_native_so.sh [--flavor local|hybrid|remote] [app.apk]
# Test: tools/probes/check_native_so_test.sh (stub toolchain via ANDROID_NDK_HOME + FLOW_JNILIBS_DIR).
set -euo pipefail
app="$(cd "$(dirname "$0")/../.." && pwd)"
ndk="${ANDROID_NDK_HOME:-$HOME/Android/Sdk/ndk/27.2.12479018}"
bin="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
flavor=local
if [[ "${1:-}" == "--flavor" ]]; then flavor="$2"; shift 2; fi
dir="${FLOW_JNILIBS_DIR:-$app/app/src/$flavor/jniLibs/arm64-v8a}"
echo "flavor $flavor: $dir"
so="$dir/libflow_ffi.so"
failed=()
# fail <check> <why>: report a failed check by name; the gate exits 1 at the end.
fail() { echo "FAIL [$1]: $2"; failed+=("$1"); }
# A command that dies outside a check (missing tool, unreadable .so) still says what it was.
trap 'echo "FAIL [internal]: line $LINENO: \`$BASH_COMMAND\` exited $?"; echo "native gate ($flavor): failed checks: ${failed[*]:-} internal"' ERR
# has_word <text> <word>: exact-word match on captured output.
has_word() { grep -qw -- "$2" <<<"$1"; }
# scan_so <check> <pattern> <found-msg> <clean-msg>: grep libflow_ffi.so, failing closed:
# rc 0 is a hit (fail), 1 is clean, anything else (unreadable .so) fails too.
scan_so() {
  local rc=0; grep -qaE -- "$2" "$so" || rc=$?
  case $rc in
    0) fail "$1" "$3" ;;
    1) echo "$4" ;;
    *) fail "$1" "grep exited $rc reading $so" ;;
  esac
}

want=$(grep -o 'Java_dev_flowclone_core_NativeFlowCore_native[A-Za-z]*' "$app/JNI_CONTRACT.md" | sort -u)
# D39 / D41: hybrid / remote also export the admin API (`admin`) and the model library
# (`library`); local must not.
if [[ "$flavor" != local ]]; then
  want=$(printf '%s\n%s\n' "$want" "$(grep -o 'Java_dev_flowclone_\(admin_NativeAdmin\|server_ServerLibraryNative\)_native[A-Za-z]*' "$app/JNI_CONTRACT.md")" | sort -u)
fi
so_def=$("$bin/llvm-nm" -D --defined-only "$so")
so_undef=$("$bin/llvm-nm" -D --undefined-only "$so")
have=$(awk '$2=="T" && $3 ~ /^Java_/ {print $3}' <<<"$so_def" | sort -u)
missing=$(comm -23 <(echo "$want") <(echo "$have"))
extra=$(comm -13 <(echo "$want") <(echo "$have"))
echo "contract symbols: $(echo "$want" | wc -l) wanted, $(echo "$have" | wc -l) exported"
[[ -z "$missing" ]] || fail contract "MISSING: $missing"
[[ -z "$extra" ]] || fail contract "EXTRA (not in contract): $extra"
for f in "$dir"/*.so; do
  hdr=$("$bin/llvm-readelf" -lW "$f")
  bad=$(awk '$1=="LOAD"{a=strtonum($NF); if (a < 16384) print $NF}' <<<"$hdr")
  if [[ -n "$bad" ]]; then fail load-align "$(basename "$f"): LOAD align < 16 KB: $bad"
  else echo "$(basename "$f"): LOAD segments 16 KB aligned"; fi
done
# D80: no .so carries sherpa-onnx's text-to-speech code (espeak-ng, GPL-3.0; piper).
for f in "$dir"/*.so; do
  rc=0; tts=$(LC_ALL=C grep -a -c -E 'espeak-ng|espeak_|piper_phonemize|phontab' "$f") || rc=$?
  case $rc in
    0) fail no-tts "$(basename "$f"): $tts espeak-ng/piper markers (use tools/build-sherpa-android.sh's no-TTS sherpa-onnx)" ;;
    1) echo "$(basename "$f"): no espeak-ng/piper code" ;;
    *) fail no-tts "grep exited $rc reading $f" ;;
  esac
done
for f in "$dir"/*.so; do
  dyn=$("$bin/llvm-readelf" -dW "$f")
  for n in $(sed -n 's/.*NEEDED.*\[\(.*\)\]/\1/p' <<<"$dyn"); do
    case "$n" in libc.so|libm.so|libdl.so|liblog.so|libandroid.so) ;;
      *) [[ -f "$dir/$n" ]] || fail needed "$(basename "$f"): NEEDED $n not shipped" ;;
    esac
  done
done
llm="$dir/libflow_llm.so"
if [[ -f "$llm" ]]; then
  # The plugin must export only the flow_llm_* C ABI (flow-core/src/llama.rs); exporting its
  # ggml would let the dynamic linker interpose it with whisper's ggml in libflow_ffi.so.
  llm_def=$("$bin/llvm-nm" -D --defined-only "$llm")
  llm_have=$(awk '$2=="T"{print $3}' <<<"$llm_def" | sort -u)
  llm_want=$(printf '%s\n' flow_llm_abi_version flow_llm_backends flow_llm_clean flow_llm_free_string flow_llm_info flow_llm_load flow_llm_prefill flow_llm_prefill_cancel flow_llm_set_draft flow_llm_unload)
  if [[ "$llm_have" == "$llm_want" ]]; then echo "libflow_llm.so: exports exactly the 10 flow_llm_* symbols"
  else fail llm-exports "libflow_llm.so: unexpected exports: $(comm -3 <(echo "$llm_have") <(echo "$llm_want") | tr -d '\t' | tr '\n' ' ')"; fi
  if grep -q ' T ggml_' <<<"$so_def"; then
    echo "note: libflow_ffi.so exports ggml_* (whisper's); the plugin keeps its own ggml private"
  fi
else
  echo "libflow_llm.so: not packaged (AI cleanup unavailable; build with --llm)"
fi
so_all=$("$bin/llvm-nm" "$so" 2>/dev/null || true)  # a stripped .so has no symtab: count 0
echo "whisper linked: $(grep -c ' whisper_full' <<<"$so_all" || true) whisper_full* symbols"
# D33/H5: the Hebrew zipformer runs on sherpa's *online* recognizer: libflow_ffi.so must import
# it and the shipped libsherpa-onnx-c-api.so must export it (offline-only builds lack it).
sherpa="$dir/libsherpa-onnx-c-api.so"
if [[ -f "$sherpa" ]]; then
  sherpa_def=$("$bin/llvm-nm" -D --defined-only "$sherpa")
  for sym in SherpaOnnxCreateOnlineRecognizer SherpaOnnxCreateOnlineStream SherpaOnnxOnlineStreamAcceptWaveform SherpaOnnxDecodeOnlineStream SherpaOnnxOnlineStreamInputFinished SherpaOnnxGetOnlineStreamResultAsJson; do
    has_word "$so_undef" "$sym" || fail sherpa-online "libflow_ffi.so: does not import $sym (feature parakeet)"
    has_word "$sherpa_def" "$sym" || fail sherpa-online "libsherpa-onnx-c-api.so: does not export $sym"
  done
  echo "sherpa online (zipformer): $(grep -c 'SherpaOnnx.*Online' <<<"$so_undef" || true) online symbols imported"
fi
case "$flavor" in
  remote)
    bad=$(find "$dir" -mindepth 1 -maxdepth 1 -printf '%f\n' | grep -E '^lib(whisper|sherpa|onnxruntime|flow_llm|c\+\+_shared|llama|ggml)' || true)
    if [[ -z "$bad" ]]; then echo "remote: no ASR/LLM libs"; else fail remote-libs "remote: ships $bad"; fi
    n=$(grep -cE ' (whisper_|llama_|ggml_)' <<<"$so_def" || true)
    if [[ "$n" == 0 ]]; then echo "remote: libflow_ffi.so exports no whisper_/llama_/ggml_"; else fail remote-symbols "remote: $n ASR symbols exported"; fi ;;
  local)
    scan_so local-network 'tungstenite|rustls' "local: network code in libflow_ffi.so" "local: no tungstenite/rustls in libflow_ffi.so"
    scan_so local-cloud 'reqwest|webpki|flow-cloud' "local: cloud STT client in libflow_ffi.so" "local: no reqwest/webpki/flow-cloud in libflow_ffi.so" ;;
esac
case "$flavor" in
  hybrid|remote)
    if grep -qaE 'flow-cloud' "$so" && grep -qaE 'reqwest' "$so"; then echo "$flavor: cloud STT client linked (feature cloud)"
    else fail cloud-client "$flavor: libflow_ffi.so lacks the cloud client (build-native.sh --flavor $flavor adds feature cloud)"; fi ;;
esac
if [[ -n "${1:-}" ]]; then
  za="$(find "${ANDROID_HOME:-$HOME/Android/Sdk}"/build-tools -mindepth 1 -maxdepth 1 | sort -V | tail -1)/zipalign"
  # zipalign -c exits non-zero when an entry is misaligned; its per-entry lines say which.
  za_rc=0; za_out=$("$za" -c -P 16 -v 4 "$1") || za_rc=$?
  bad=$(grep -E 'lib/.*\.so' <<<"$za_out" | grep -v '(OK' || true)
  if [[ -n "$bad" ]]; then fail apk-align "APK: .so not 16 KB aligned: $bad"
  elif [[ $za_rc != 0 ]]; then fail apk-align "APK: zipalign -c exited $za_rc"
  else echo "APK: lib/*.so entries 16 KB aligned (zipalign -P 16)"; fi
  listing=$(unzip -lv "$1")
  awk '/lib\/.*\.so/{print "  " $8, $1, "bytes", $2}' <<<"$listing"
fi
trap - ERR
if (( ${#failed[@]} )); then
  echo "native gate ($flavor): failed checks: $(printf '%s\n' "${failed[@]}" | sort -u | tr '\n' ' ')"
  exit 1
fi
echo "native gate ($flavor): all checks passed"
