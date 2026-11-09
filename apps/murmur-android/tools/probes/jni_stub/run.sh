#!/usr/bin/env bash
# Build the host stub libflow_ffi.so and run the JNI link test against it.
# Proves NativeFlowCore's external declarations match JNI_CONTRACT.md symbols.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../../.." && pwd)"
jdk="${JAVA_HOME_21:-$HOME/.gradle/jdks/eclipse_adoptium-21-amd64-linux.2}"
out="$app/app/build/jni-stub"
mkdir -p "$out"
gcc -shared -fPIC -O1 -I"$jdk/include" -I"$jdk/include/linux" -o "$out/libflow_ffi.so" "$here/flow_ffi_stub.c"
cd "$app"
./gradlew testLocalDebugUnitTest -q -Pflow.jniStubDir="$out" --tests 'dev.flowclone.core.NativeFlowCoreStubTest'
python3 -I - "$app" <<'EOF'
import sys, xml.etree.ElementTree as ET, pathlib
p = pathlib.Path(sys.argv[1], "app/build/test-results/testLocalDebugUnitTest/TEST-dev.flowclone.core.NativeFlowCoreStubTest.xml")
r = ET.parse(p).getroot()
print(f"jni stub link test: {r.get('tests')} run, {int(r.get('failures'))+int(r.get('errors'))} failed, {r.get('skipped')} skipped")
EOF
