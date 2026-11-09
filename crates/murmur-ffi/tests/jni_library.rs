//! D41: the model-library JNI exports (`dev.murmur.server.ServerLibraryNative`)
//! in a real JVM against an in-process murmur-server: list, a verified download
//! with progress callbacks, and cancel through the listener (a throwing
//! listener method is cleared, never left pending). Skips without `java`.
#![cfg(all(feature = "jni", feature = "library"))]

mod support;

use std::path::PathBuf;
use std::process::Command;

use support::library_server::{sha, weights, LibraryServer, TOKEN};

#[test]
fn jvm_lists_and_downloads() {
    if Command::new("java").arg("-version").output().is_err() {
        eprintln!("SKIP: no java on PATH");
        return;
    }
    // Same features as jni_smoke's build of the cdylib, so neither undoes the other.
    let mut features = vec!["jni", "remote", "library"];
    if cfg!(feature = "cloud") {
        features.push("cloud");
    }
    let st = Command::new(env!("CARGO"))
        .args([
            "build",
            "-q",
            "-p",
            "murmur-ffi",
            "--features",
            &features.join(","),
            "--lib",
        ])
        .status()
        .unwrap();
    assert!(st.success());
    let exe = std::env::current_exe().unwrap();
    let lib_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/java/dev/murmur/server/ServerLibraryNative.java");
    let s = LibraryServer::start();
    let d = tempfile::tempdir().unwrap();
    let dest = d.path().join("ggml-custom.bin");
    let part = d.path().join("ggml-custom.bin.part");
    let w = weights();
    let file = serde_json::json!({"name": "ggml-custom.bin", "size": w.len(), "sha256": sha(&w)})
        .to_string();
    let out = Command::new("java")
        .arg(format!("-Djava.library.path={}", lib_dir.display()))
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg(&src)
        .args([
            s.server_json(TOKEN),
            "my-whisper".into(),
            file,
            dest.display().to_string(),
            part.display().to_string(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut lines = stdout.lines();
    let l: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(l["ok"], true, "{l}");
    assert_eq!(l["models"][0]["id"], "my-whisper");
    assert_eq!(lines.next().unwrap(), r#"{"ok":true}"#);
    assert_eq!(lines.next().unwrap(), format!("PROGRESS {}", w.len()));
    assert_eq!(std::fs::read(&dest).unwrap(), w);
    let c: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(c["error"], "cancelled", "{c}");
    assert!(!d.path().join("ggml-custom.bin.2").exists());
    assert!(!d.path().join("ggml-custom.bin.part.2").exists());
}
