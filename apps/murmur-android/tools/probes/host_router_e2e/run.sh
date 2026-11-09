#!/usr/bin/env bash
# Host end-to-end of the hybrid path without a device (D17):
#   murmur-server (mock ASR, 127.0.0.1, token auth)  <-WS-  host libmurmur_ffi.so (jni,remote)
#   <- shipped Kotlin NativeMurmurCore + EngineRoute (NativeMurmurCoreHostTest.routerRoundTripThroughTheShellsAssembly)
# Asserts route.engine = remote, RouteStatus, update/bad_config/both-off kinds and Test
# connection OK. (Fallback on server death is covered by murmur-contract-tests toggle_matrix.)
# Usage: tools/probes/host_router_e2e/run.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
app="$(cd "$here/../../.." && pwd)"
repo="$(cd "$app/../.." && pwd)"
work="$app/app/build/host-router-e2e"
rm -rf "$work"; mkdir -p "$work"
export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS:--I/usr/lib/gcc/x86_64-linux-gnu/13/include}"
(cd "$repo" && cargo build -q -p murmur-ffi --features jni,remote --lib && cargo build -q -p murmur-server --no-default-features --bin murmur-server)
port=$((20000 + RANDOM % 20000))
token="host-e2e-token-0123456789abcdef"
printf '%s' "$token" > "$work/token"; chmod 600 "$work/token"
cat > "$work/config.toml" <<EOF
bind = ["127.0.0.1:$port"]
token_file = "$work/token"
workers = 1
[model]
asr_backend = { mock = { text = "hello btw" } }
EOF
"$repo/target/debug/murmur-server" --config "$work/config.toml" > "$work/server.log" 2>&1 &
srv=$!
trap 'kill $srv 2>/dev/null || true' EXIT
for _ in $(seq 50); do grep -q "ready" "$work/server.log" && break; sleep 0.2; done
grep -q "ready" "$work/server.log" || { cat "$work/server.log"; exit 1; }
cd "$app"
./gradlew --offline testLocalDebugUnitTest -q --rerun -Pmurmur.jniLibDir="$repo/target/debug" \
  -Pmurmur.server="127.0.0.1:$port/$token" --tests 'dev.murmur.core.NativeMurmurCoreHostTest' 2>&1 | grep -E "HOST-ROUTER|FAILED|AssertionError" || true
python3 -I - "$app" <<'PY'
import sys, xml.etree.ElementTree as ET, pathlib
p = pathlib.Path(sys.argv[1], "app/build/test-results/testLocalDebugUnitTest/TEST-dev.murmur.core.NativeMurmurCoreHostTest.xml")
r = ET.parse(p).getroot()
bad = int(r.get('failures')) + int(r.get('errors'))
for c in r.iter("testcase"):
    el = c.find("failure") if c.find("failure") is not None else c.find("error")
    if el is not None: print("FAIL", c.get("name"), (el.get("message") or "")[:300])
print(f"host router e2e (server up): {r.get('tests')} run, {bad} failed, {r.get('skipped')} skipped")
sys.exit(1 if bad else 0)
PY
grep -E "sid|session|status" "$work/server.log" | tail -3 || true
