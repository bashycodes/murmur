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
    // Same features as this test binary, so `--features jni,remote` exercises the server route.
    // (+ `cloud` when set, so jni_cloud's build of the same cdylib is not undone in between.)
    let mut features = vec!["jni"];
    if cfg!(feature = "remote") {
        features.push("remote");
    }
    if cfg!(feature = "cloud") {
        features.push("cloud");
    }
    let features = features.join(",");
    let st = Command::new(env!("CARGO"))
        .args([
            "build",
            "-q",
            "-p",
            "flow-ffi",
            "--features",
            &features,
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
        .join("tests/java/dev/flowclone/core/JniSmoke.java");
    // With `remote`: an in-process flow-server (mock ASR) the JVM routes to.
    #[cfg(feature = "remote")]
    let server = InProcessServer::start("hello btw");
    #[cfg(feature = "remote")]
    let server_args = vec![server.addr.clone(), TOKEN.to_string()];
    #[cfg(not(feature = "remote"))]
    let server_args: Vec<String> = Vec::new();
    let out = Command::new("java")
        .arg(format!("-Djava.library.path={}", lib_dir.display()))
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg(&src)
        .args(&server_args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next().unwrap(),
        concat!("flow-ffi ", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(lines.next().unwrap(), "null", "no partials configured");
    let commit: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(commit["status"], "RAW_TRANSCRIPT", "{commit}");
    assert_eq!(commit["final_text"], "hello by the way ");
    assert_eq!(commit["timings"]["audio_ms"], 1000);
    let cands: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(cands[0]["corrected"], "Wispr");
    assert!(lines.next().unwrap().contains("bad session config"));
    let null_commit: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(null_commit["status"], "ERROR");
    assert_eq!(lines.next().unwrap(), "[]");

    // D17 router round trip (client only, mock local handed over by handle).
    let engine = if cfg!(feature = "remote") {
        "remote"
    } else {
        "local"
    };
    // 2026-10-22: start info + poll carry the live route.
    let start: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(start["queued"], false, "{start}");
    assert_eq!(start["route"]["engine"], engine, "{start}");
    let poll: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(poll["route"], start["route"], "{poll}");
    assert_eq!(poll["endpoint_reached"], false, "{poll}");
    assert!(poll["speech_ms"].is_number(), "endpointing on: {poll}");
    let idle: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert!(idle.get("route").is_none(), "{idle}");
    assert_eq!(lines.next().unwrap(), "INFO null KIND bad_config");
    let routed: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(routed["status"], "RAW_TRANSCRIPT", "{routed}");
    assert_eq!(routed["final_text"], "hello by the way ");
    assert_eq!(routed["route"]["engine"], engine, "{routed}");
    let status: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(status["last"]["engine"], engine, "{status}");
    assert_eq!(status["circuit"]["open"], false);
    let cands: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(
        cands[0]["corrected"], "Wispr",
        "router learn_edit uses the last dictionary"
    );
    assert_eq!(lines.next().unwrap(), "KIND bad_config");
    assert_eq!(lines.next().unwrap(), "KIND no_engine_enabled");
    let caps: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(caps["remote"], cfg!(feature = "remote"), "{caps}");
    assert_eq!(caps["proto"], 1);
    let probe: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(probe["ok"], false, "{probe}");
    let want = if cfg!(feature = "remote") {
        "connect_failed"
    } else {
        "server_not_configured"
    };
    assert_eq!(probe["error"], want, "{probe}");
    if cfg!(feature = "remote") {
        let ok: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(ok["ok"], true, "{ok}");
        assert!(ok["rtt_ms"]["median"].is_number(), "{ok}");
        assert_eq!(ok["postprocess_match"], true, "{ok}");
    }
}

#[cfg(feature = "remote")]
const TOKEN: &str = "jni-smoke-token-0123456789abcdef";

#[cfg(feature = "remote")]
struct InProcessServer {
    addr: String,
    _handle: flow_server::ServerHandle,
    _rt: tokio::runtime::Runtime,
}

#[cfg(feature = "remote")]
impl InProcessServer {
    fn start(text: &str) -> Self {
        use flow_core::config::{AsrBackend, ModelConfig};
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let cfg = flow_server::config::Config {
            bind: vec!["127.0.0.1:0".into()],
            token: Some(TOKEN.into()),
            model: ModelConfig {
                asr_backend: AsrBackend::Mock { text: text.into() },
                ..Default::default()
            },
            ..Default::default()
        };
        let handle = rt.block_on(async {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            flow_server::serve(cfg, vec![l])
                .await
                .expect("server starts")
        });
        Self {
            addr: handle.addr().to_string(),
            _handle: handle,
            _rt: rt,
        }
    }
}

/// Every symbol in JNI_CONTRACT.md is exported by the cdylib, and no stale
/// `com.flowclone` names remain.
#[test]
fn contract_symbols_exported() {
    let contract = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/flow-android/JNI_CONTRACT.md"),
    )
    .unwrap();
    let want: std::collections::BTreeSet<&str> = contract
        .split('`')
        .filter(|w| w.starts_with("Java_dev_flowclone_core_NativeFlowCore_native"))
        .collect();
    assert_eq!(want.len(), 31, "{want:?}");
    let src = include_str!("../src/jni_api.rs");
    let have: std::collections::BTreeSet<&str> = src
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.starts_with("Java_"))
        .collect();
    assert_eq!(have, want);
}
