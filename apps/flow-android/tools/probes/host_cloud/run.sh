#!/usr/bin/env bash
# D18 host test: build crates/flow-ffi for the HOST with features jni,cloud and run
# NativeFlowCoreCloudHostTest: the shipped Kotlin cloud path (DevSettings → EngineRoute →
# nativeOpenRouter → nativeSetCloudKey / nativeProbeCloud / nativeCloudUsage) against the real
# router + flow-cloud, talking to a JVM mock of an OpenAI-compatible provider. No keys, no network.
# Usage: tools/probes/host_cloud/run.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../../.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
(cd "$repo" && cargo build -q -p flow-ffi --release --features jni,cloud --lib)
lib="$repo/target/release"
test -f "$lib/libflow_ffi.so"
cd "$app"
./gradlew testHybridDebugUnitTest -q --rerun -Pflow.jniLibDir="$lib" --tests 'dev.flowclone.core.NativeFlowCoreCloudHostTest'
python3 -I - "$app" <<'PY'
import sys, xml.etree.ElementTree as ET, pathlib
p = pathlib.Path(sys.argv[1], "app/build/test-results/testHybridDebugUnitTest/TEST-dev.flowclone.core.NativeFlowCoreCloudHostTest.xml")
r = ET.parse(p).getroot()
print(f"host cloud test: {r.get('tests')} run, {int(r.get('failures'))+int(r.get('errors'))} failed, {r.get('skipped')} skipped")
PY
