#!/usr/bin/env bash
# D28 host test: build crates/murmur-ffi for the HOST with features jni,cloud and run
# NativeMurmurCoreCloudHostTest: the shipped Kotlin OpenRouter path (DevSettings → EngineRoute →
# nativeOpenRouter, nativeMigrateCloud, nativeCloudCatalog, nativeCleanupStatus,
# nativeSetCloudModel / nativeSetCleanup, nativeCloudKeyInfo) against the real router +
# murmur-cloud. The OpenRouter address is fixed, so the test never sets a key: every cloud call is
# refused up front. No keys, no network.
# Usage: tools/probes/host_cloud/run.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../../.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
(cd "$repo" && cargo build -q -p murmur-ffi --release --features jni,cloud --lib)
lib="$repo/target/release"
test -f "$lib/libmurmur_ffi.so"
cd "$app"
./gradlew testHybridDebugUnitTest -q --rerun -Pmurmur.jniLibDir="$lib" --tests 'dev.murmur.core.NativeMurmurCoreCloudHostTest'
python3 -I - "$app" <<'PY'
import sys, xml.etree.ElementTree as ET, pathlib
p = pathlib.Path(sys.argv[1], "app/build/test-results/testHybridDebugUnitTest/TEST-dev.murmur.core.NativeMurmurCoreCloudHostTest.xml")
r = ET.parse(p).getroot()
print(f"host cloud test: {r.get('tests')} run, {int(r.get('failures'))+int(r.get('errors'))} failed, {r.get('skipped')} skipped")
PY
