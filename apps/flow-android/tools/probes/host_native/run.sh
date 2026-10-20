#!/usr/bin/env bash
# Build crates/flow-ffi for the HOST (x86_64, features jni,whisper) and run
# NativeFlowCoreHostTest: the shipped Kotlin NativeFlowCore + CoreJson against the
# real core, mock engine plus whisper on jfk.wav when a model is present.
# Proves the Kotlin<->Rust boundary without a device. Timings are appended to
# app/build/reports/host-native/timings.txt (one line per run).
# With a GGUF (FLOW_LLM_MODEL, default models/qwen2.5-0.5b-instruct-q4_k_m.gguf) it also builds the
# flow-llm plugin and runs LLM cleanup through the same binding (flow-ffi feature llm).
# Usage: tools/probes/host_native/run.sh [model.bin] [wav]   (default: tiny.en + jfk.wav from models/)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../../.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
models="${FLOW_MODELS:-$repo/models}"
[[ -d "$models" ]] || models="$(cd "$repo/../../.." && pwd)/models"  # worktree -> main checkout
model="${1:-$models/ggml-tiny.en.bin}"
wav="${2:-$models/jfk.wav}"
llm_model="${FLOW_LLM_MODEL:-$models/qwen2.5-0.5b-instruct-q4_k_m.gguf}"
# llama-cpp-sys-2 runs bindgen; this box's libclang lacks its resource headers.
export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I/usr/lib/gcc/x86_64-linux-gnu/13/include}"
if [[ -f "$llm_model" ]]; then
  (cd "$repo" && cargo build -q -p flow-llm --release --lib && cargo build -q -p flow-ffi --release --features jni,whisper,llm --lib)
else
  (cd "$repo" && cargo build -q -p flow-ffi --release --features jni,whisper --lib)
fi
lib="$repo/target/release"
test -f "$lib/libflow_ffi.so"
cd "$app"
./gradlew testDebugUnitTest -q --rerun -Pflow.jniLibDir="$lib" -Pflow.model="$model" -Pflow.wav="$wav" \
  -Pflow.threads="${FLOW_THREADS:-4}" -Pflow.llmModel="$llm_model" -Pflow.llmLib="$lib/libflow_llm.so" -Pflow.reportDir="$app/app/build/reports/host-native" \
  --tests 'dev.flowclone.core.NativeFlowCoreHostTest'
python3 -I - "$app" <<'PY'
import sys, xml.etree.ElementTree as ET, pathlib
p = pathlib.Path(sys.argv[1], "app/build/test-results/testDebugUnitTest/TEST-dev.flowclone.core.NativeFlowCoreHostTest.xml")
r = ET.parse(p).getroot()
print(f"host native test: {r.get('tests')} run, {int(r.get('failures'))+int(r.get('errors'))} failed, {r.get('skipped')} skipped")
PY
tail -n 2 "$app/app/build/reports/host-native/timings.txt" 2>/dev/null || true
