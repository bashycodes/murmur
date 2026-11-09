// With `parakeet`, murmur-cli needs libsherpa-onnx-c-api.so / libonnxruntime.so
// at runtime. sherpa-onnx-sys copies them next to the binary, but its rpath
// link arg does not reach this crate's binaries, so add `$ORIGIN` here.
fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if std::env::var_os("CARGO_FEATURE_PARAKEET").is_some()
        && matches!(os.as_str(), "linux" | "android")
    {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
