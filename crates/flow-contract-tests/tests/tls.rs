//! wss:// with a self-signed, pinned certificate (LAN without Tailscale).

use flow_contract_tests::*;
use flow_core::config::SessionConfig;
use flow_router::{Router, RouterConfig};
use sha2::{Digest, Sha256};

struct Tls {
    _dir: tempfile::TempDir,
    server: TestServer,
    pin: String,
}

fn tls_server() -> Tls {
    let ck =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    std::fs::write(&cert, ck.cert.pem()).unwrap();
    std::fs::write(&key, ck.key_pair.serialize_pem()).unwrap();
    let pin = Sha256::digest(ck.cert.der().as_ref())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut cfg = server_cfg(mock_model("over tls"));
    cfg.tls = Some(flow_server::config::TlsConfig { cert, key });
    Tls {
        _dir: dir,
        server: spawn_server(cfg),
        pin,
    }
}

fn router(t: &Tls, pin: Option<String>) -> Router {
    let mut c: RouterConfig = router_config(t.server.addr(), true, false);
    c.server.tls = true;
    c.server.tls_pin_sha256 = pin;
    hybrid_router(c, None)
}

fn dictate(r: &Router) -> flow_core::CommitResult {
    let _ = r.start_session(SessionConfig::default());
    for c in speech(1000).chunks(FRAME) {
        r.push_audio(c);
    }
    r.commit()
}

#[test]
fn pinned_self_signed_certificate_works() {
    let t = tls_server();
    let colons = t
        .pin
        .as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).unwrap().to_uppercase())
        .collect::<Vec<_>>()
        .join(":");
    let r = dictate(&router(&t, Some(colons)));
    assert_eq!(r.route.as_ref().unwrap().engine, "remote", "{r:?}");
    assert!(r.final_text.contains("over tls"));
}

#[test]
fn wrong_pin_or_no_pin_is_refused() {
    let t = tls_server();
    let r = dictate(&router(&t, Some("00".repeat(32))));
    assert_eq!(
        r.route.as_ref().unwrap().fallback.as_deref(),
        Some("connect_failed")
    );
    // Without a pin the self-signed cert fails the Mozilla roots.
    let rt = router(&t, None);
    let h = rt.probe_server();
    assert_eq!(h.error.as_deref(), Some("connect_failed"));
    assert!(h.message.unwrap().contains("TLS"));
}

#[test]
fn plaintext_client_cannot_talk_to_a_tls_server() {
    let t = tls_server();
    let r = hybrid_router(router_config(t.server.addr(), true, false), None);
    assert!(!r.probe_server().ok);
}
