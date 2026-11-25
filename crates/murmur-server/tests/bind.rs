//! D111: plain `ws://` is served on any bind, wildcard and public ones
//! included, without `[tls]` and without a flag. `--insecure-bind-any` is
//! still accepted (a no-op) so existing command lines keep working.

mod common;

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::*;

#[tokio::test(flavor = "multi_thread")]
async fn serve_accepts_any_bind_without_tls() {
    for bind in [
        "0.0.0.0:7313",
        "[::]:7313",
        "203.0.113.5:7313",
        "[2001:db8::1]:7313",
        "192.168.1.5:7313",
        "100.101.102.103:7313",
        "[::1]:7313",
    ] {
        let mut c = mock_cfg("x");
        c.bind = vec!["127.0.0.1:7313".into(), bind.into()];
        assert!(c.tls.is_none());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let r = murmur_server::serve(c, vec![l]).await;
        assert!(r.is_ok(), "{bind}: {:?}", r.err());
    }
}

/// A plain `ws://` client completes the handshake against a server whose
/// configured bind is a wildcard, with no TLS and no flag.
#[tokio::test(flavor = "multi_thread")]
async fn a_wildcard_bind_serves_plain_ws() {
    let mut c = mock_cfg("hello");
    c.bind = vec!["0.0.0.0:7313".into()];
    let h = start(c).await;
    let ws = connect(h.addr(), Some(TOKEN)).await;
    assert!(ws.is_ok(), "{:?}", ws.err());
}

fn write_cfg(dir: &std::path::Path, bind: &str) -> std::path::PathBuf {
    let token = dir.join("token");
    std::fs::write(&token, format!("{TOKEN}\n")).unwrap();
    let p = dir.join("config.toml");
    std::fs::write(
        &p,
        format!(
            "bind = [\"{bind}\"]\ntoken_file = {token:?}\nstate_dir = {dir:?}\n[model]\nasr_engine = {{ mock = {{ text = \"x\" }} }}\n"
        ),
    )
    .unwrap();
    p
}

#[test]
fn check_config_accepts_0_0_0_0_without_tls() {
    let d = tempfile::tempdir().unwrap();
    let cfg = write_cfg(d.path(), "0.0.0.0:0");
    let out = Command::new(env!("CARGO_BIN_EXE_murmur-server"))
        .arg("check-config")
        .arg("--config")
        .arg(&cfg)
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The binary serves 0.0.0.0 without TLS, with and without the old flag.
#[test]
fn the_binary_serves_0_0_0_0_with_or_without_the_old_flag() {
    for flag in [&[][..], &["--insecure-bind-any"][..]] {
        let d = tempfile::tempdir().unwrap();
        let cfg = write_cfg(d.path(), "0.0.0.0:0");
        let mut child = Command::new(env!("CARGO_BIN_EXE_murmur-server"))
            .arg("--config")
            .arg(&cfg)
            .args(flag)
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
        assert!(log.contains("ready"), "{flag:?}: {log}");
        assert!(log.contains("plain ws://"), "{flag:?}: {log}");
    }
}
