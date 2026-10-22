#!/usr/bin/env bash
# Build-flavour gate (docs/hybrid-architecture.md §1.4, invariants I3/I4;
# docs/cloud-stt.md §9, invariant C2):
#   * client-only builds contain no network code (and no cloud STT client);
#   * server-only builds contain no ASR/LLM native libraries.
# Checked on the dependency graph (`cargo tree -e normal`), so nothing is
# compiled. Checks whose cargo feature does not exist yet (the shells add
# `remote` / `local` on their own branches) print SKIP; set
# CHECK_BUILD_MATRIX_STRICT=1 to make a SKIP fail (use after the merges).
# Checks of the shells' `cloud` feature (D18) are SKIP until W/A add it and
# count for strict mode only with CHECK_BUILD_MATRIX_STRICT_CLOUD=1.
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
check "flow-windows local,app (windows) has no WS client" "$WIN_NET" "${WIN[@]}" --features local,app "${WINT[@]}"
check "flow-windows local,app (windows) has no cloud client" "$CLOUD" "${WIN[@]}" --features local,app "${WINT[@]}"
check "flow-windows remote,cloud,app (windows) has no ASR/LLM libs" "$ASR" "${WIN[@]}" --features remote,cloud,app "${WINT[@]}"

if [ "$fail" -ne 0 ]; then
    echo "build matrix: FAILED"
    exit 1
fi
if [ "$skip" -ne 0 ] && [ "${CHECK_BUILD_MATRIX_STRICT:-0}" = "1" ]; then
    echo "build matrix: $skip check(s) skipped in strict mode"
    exit 1
fi
if [ "$cloud_skip" -ne 0 ] && [ "${CHECK_BUILD_MATRIX_STRICT_CLOUD:-0}" = "1" ]; then
    echo "build matrix: $cloud_skip cloud check(s) skipped in cloud-strict mode"
    exit 1
fi
echo "build matrix: ok ($skip skipped, $cloud_skip cloud pending)"
