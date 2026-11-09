//! D39: murmur-remote's admin client (what Windows and, through murmur-ffi,
//! Android use) against a real murmur-server, over ws-style plain HTTP and
//! over TLS with a pinned self-signed certificate.

use murmur_contract_tests::*;
use murmur_protocol::admin::code;
use murmur_remote::admin::AdminClient;
use sha2::{Digest, Sha256};

fn client(cfg: murmur_router::ServerConfig) -> AdminClient {
    AdminClient::new(cfg, "contract-tests")
}

fn round_trip(c: &AdminClient, mk: impl Fn(&str) -> AdminClient) {
    let me = c.whoami().unwrap();
    assert!(me.admin);
    let created = c.create_key("phone", false).unwrap();
    assert_eq!(created.secret.len(), 43);
    let l = c.list_keys().unwrap();
    assert_eq!(l.keys.len(), 2);
    assert!(l.keys.iter().any(|k| k.current && k.id == me.id));
    let phone = mk(&created.secret);
    assert!(!phone.whoami().unwrap().admin);
    assert_eq!(phone.list_keys().unwrap_err().code, code::FORBIDDEN);
    assert_eq!(
        c.create_key("phone", false).unwrap_err().code,
        code::NAME_TAKEN
    );
    assert_eq!(c.revoke_key(&me.id).unwrap_err().code, code::LAST_ADMIN);
    assert!(c.revoke_key(&created.key.id).unwrap().key.revoked.is_some());
    assert_eq!(phone.whoami().unwrap_err().code, code::UNAUTHORIZED);
    assert_eq!(
        c.revoke_key("k-0000000000000000").unwrap_err().code,
        code::NOT_FOUND
    );
}

#[test]
fn admin_client_plain() {
    let s = spawn_mock_server("x");
    let mk = |t: &str| client(server_config(s.addr(), t));
    round_trip(&mk(TOKEN), mk);
    s.kill();
    assert_eq!(mk(TOKEN).whoami().unwrap_err().code, code::CONNECT_FAILED);
}

#[test]
fn admin_client_over_pinned_tls() {
    let ck =
        rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    std::fs::write(&cert, ck.cert.pem()).unwrap();
    std::fs::write(&key, ck.key_pair.serialize_pem()).unwrap();
    let pin: String = Sha256::digest(ck.cert.der().as_ref())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut cfg = server_cfg(mock_model("x"));
    cfg.tls = Some(murmur_server::config::TlsConfig { cert, key });
    let s = spawn_server(cfg);
    let mk = |t: &str| {
        let mut c = server_config(s.addr(), t);
        c.tls = true;
        c.tls_pin_sha256 = Some(pin.clone());
        client(c)
    };
    round_trip(&mk(TOKEN), mk);
    // A wrong pin never reaches the API.
    let mut c = server_config(s.addr(), TOKEN);
    c.tls = true;
    c.tls_pin_sha256 = Some("00".repeat(32));
    assert_eq!(client(c).whoami().unwrap_err().code, code::CONNECT_FAILED);
}
