#!/usr/bin/env bash
# Build the host stub libmurmur_ffi.so and run the JNI link test against it.
# Proves NativeMurmurCore's external declarations match JNI_CONTRACT.md symbols.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../../.." && pwd)"
jdk="${JAVA_HOME_21:-$HOME/.gradle/jdks/eclipse_adoptium-21-amd64-linux.2}"
out="$app/app/build/jni-stub"
mkdir -p "$out"
gcc -shared -fPIC -O1 -I"$jdk/include" -I"$jdk/include/linux" -o "$out/libmurmur_ffi.so" "$here/murmur_ffi_stub.c"
cd "$app"
./gradlew testLocalDebugUnitTest -q -Pmurmur.jniStubDir="$out" --tests 'dev.murmur.core.NativeMurmurCoreStubTest'
python3 -I - "$app" <<'EOF'
import sys, xml.etree.ElementTree as ET, pathlib
p = pathlib.Path(sys.argv[1], "app/build/test-results/testLocalDebugUnitTest/TEST-dev.murmur.core.NativeMurmurCoreStubTest.xml")
r = ET.parse(p).getroot()
print(f"jni stub link test: {r.get('tests')} run, {int(r.get('failures'))+int(r.get('errors'))} failed, {r.get('skipped')} skipped")
EOF
