#!/usr/bin/env bash
# Build-flavour gate (docs/hybrid-architecture.md §1.4, invariants I3/I4):
#   * client-only builds contain no network code;
#   * server-only builds contain no ASR/LLM native libraries.
# Checked on the dependency graph (`cargo tree -e normal`), so nothing is
# compiled. Checks whose cargo feature does not exist yet (the shells add
# `remote` / `local` on their own branches) print SKIP; set
# CHECK_BUILD_MATRIX_STRICT=1 to make a SKIP fail (use after the merges).
#
# Usage: tools/check-build-matrix.sh   (from anywhere; exit 1 on a violation)
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NET='^(tokio|tokio-tungstenite|tungstenite|rustls|tokio-rustls|axum|hyper|hyper-util|reqwest|ureq|mio) v'
# Tauri itself uses tokio, so the Windows check looks for our client stack.
WIN_NET='^(flow-remote|tokio-tungstenite|tungstenite|axum) v'
ASR='^(whisper-rs|whisper-rs-sys|sherpa-onnx|sherpa-onnx-sys|llama-cpp-2|llama-cpp-sys-2|flow-asr-sherpa) v'
fail=0
skip=0

# check <label> <forbidden-regex> <cargo tree args...>
check() {
    local label="$1" re="$2"
    shift 2
    local out
    if ! out=$(cargo tree -e normal --prefix none --format '{p}' "$@" 2>&1); then
        if grep -qiE "does not (have|contain) (the |this |these )?features?|feature .* (is not|not) (defined|found)|none of the selected packages contains (these|this) feature|Package .* does not have feature" <<<"$out"; then
            echo "SKIP  $label (feature not defined yet)"
            skip=$((skip + 1))
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

if [ "$fail" -ne 0 ]; then
    echo "build matrix: FAILED"
    exit 1
fi
if [ "$skip" -ne 0 ] && [ "${CHECK_BUILD_MATRIX_STRICT:-0}" = "1" ]; then
    echo "build matrix: $skip check(s) skipped in strict mode"
    exit 1
fi
echo "build matrix: ok ($skip skipped)"
