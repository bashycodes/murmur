//! I3/I4 build-flavour gate: runs `tools/check-build-matrix.sh` (cargo tree
//! plus the D28 host scan). Strict and cloud-strict mode are the script's
//! defaults, so a SKIP fails the gate.

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

fn gate() -> String {
    let out = Command::new("bash")
        .arg(root().join("tools/check-build-matrix.sh"))
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// D28: the cloud build talks to OpenRouter only. The host-string checks
/// ran and passed.
#[test]
fn gate_checks_provider_host_strings() {
    let g = gate();
    for line in [
        "ok    cloud-build Rust sources name no provider host but openrouter.ai",
        "ok    flow-cloud/src URL literals are https://openrouter.ai only",
        "ok    flow-cloud artefact contains no provider host but openrouter.ai",
        "ok    -p flow-ffi --features jni,remote,cloud does not enable flow-cloud/mock-server",
        "ok    shell sources name no provider host but openrouter.ai",
        "build matrix: ok (0 skipped, 0 cloud pending)",
    ] {
        assert!(g.contains(line), "missing {line:?}:\n{g}");
    }
}

/// The host scan would fail on a planted provider host in a shipped
/// artefact (FLOW_CLOUD_ARTIFACTS), and pass on a clean one.
#[test]
fn gate_detects_a_provider_host_in_an_artefact() {
    let dir = std::env::temp_dir().join(format!("flow-hostscan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("libbad.so");
    let good = dir.join("libgood.so");
    std::fs::write(&bad, b"\x7fELF....https://api.deepgram.com/v1/listen\0").unwrap();
    std::fs::write(&good, b"\x7fELF....https://openrouter.ai/api/v1\0").unwrap();
    let run = |p: &std::path::Path| {
        Command::new("bash")
            .arg(root().join("tools/check-build-matrix.sh"))
            .env("FLOW_CLOUD_ARTIFACTS", p)
            .env("CHECK_BUILD_MATRIX_NO_BUILD", "1")
            .output()
            .unwrap()
    };
    let b = run(&bad);
    let bt = String::from_utf8_lossy(&b.stdout);
    assert!(!b.status.success(), "{bt}");
    assert!(
        bt.contains("FAIL  libbad.so contains a non-OpenRouter provider host"),
        "{bt}"
    );
    let g = run(&good);
    let gt = String::from_utf8_lossy(&g.stdout);
    assert!(g.status.success(), "{gt}");
    assert!(gt.contains("ok    libgood.so contains no provider host but openrouter.ai"));
    let _ = std::fs::remove_dir_all(&dir);
}
