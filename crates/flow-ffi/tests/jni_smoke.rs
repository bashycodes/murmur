//! Load the cdylib into a real JVM and drive it through the JNI exports.
//! Skips when no `java` (JDK 11+ source launcher) is on PATH.
#![cfg(feature = "jni")]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn jvm_round_trip() {
    if Command::new("java").arg("-version").output().is_err() {
        eprintln!("SKIP: no java on PATH");
        return;
    }
    // `cargo test` does not emit the cdylib; build it (same profile/target dir).
    let st = Command::new(env!("CARGO"))
        .args([
            "build",
            "-q",
            "-p",
            "flow-ffi",
            "--features",
            "jni",
            "--lib",
        ])
        .status()
        .unwrap();
    assert!(st.success());
    // Integration tests live in target/<profile>/deps; the cdylib one level up.
    let exe = std::env::current_exe().unwrap();
    let lib_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
    assert!(
        lib_dir.join("libflow_ffi.so").exists(),
        "build the cdylib: {}",
        lib_dir.display()
    );
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/java/com/flowclone/core/JniSmoke.java");
    let out = Command::new("java")
        .arg(format!("-Djava.library.path={}", lib_dir.display()))
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg(&src)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut lines = stdout.lines();
    let commit: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(commit["status"], "RAW_TRANSCRIPT");
    assert_eq!(commit["final_text"], "hello by the way ");
    assert_eq!(commit["timings"]["audio_ms"], 1000);
    let cands: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(cands[0]["corrected"], "Wispr");
    assert!(lines.next().unwrap().contains("bad session config"));
}
