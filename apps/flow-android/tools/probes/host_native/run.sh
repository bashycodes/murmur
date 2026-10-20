#!/usr/bin/env bash
# Build crates/flow-ffi for the HOST (x86_64, features jni,whisper) and run
# NativeFlowCoreHostTest: the shipped Kotlin NativeFlowCore + CoreJson against the
# real core, mock engine plus whisper on jfk.wav when a model is present.
# Proves the Kotlin<->Rust boundary without a device. Timings are appended to
# app/build/reports/host-native/timings.txt (one line per run).
# Usage: tools/probes/host_native/run.sh [model.bin] [wav]   (default: tiny.en + jfk.wav from models/)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../../.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
models="${FLOW_MODELS:-$repo/models}"
[[ -d "$models" ]] || models="$(cd "$repo/../../.." && pwd)/models"  # worktree -> main checkout
model="${1:-$models/ggml-tiny.en.bin}"
wav="${2:-$models/jfk.wav}"
(cd "$repo" && cargo build -q -p flow-ffi --release --features jni,whisper --lib)
lib="$repo/target/release"
test -f "$lib/libflow_ffi.so"
cd "$app"
./gradlew testDebugUnitTest -q --rerun -Pflow.jniLibDir="$lib" -Pflow.model="$model" -Pflow.wav="$wav" \
  -Pflow.threads="${FLOW_THREADS:-4}" -Pflow.reportDir="$app/app/build/reports/host-native" \
  --tests 'dev.flowclone.core.NativeFlowCoreHostTest'
python3 -I - "$app" <<'PY'
import sys, xml.etree.ElementTree as ET, pathlib
p = pathlib.Path(sys.argv[1], "app/build/test-results/testDebugUnitTest/TEST-dev.flowclone.core.NativeFlowCoreHostTest.xml")
r = ET.parse(p).getroot()
print(f"host native test: {r.get('tests')} run, {int(r.get('failures'))+int(r.get('errors'))} failed, {r.get('skipped')} skipped")
PY
tail -n 2 "$app/app/build/reports/host-native/timings.txt" 2>/dev/null || true
