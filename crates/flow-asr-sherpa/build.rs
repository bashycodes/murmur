// sherpa-onnx-sys copies libsherpa-onnx-c-api.so + libonnxruntime.so next to
// the build outputs, but its `-rpath,$ORIGIN` link arg only applies to its
// own targets. Give this crate's examples the same rpath so they run without
// LD_LIBRARY_PATH (tests run under cargo, which sets the search path).
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(os.as_str(), "linux" | "android") {
        println!("cargo:rustc-link-arg-examples=-Wl,-rpath,$ORIGIN");
    }
}
