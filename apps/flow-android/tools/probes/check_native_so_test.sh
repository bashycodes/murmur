#!/usr/bin/env bash
# Regression test for tools/probes/check_native_so.sh, no NDK needed: a stub llvm-nm/llvm-readelf
# (ANDROID_NDK_HOME) and stub libs (FLOW_JNILIBS_DIR, empty files) stand in for a local build.
#  1. Flake: the stub llvm-nm prints the wanted symbols first and then a long tail, so a reader
#     that exits on the first match (grep -q under pipefail) gets the producer killed by SIGPIPE.
#     The old gate failed most of these runs with "does not import SherpaOnnx..."; all must pass.
#  2. A real failure (a missing sherpa import) exits 1 and names the check.
# Usage: tools/probes/check_native_so_test.sh [runs]   (default 20)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../.." && pwd)"
runs="${1:-20}"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
bin="$tmp/ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
libs="$tmp/jniLibs"
mkdir -p "$bin" "$libs"
for l in libflow_ffi libflow_llm libsherpa-onnx-c-api libc++_shared libonnxruntime; do : > "$libs/$l.so"; done
grep -o 'Java_dev_flowclone_core_NativeFlowCore_native[A-Za-z]*' "$app/JNI_CONTRACT.md" | sort -u > "$tmp/contract"
sherpa_syms="SherpaOnnxCreateOnlineRecognizer SherpaOnnxCreateOnlineStream SherpaOnnxOnlineStreamAcceptWaveform SherpaOnnxDecodeOnlineStream SherpaOnnxOnlineStreamInputFinished SherpaOnnxGetOnlineStreamResultAsJson"

cat > "$bin/llvm-nm" <<EOF
#!/usr/bin/env bash
# Stub: canned symbol tables; DROP_IMPORT removes one import from libflow_ffi.so.
mode=all; for a in "\$@"; do case "\$a" in --undefined-only) mode=undef ;; --defined-only) mode=def ;; esac; done
f="\$(basename "\${@: -1}")"
tail_lines() { seq -f "                 U filler_symbol_%g" 1 20000; }
case "\$f:\$mode" in
  libflow_ffi.so:undef)
    for s in $sherpa_syms; do [[ "\$s" == "\${DROP_IMPORT:-}" ]] || echo "                 U \$s"; done
    tail_lines ;;
  libflow_ffi.so:def) sed 's/^/0000000000001000 T /' "$tmp/contract" ;;
  libflow_ffi.so:all) echo "0000000000002000 T whisper_full"; tail_lines ;;
  libflow_llm.so:def)
    for s in flow_llm_abi_version flow_llm_backends flow_llm_clean flow_llm_free_string flow_llm_info flow_llm_load flow_llm_prefill flow_llm_prefill_cancel flow_llm_unload; do echo "0000000000001000 T \$s"; done ;;
  libsherpa-onnx-c-api.so:def) for s in $sherpa_syms; do echo "0000000000001000 T \$s"; done; tail_lines ;;
esac
EOF
cat > "$bin/llvm-readelf" <<'EOF'
#!/usr/bin/env bash
# Stub: every .so has one 16 KB-aligned LOAD segment and needs only bionic.
case "$1" in
  -lW) echo "  LOAD 0x000000 0x0000000000000000 0x0000000000000000 0x001000 0x001000 R E 0x4000" ;;
  -dW) echo " 0x0000000000000001 (NEEDED) Shared library: [libc.so]" ;;
esac
EOF
chmod +x "$bin/llvm-nm" "$bin/llvm-readelf"

# GATE=<path> runs another copy of the gate (e.g. the old one, to watch it flake).
gate() { ANDROID_NDK_HOME="$tmp/ndk" FLOW_JNILIBS_DIR="$libs" "${GATE:-$here/check_native_so.sh}" --flavor local; }
bad=0
for _ in $(seq 1 "$runs"); do
  if ! out=$(gate 2>&1); then bad=$((bad + 1)); last="$out"; fi
done
if (( bad )); then
  echo "FAIL flake: $bad/$runs passing gate runs exited non-zero; last output:"; echo "$last"; exit 1
fi
echo "ok: $runs/$runs gate runs passed with long nm output (no SIGPIPE flake)"

if out=$(DROP_IMPORT=SherpaOnnxDecodeOnlineStream gate 2>&1); then
  echo "FAIL: gate passed with a missing sherpa import"; echo "$out"; exit 1
fi
grep -q 'FAIL \[sherpa-online\]: libflow_ffi.so: does not import SherpaOnnxDecodeOnlineStream' <<<"$out" \
  && grep -q 'failed checks: sherpa-online' <<<"$out" \
  || { echo "FAIL: missing import not named"; echo "$out"; exit 1; }
echo "ok: a missing import exits 1 and names check sherpa-online"
