//! D44/D46: the `nativeMeeting*` exports in a real JVM (mock ASR core).
//! Skips when no `java` (JDK 11+ source launcher) is on PATH.
#![cfg(feature = "jni")]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn jvm_meeting_round_trip() {
    if Command::new("java").arg("-version").output().is_err() {
        eprintln!("SKIP: no java on PATH");
        return;
    }
    // Same feature set as jni_smoke builds, so the shared cdylib is not rebuilt in between.
    let mut features = vec!["jni"];
    if cfg!(feature = "remote") {
        features.push("remote");
    }
    if cfg!(feature = "cloud") {
        features.push("cloud");
    }
    if cfg!(feature = "library") {
        features = vec!["jni", "remote", "library"];
        if cfg!(feature = "cloud") {
            features.push("cloud");
        }
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
        .join("tests/java/dev/murmur/core/JniMeeting.java");
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
    // Two chunks, each one dictation through the session's dictionary.
    let segs = lines.next().unwrap();
    assert_eq!(segs.matches("hello by the way").count(), 2, "{segs}");
    assert!(segs.contains("\"start_ms\""), "{segs}");
    let p: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(p["total_ms"], 26_000, "{p}");
    assert_eq!(p["done_ms"], 26_000, "{p}");
    assert_eq!(p["segments"], 2, "{p}");
    assert_eq!(lines.next().unwrap(), "KIND cancelled");
    assert_eq!(lines.next().unwrap(), "KIND bad_config");
    assert_eq!(lines.next().unwrap(), "{}");
}
