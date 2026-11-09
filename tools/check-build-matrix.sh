#!/usr/bin/env bash
# Build-flavour gate (docs/hybrid-architecture.md §1.4, invariants I3/I4;
# docs/cloud-stt.md §9, invariant C2):
#   * client-only builds contain no network code (and no cloud STT client);
#   * server-only builds contain no ASR/LLM native libraries.
# Checked on the dependency graph (`cargo tree -e normal`), so nothing is
# compiled. A check whose cargo feature does not exist prints SKIP, and a
# SKIP fails the gate: strict mode (CHECK_BUILD_MATRIX_STRICT) and
# cloud-strict mode (CHECK_BUILD_MATRIX_STRICT_CLOUD) are on by default now
# that every flavour and both shells' D28 work are merged. Set either to 0
# only on a feature branch that has not caught up yet.
#
# D28: cloud builds talk to OpenRouter only. The host-string checks below
# fail on any other speech/LLM provider host in (a) the Rust sources linked
# into cloud builds, (b) flow-cloud's compiled artefact (built here, dev
# profile, usually cached) and (c) any shipped artefact listed in
# FLOW_CLOUD_ARTIFACTS (space-separated .so/.exe/.apk paths), plus the
# shells' main sources.
# CHECK_BUILD_MATRIX_NO_BUILD=1 skips (b).
# D80: NOTICE and THIRD-PARTY-LICENSES are current (tools/notice/crate-licences.py
# --check); CHECK_BUILD_MATRIX_NOTICES=0 skips it.
#
# Usage: tools/check-build-matrix.sh   (from anywhere; exit 1 on a violation)
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NET='^(tokio|tokio-tungstenite|tungstenite|rustls|tokio-rustls|axum|hyper|hyper-util|reqwest|ureq|mio) v'
# Tauri itself uses tokio, so the Windows check looks for our client stack.
WIN_NET='^(flow-remote|tokio-tungstenite|tungstenite|axum) v'
ASR='^(whisper-rs|whisper-rs-sys|sherpa-onnx|sherpa-onnx-sys|llama-cpp-2|llama-cpp-sys-2|flow-asr-sherpa) v'
# Cloud STT client stack (C2): never in client-only builds.
CLOUD='^(flow-cloud|reqwest|hyper|hyper-util|hyper-rustls|rustls|tokio-rustls|webpki-roots|rustls-webpki) v'
fail=0
skip=0
cloud_skip=0

# D26: the product version copies agree with VERSION (tools/check-version.sh).
if vout=$(tools/check-version.sh 2>&1 >/dev/null); then
    echo "ok    product version $(cat VERSION) is consistent"
else
    echo "FAIL  product version copies disagree (tools/check-version.sh):"
    sed 's/^/      /' <<<"$vout"
    fail=1
fi

# D80: NOTICE's crate section and THIRD-PARTY-LICENSES match the lockfiles.
# Needs both workspaces' crates in the registry cache (cargo fetch). The app
# workflows set CHECK_BUILD_MATRIX_NOTICES=0: repo-checks runs it on every push.
if [ "${CHECK_BUILD_MATRIX_NOTICES:-1}" = 0 ]; then
    echo "off   third-party notices (CHECK_BUILD_MATRIX_NOTICES=0; repo-checks runs it)"
elif nout=$(python3 -I tools/notice/crate-licences.py --check 2>&1); then
    echo "ok    $nout"
else
    echo "FAIL  third-party notices are stale (tools/notice/crate-licences.py):"
    sed 's/^/      /' <<<"$nout"
    fail=1
fi

# check <label> <forbidden-regex> <cargo tree args...>
check() {
    local label="$1" re="$2"
    shift 2
    local out
    if ! out=$(cargo tree -e normal --prefix none --format '{p}' "$@" 2>&1); then
        if grep -qiE "does not (have|contain) (the |this |these )?features?|feature .* (is not|not) (defined|found)|none of the selected packages contains (these|this) feature|Package .* does not have feature" <<<"$out"; then
            echo "SKIP  $label (feature not defined yet)"
            if [[ "$label" == *cloud* ]]; then
                cloud_skip=$((cloud_skip + 1))
            else
                skip=$((skip + 1))
            fi
            return
        fi
        echo "ERROR $label: cargo tree failed:"
        echo "$out" | head -5 | sed 's/^/      /'
        fail=1
        return
    fi
    local bad
    bad=$(grep -E "$re" <<<"$out" | sort -u)
    if [ -n "$bad" ]; then
        echo "FAIL  $label contains:"
        echo "$bad" | sed 's/^/      /'
        fail=1
    else
        echo "ok    $label"
    fi
}

# Router stack: no network code below the injected remote factory.
check "flow-protocol has no network deps" "$NET" -p flow-protocol
check "flow-router has no network deps" "$NET" -p flow-router
check "flow-remote has no ASR/LLM libs" "$ASR" -p flow-remote --features tls
# flow-ffi (Android / C ABI): client-only vs server-only flavours.
check "flow-ffi jni,whisper (local flavour) has no network deps" "$NET" -p flow-ffi --features jni,whisper
check "flow-ffi jni,whisper,llm,parakeet (local flavour) has no network deps" "$NET" -p flow-ffi --features jni,whisper,llm,parakeet
check "flow-ffi jni,remote (remote flavour) has no ASR/LLM libs" "$ASR" -p flow-ffi --features jni,remote
# D41: the model-library client (hybrid / remote only) links no ASR/LLM code either.
check "flow-ffi jni,remote,cloud,library (remote flavour) has no ASR/LLM libs" "$ASR" -p flow-ffi --features jni,remote,cloud,library
# Windows shell (own workspace).
WIN=(--manifest-path apps/flow-windows/Cargo.toml --locked --no-default-features)
check "flow-windows local,app has no WS client" "$WIN_NET" "${WIN[@]}" --features local,app
check "flow-windows remote,app has no ASR/LLM libs" "$ASR" "${WIN[@]}" --features remote,app

# Cloud STT (D18 C2): the client-only flavours link no cloud client; the
# cloud crate itself links no ASR/LLM native code.
check "flow-core has no cloud client" "$CLOUD" -p flow-core
check "flow-router has no cloud client" "$CLOUD" -p flow-router
check "flow-cloud has no ASR/LLM libs" "$ASR" -p flow-cloud
check "flow-ffi jni,whisper,llm,parakeet (local flavour) has no cloud client" "$CLOUD" -p flow-ffi --features jni,whisper,llm,parakeet
check "flow-windows local,app has no cloud client" "$CLOUD" "${WIN[@]}" --features local,app
check "flow-ffi jni,remote,cloud (remote+cloud flavour) has no ASR/LLM libs" "$ASR" -p flow-ffi --features jni,remote,cloud
check "flow-windows remote,cloud,app has no ASR/LLM libs" "$ASR" "${WIN[@]}" --features remote,cloud,app

# The same flavour checks on the shells' real targets (review P4): `cargo
# tree` on the host misses target-specific dependencies
# (`[target.'cfg(...)'.dependencies]`). Resolution only; no target
# toolchain is needed.
ANDROID=(--target aarch64-linux-android)
WINT=(--target x86_64-pc-windows-msvc)
check "flow-ffi jni,whisper,llm,parakeet (local flavour, android) has no network deps" "$NET" -p flow-ffi --features jni,whisper,llm,parakeet "${ANDROID[@]}"
check "flow-ffi jni,whisper,llm,parakeet (local flavour, android) has no cloud client" "$CLOUD" -p flow-ffi --features jni,whisper,llm,parakeet "${ANDROID[@]}"
check "flow-ffi jni,remote,cloud (remote+cloud flavour, android) has no ASR/LLM libs" "$ASR" -p flow-ffi --features jni,remote,cloud "${ANDROID[@]}"
check "flow-ffi jni,remote,cloud,admin (remote flavour + D39 admin, android) has no ASR/LLM libs" "$ASR" -p flow-ffi --features jni,remote,cloud,admin "${ANDROID[@]}"
check "flow-ffi jni,remote,cloud,library (remote flavour, android) has no ASR/LLM libs" "$ASR" -p flow-ffi --features jni,remote,cloud,library "${ANDROID[@]}"
check "flow-windows local,app (windows) has no WS client" "$WIN_NET" "${WIN[@]}" --features local,app "${WINT[@]}"
check "flow-windows local,app (windows) has no cloud client" "$CLOUD" "${WIN[@]}" --features local,app "${WINT[@]}"
check "flow-windows remote,cloud,app (windows) has no ASR/LLM libs" "$ASR" "${WIN[@]}" --features remote,cloud,app "${WINT[@]}"

# ---- D28: no non-OpenRouter provider host in cloud builds ------------------
# Known third-party speech / LLM API hosts (the D18 presets and the
# providers OpenRouter routes to). A hit means a direct provider path
# exists next to OpenRouter.
PROVIDER_HOSTS='(api\.elevenlabs\.io|api\.deepgram\.com|api\.assemblyai\.com|api\.groq\.com|api\.openai\.com|api\.anthropic\.com|api\.mistral\.ai|api\.together\.(ai|xyz)|api\.fireworks\.ai|api\.deepinfra\.com|api\.cerebras\.ai|api\.sambanova\.ai|generativelanguage\.googleapis\.com|speech\.googleapis\.com|api\.speechmatics\.com|api\.rev\.ai|api\.gladia\.io|api\.x\.ai|stt\.speech\.microsoft\.com|cognitiveservices\.azure\.com|api\.cohere\.(ai|com)|api\.perplexity\.ai)'
# (a) Rust sources linked into cloud builds (not tests / fixtures).
hits=$(grep -rnE "$PROVIDER_HOSTS" crates/flow-cloud/src crates/flow-router/src crates/flow-core/src \
    crates/flow-ffi/src crates/flow-remote/src crates/flow-protocol/src 2>/dev/null)
if [ -n "$hits" ]; then
    echo "FAIL  cloud-build Rust sources name a non-OpenRouter provider host:"
    echo "$hits" | head -20 | sed 's/^/      /'
    fail=1
else
    echo "ok    cloud-build Rust sources name no provider host but openrouter.ai"
fi
# flow-cloud's own URL literals: openrouter.ai only.
# Non-test code only: testing.rs and `#[cfg(test)]` modules (at the end of
# each file) are excluded.
urls=$(find crates/flow-cloud/src -name '*.rs' ! -name testing.rs -exec awk '/^#\[cfg\(test\)\]/{exit} {print}' {} \; |
    grep -oE 'https?://[A-Za-z0-9.-]+' | sort -u | grep -vE '^https://openrouter\.ai$')
if [ -n "$urls" ]; then
    echo "FAIL  flow-cloud/src has URL literals other than https://openrouter.ai:"
    echo "$urls" | sed 's/^/      /'
    fail=1
else
    echo "ok    flow-cloud/src URL literals are https://openrouter.ai only"
fi
# Shell sources (main code only).
shell_hits=$(grep -rnE "$PROVIDER_HOSTS" apps/flow-windows/src apps/flow-windows/ui/src \
    apps/flow-android/app/src/main 2>/dev/null)
if [ -n "$shell_hits" ]; then
    if [ "${CHECK_BUILD_MATRIX_STRICT_CLOUD:-1}" = "1" ]; then
        echo "FAIL  shell sources name a non-OpenRouter provider host:"
        fail=1
    else
        echo "SKIP  shell sources still name non-OpenRouter provider hosts (cloud-strict off):"
        cloud_skip=$((cloud_skip + 1))
    fi
    echo "$shell_hits" | awk -F: '{print $1}' | sort | uniq -c | sed 's/^/      /'
else
    echo "ok    shell sources name no provider host but openrouter.ai"
fi
# (b) flow-cloud's compiled artefact, as a cloud build links it (no
# mock-server feature).
if [ "${CHECK_BUILD_MATRIX_NO_BUILD:-0}" != "1" ]; then
    if arts=$(cargo build -p flow-cloud --lib --message-format=json 2>/dev/null |
        grep -o '"filenames":\[[^]]*\]' | grep 'flow_cloud' | grep -oE '"[^"]+\.(rlib|rmeta|so|a)"' | tr -d '"'); then
        bad=""
        for a in $arts; do
            [ -f "$a" ] || continue
            if grep -aqE "$PROVIDER_HOSTS" "$a"; then bad="$bad $a"; fi
        done
        if [ -z "$arts" ]; then
            echo "ERROR flow-cloud artefact: no build output found"
            fail=1
        elif [ -n "$bad" ]; then
            echo "FAIL  flow-cloud artefact contains a non-OpenRouter provider host:$bad"
            fail=1
        else
            echo "ok    flow-cloud artefact contains no provider host but openrouter.ai"
        fi
    else
        echo "ERROR flow-cloud artefact: cargo build failed"
        fail=1
    fi
else
    echo "SKIP  flow-cloud artefact host scan (CHECK_BUILD_MATRIX_NO_BUILD=1)"
fi
# (c) Shipped artefacts (APK .so, Windows .exe), when given.
for a in ${FLOW_CLOUD_ARTIFACTS:-}; do
    if [ ! -f "$a" ]; then
        echo "ERROR $a: not found"
        fail=1
    elif [[ "$a" == *.apk ]]; then
        if unzip -p "$a" 'lib/*/libflow_ffi*.so' 2>/dev/null | grep -aqE "$PROVIDER_HOSTS"; then
            echo "FAIL  $(basename "$a") native lib contains a non-OpenRouter provider host"
            fail=1
        else
            echo "ok    $(basename "$a") native lib contains no provider host but openrouter.ai"
        fi
    elif grep -aqE "$PROVIDER_HOSTS" "$a"; then
        echo "FAIL  $(basename "$a") contains a non-OpenRouter provider host"
        fail=1
    else
        echo "ok    $(basename "$a") contains no provider host but openrouter.ai"
    fi
done
# The test-only mock seam (`mock-server`: plain-http base-URL override)
# must never be enabled in a shipped cloud flavour.
for spec in "-p flow-ffi --features jni,remote,cloud" \
    "--manifest-path apps/flow-windows/Cargo.toml --locked --no-default-features --features remote,cloud,app"; do
    # shellcheck disable=SC2086
    if out=$(cargo tree -e features,normal --prefix none -i flow-cloud $spec 2>&1); then
        if grep -q 'flow-cloud feature "mock-server"' <<<"$out"; then
            echo "FAIL  $spec enables flow-cloud/mock-server"
            fail=1
        else
            echo "ok    $spec does not enable flow-cloud/mock-server"
        fi
    elif grep -qiE "does not (have|contain)|not (defined|found)|did not match" <<<"$out"; then
        echo "SKIP  $spec mock-server check (feature not defined yet)"
        cloud_skip=$((cloud_skip + 1))
    else
        echo "ERROR $spec: cargo tree failed"
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    echo "build matrix: FAILED"
    exit 1
fi
if [ "$skip" -ne 0 ] && [ "${CHECK_BUILD_MATRIX_STRICT:-1}" = "1" ]; then
    echo "build matrix: $skip check(s) skipped in strict mode"
    exit 1
fi
if [ "$cloud_skip" -ne 0 ] && [ "${CHECK_BUILD_MATRIX_STRICT_CLOUD:-1}" = "1" ]; then
    echo "build matrix: $cloud_skip cloud check(s) skipped in cloud-strict mode"
    exit 1
fi
echo "build matrix: ok ($skip skipped, $cloud_skip cloud pending)"
