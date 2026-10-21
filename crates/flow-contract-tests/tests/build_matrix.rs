//! I3/I4 build-flavour gate: runs `tools/check-build-matrix.sh` (cargo tree
//! only, nothing is compiled). Checks for shell features that do not exist
//! yet are reported as SKIP; `CHECK_BUILD_MATRIX_STRICT=1` fails them.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn build_matrix_gate_passes() {
    let out = Command::new("bash")
        .arg(root().join("tools/check-build-matrix.sh"))
        .output()
        .expect("run check-build-matrix.sh");
    let text = String::from_utf8_lossy(&out.stdout);
    eprintln!("{text}{}", String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "build matrix gate failed:\n{text}");
    assert!(text.contains("ok    flow-router has no network deps"));
    assert!(text.contains("ok    flow-ffi jni,whisper (local flavour) has no network deps"));
}

/// The gate would see a network crate if one were there.
#[test]
fn gate_detects_network_code_in_flow_remote() {
    let out = Command::new(env!("CARGO"))
        .current_dir(root())
        .args([
            "tree",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "-p",
            "flow-remote",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.lines().any(|l| l.starts_with("tokio-tungstenite v")),
        "{text}"
    );
}

/// D18 C2: the gate's cloud pattern matches what flow-cloud really pulls
/// in, and the client-only checks ran (not skipped).
#[test]
fn gate_detects_the_cloud_client_and_checks_client_only_builds() {
    let out = Command::new(env!("CARGO"))
        .current_dir(root())
        .args([
            "tree",
            "-e",
            "normal",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "-p",
            "flow-cloud",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for c in ["reqwest v", "rustls v", "hyper v"] {
        assert!(text.lines().any(|l| l.starts_with(c)), "{c}: {text}");
    }
    let out = Command::new("bash")
        .arg(root().join("tools/check-build-matrix.sh"))
        .output()
        .unwrap();
    let gate = String::from_utf8_lossy(&out.stdout);
    for line in [
        "ok    flow-ffi jni,whisper,llm,parakeet (local flavour) has no cloud client",
        "ok    flow-windows local,app has no cloud client",
        "ok    flow-router has no cloud client",
        "ok    flow-cloud has no ASR/LLM libs",
    ] {
        assert!(gate.contains(line), "missing {line:?}:\n{gate}");
    }
}
