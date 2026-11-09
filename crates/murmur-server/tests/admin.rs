//! Admin API (D39) against a real in-process flow-server: whoami, list /
//! create / revoke, 401 / 403, the last-admin guard, revoke = next
//! request, `auth = "none"`, and the CLI writing the same keys file.

mod common;

use std::net::SocketAddr;
use std::path::Path;

use common::*;
use flow_protocol::admin::{code, AdminError, CreatedKey, KeyList, RevokedKey, WhoAmI};
use flow_server::keys::{KeyStore, KEYS_FILE, OWNER};
use flow_server::Config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let body = body.unwrap_or("");
    s.write_all(
        format!(
            "{method} {path} HTTP/1.1\r\nHost: x\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let status = out[9..12].parse().unwrap();
    let head = out
        .split("\r\n\r\n")
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if (200..300).contains(&status) {
        assert!(head.contains("cache-control: no-store"), "{head}");
    }
    let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

fn err(body: &str) -> String {
    serde_json::from_str::<AdminError>(body)
        .unwrap_or_else(|e| panic!("{e}: {body}"))
        .code
}

async fn create(addr: SocketAddr, token: &str, name: &str, admin: bool) -> CreatedKey {
    let (st, body) = http(
        addr,
        "POST",
        "/v1/admin/keys",
        Some(token),
        Some(&format!(r#"{{"name":"{name}","admin":{admin}}}"#)),
    )
    .await;
    assert_eq!(st, 201, "{body}");
    serde_json::from_str(&body).unwrap()
}

async fn list(addr: SocketAddr, token: &str) -> KeyList {
    let (st, body) = http(addr, "GET", "/v1/admin/keys", Some(token), None).await;
    assert_eq!(st, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn file_cfg(dir: &Path) -> Config {
    let mut c = mock_cfg("hello");
    c.state_dir = Some(dir.join("state"));
    c
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_lists_creates_and_revokes() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    let (st, body) = http(a, "GET", "/v1/whoami", Some(TOKEN), None).await;
    assert_eq!(st, 200);
    let me: WhoAmI = serde_json::from_str(&body).unwrap();
    assert_eq!((me.name.as_str(), me.admin), (OWNER, true));

    let phone = create(a, TOKEN, "phone", false).await;
    assert_eq!(phone.secret.len(), 43);
    assert!(!phone.key.admin && !phone.key.current && phone.key.active());
    // The new key works at once, for the stream too.
    assert!(connect(a, Some(&phone.secret)).await.is_ok());

    let l = list(a, TOKEN).await;
    assert_eq!(l.keys.len(), 2);
    assert!(l.keys[0].current && l.keys[0].name == OWNER);
    // Connecting and listing keys touch "last connected", not "last used";
    // `last_seen` mirrors last connected for clients older than the split.
    for k in &l.keys {
        assert!(k.last_connected.is_some(), "{k:?}");
        assert_eq!(k.last_seen, k.last_connected, "{k:?}");
        assert_eq!(k.last_used, None, "{k:?}");
    }
    assert!(!body_has_secret(
        &serde_json::to_string(&l).unwrap(),
        &phone.secret
    ));

    let (st, body) = http(
        a,
        "DELETE",
        &format!("/v1/admin/keys/{}", phone.key.id),
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!(st, 200, "{body}");
    let r: RevokedKey = serde_json::from_str(&body).unwrap();
    assert!(r.key.revoked.is_some());
    // Revoke = next connection.
    assert!(connect(a, Some(&phone.secret)).await.is_err());
    let (st, body) = http(a, "GET", "/v1/whoami", Some(&phone.secret), None).await;
    assert_eq!((st, err(&body).as_str()), (401, code::UNAUTHORIZED));
    // Gone already, unknown id.
    let (st, body) = http(
        a,
        "DELETE",
        &format!("/v1/admin/keys/{}", phone.key.id),
        Some(TOKEN),
        None,
    )
    .await;
    assert_eq!((st, err(&body).as_str()), (404, code::NOT_FOUND));
    // Revoked keys stay listed.
    assert_eq!(list(a, TOKEN).await.keys.len(), 2);
    s.kill();
}

fn body_has_secret(body: &str, secret: &str) -> bool {
    body.contains(secret)
}

#[tokio::test(flavor = "multi_thread")]
async fn non_admin_keys_get_403_and_bad_requests_400() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    let phone = create(a, TOKEN, "phone", false).await;
    let p = Some(phone.secret.as_str());
    let (st, body) = http(a, "GET", "/v1/whoami", p, None).await;
    assert_eq!(st, 200);
    assert!(!serde_json::from_str::<WhoAmI>(&body).unwrap().admin);
    for (m, path, b) in [
        ("GET", "/v1/admin/keys".to_string(), None),
        (
            "POST",
            "/v1/admin/keys".to_string(),
            Some(r#"{"name":"x"}"#),
        ),
        ("DELETE", format!("/v1/admin/keys/{}", phone.key.id), None),
    ] {
        let (st, body) = http(a, m, &path, p, b).await;
        assert_eq!(
            (st, err(&body).as_str()),
            (403, code::FORBIDDEN),
            "{m} {path}"
        );
    }
    let (st, _) = http(a, "GET", "/v1/admin/keys", None, None).await;
    assert_eq!(st, 401);
    let (st, _) = http(
        a,
        "GET",
        "/v1/admin/keys",
        Some("wrong-token-0123456789"),
        None,
    )
    .await;
    assert_eq!(st, 401);
    // Nothing changed for the 403s.
    assert_eq!(list(a, TOKEN).await.keys.len(), 2);

    let t = Some(TOKEN);
    let (st, body) = http(a, "POST", "/v1/admin/keys", t, Some("not json")).await;
    assert_eq!((st, err(&body).as_str()), (400, code::BAD_REQUEST));
    let (st, body) = http(
        a,
        "POST",
        "/v1/admin/keys",
        t,
        Some(r#"{"name":"bad name"}"#),
    )
    .await;
    assert_eq!((st, err(&body).as_str()), (400, code::BAD_REQUEST));
    let (st, body) = http(a, "POST", "/v1/admin/keys", t, Some(r#"{"name":"phone"}"#)).await;
    assert_eq!((st, err(&body).as_str()), (409, code::NAME_TAKEN));
    let big = format!(r#"{{"name":"{}"}}"#, "a".repeat(8000));
    let (st, _) = http(a, "POST", "/v1/admin/keys", t, Some(&big)).await;
    assert_eq!(st, 413);
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn last_admin_key_cannot_be_revoked() {
    let s = start(mock_cfg("hello")).await;
    let a = s.addr();
    let owner = list(a, TOKEN).await.keys[0].id.clone();
    let del = |id: String, tok: String| async move {
        http(
            a,
            "DELETE",
            &format!("/v1/admin/keys/{id}"),
            Some(&tok),
            None,
        )
        .await
    };
    let (st, body) = del(owner.clone(), TOKEN.into()).await;
    assert_eq!((st, err(&body).as_str()), (409, code::LAST_ADMIN));
    // With a second admin, revoking the key in use is allowed...
    let laptop = create(a, TOKEN, "laptop", true).await;
    let (st, _) = del(owner.clone(), TOKEN.into()).await;
    assert_eq!(st, 200);
    assert_eq!(http(a, "GET", "/v1/whoami", Some(TOKEN), None).await.0, 401);
    // ...and now the laptop is the last one.
    let (st, body) = del(laptop.key.id.clone(), laptop.secret.clone()).await;
    assert_eq!((st, err(&body).as_str()), (409, code::LAST_ADMIN));
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_off_has_no_admin_api() {
    let mut c = mock_cfg("hello");
    c.token = None;
    c.auth = flow_server::config::AuthMode::None;
    let s = start(c).await;
    for p in ["/v1/whoami", "/v1/admin/keys"] {
        let (st, body) = http(s.addr(), "GET", p, None, None).await;
        assert_eq!((st, err(&body).as_str()), (404, code::AUTH_OFF), "{p}");
    }
    s.kill();
}

#[tokio::test(flavor = "multi_thread")]
async fn server_and_cli_write_the_same_keys_file() {
    let d = tempfile::tempdir().unwrap();
    let s = start(file_cfg(d.path())).await;
    let a = s.addr();
    let cli = || KeyStore::open(&d.path().join("state"), None).unwrap();
    // Interleave server (API) and CLI writers; every key survives.
    for i in 0..5 {
        create(a, TOKEN, &format!("api-{i}"), false).await;
        cli().add(&format!("cli-{i}"), false).unwrap();
    }
    let names: Vec<String> = list(a, TOKEN)
        .await
        .keys
        .into_iter()
        .map(|k| k.name)
        .collect();
    assert_eq!(names.len(), 11, "{names:?}");
    // The CLI sees the API's keys and the API the CLI's.
    assert_eq!(cli().active(), 11);
    let created = create(a, TOKEN, "tablet", false).await;
    let body = std::fs::read_to_string(d.path().join("state").join(KEYS_FILE)).unwrap();
    assert!(!body.contains(&created.secret), "only the hash is stored");
    cli().revoke("tablet", false).unwrap();
    assert!(connect(a, Some(&created.secret)).await.is_err());
    s.kill();
}
