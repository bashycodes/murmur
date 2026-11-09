//! The shipped whisper.cpp must run on any x86-64 CPU with AVX2, not only
//! on the machine that built it. v0.9.1's CI exe was compiled on an
//! AVX-512 runner with ggml's GGML_NATIVE default, and crashed with
//! 0xc000001d (illegal instruction, a `vmovups zmm`) four seconds into the
//! first local whisper dictation on an Intel Core Ultra 9 288V.
//! `.cargo/config.toml` pins GGML_NATIVE=OFF; this guards it on CI.
#![cfg(all(feature = "whisper", target_arch = "x86_64"))]

#[test]
fn whisper_build_has_no_avx512_and_keeps_avx2() {
    let info = flow_core::whisper::build_cpu_features();
    assert!(
        !info.contains("AVX512"),
        "whisper.cpp was compiled for AVX-512 (GGML_NATIVE on an AVX-512 \
         build host?); it would crash on CPUs without it: {info}"
    );
    assert!(
        info.contains("AVX2 = 1"),
        "whisper.cpp lost its AVX2 kernels (slow decode): {info}"
    );
}
