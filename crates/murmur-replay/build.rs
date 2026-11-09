// sherpa-onnx-sys copies libsherpa-onnx-c-api.so + libonnxruntime.so next to
// the build outputs; give the binary an $ORIGIN rpath (as murmur-asr-sherpa
// does for its examples) so it runs without LD_LIBRARY_PATH.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(os.as_str(), "linux" | "android") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
