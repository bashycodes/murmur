//! S10: no plain-ws:// bind on a wildcard or public address unless the
//! operator passes `--insecure-bind-any`.

mod common;

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::*;

#[tokio::test(flavor = "multi_thread")]
async fn serve_refuses_wildcard_and_public_binds_without_tls() {
    for bind in [
        "0.0.0.0:7313",
        "[::]:7313",
        "8.8.8.8:7313",
        "[2001:db8::1]:7313",
    ] {
        let mut c = mock_cfg("x");
        c.bind = vec!["127.0.0.1:7313".into(), bind.into()];
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let e = flow_server::serve(c, vec![l]).await.err();
        let e = e.unwrap_or_else(|| panic!("{bind} was accepted"));
        assert!(e.contains("--insecure-bind-any"), "{bind}: {e}");
    }
    for bind in [
        "192.168.1.5:7313",
        "10.1.2.3:7313",
        "172.20.0.1:7313",
        "100.101.102.103:7313",
        "169.254.10.1:7313",
        "[fd7a:115c:a1e0::5]:7313",
        "[fe80::1]:7313",
        "[::1]:7313",
    ] {
        let mut c = mock_cfg("x");
        c.bind = vec![bind.into()];
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert!(flow_server::serve(c, vec![l]).await.is_ok(), "{bind}");
    }
    let mut c = mock_cfg("x");
    c.bind = vec!["0.0.0.0:7313".into()];
    c.insecure_bind_any = true;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(flow_server::serve(c, vec![l]).await.is_ok());
}

fn write_cfg(dir: &std::path::Path, bind: &str) -> std::path::PathBuf {
    let token = dir.join("token");
    std::fs::write(&token, format!("{TOKEN}\n")).unwrap();
    let p = dir.join("config.toml");
    std::fs::write(
        &p,
        format!(
            "bind = [\"{bind}\"]\ntoken_file = {token:?}\n[model]\nasr_engine = {{ mock = {{ text = \"x\" }} }}\n"
        ),
    )
    .unwrap();
    p
}

#[test]
fn the_binary_refuses_0_0_0_0_and_check_config_says_why() {
    let d = tempfile::tempdir().unwrap();
    let cfg = write_cfg(d.path(), "0.0.0.0:0");
    for sub in [&["check-config"][..], &[][..]] {
        let out = Command::new(env!("CARGO_BIN_EXE_flow-server"))
            .args(sub)
            .arg("--config")
            .arg(&cfg)
            .env_remove("RUST_LOG")
            .output()
            .unwrap();
        assert!(!out.status.success(), "{sub:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("--insecure-bind-any"), "{sub:?}: {err}");
    }
}

#[test]
fn insecure_bind_any_starts_with_a_loud_warning() {
    let d = tempfile::tempdir().unwrap();
    let cfg = write_cfg(d.path(), "0.0.0.0:0");
    let mut child = Command::new(env!("CARGO_BIN_EXE_flow-server"))
        .arg("--config")
        .arg(&cfg)
        .arg("--insecure-bind-any")
        .env_remove("RUST_LOG")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    let mut err = child.stderr.take().unwrap();
    let mut log = String::new();
    let mut buf = [0u8; 4096];
    while !log.contains("ready") && t0.elapsed() < Duration::from_secs(20) {
        let n = err.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        log.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
    let _ = child.wait();
    assert!(log.contains("ready"), "{log}");
    assert!(
        log.contains("WARNING") && log.contains("--insecure-bind-any"),
        "{log}"
    );
}
