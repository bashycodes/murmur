//! Bearer-token auth on the upgrade and on /v1/health.

mod common;

use common::*;
use tokio_tungstenite::tungstenite::Error;

fn status(e: Error) -> u16 {
    match e {
        Error::Http(r) => r.status().as_u16(),
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_or_wrong_token_is_401_before_upgrade() {
    let s = start(mock_cfg("x")).await;
    assert_eq!(status(connect(s.addr(), None).await.unwrap_err()), 401);
    assert_eq!(
        status(
            connect(s.addr(), Some("wrong-token-0123456789"))
                .await
                .unwrap_err()
        ),
        401
    );
    // A prefix of the right token is still wrong (length checked).
    assert_eq!(
        status(connect(s.addr(), Some(&TOKEN[..20])).await.unwrap_err()),
        401
    );
    assert!(connect(s.addr(), Some(TOKEN)).await.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn health_needs_the_token_too() {
    let s = start(mock_cfg("x")).await;
    assert_eq!(http_get(s.addr(), "/v1/health", None).await.0, 401);
    assert_eq!(
        http_get(s.addr(), "/v1/health", Some("nope-nope-nope-nope"))
            .await
            .0,
        401
    );
    assert_eq!(http_get(s.addr(), "/v1/health", Some(TOKEN)).await.0, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn auth_none_only_on_loopback() {
    let mut c = mock_cfg("x");
    c.token = None;
    c.auth = murmur_server::config::AuthMode::None;
    let s = start(c.clone()).await;
    assert!(connect(s.addr(), None).await.is_ok());
    c.bind = vec!["0.0.0.0:7313".into()];
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let e = murmur_server::serve(c, vec![l]).await.err().unwrap();
    assert!(e.contains("loopback"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_start_without_a_token() {
    // No token and no keys file in the state dir (never the real one).
    let d = tempfile::tempdir().unwrap();
    let mut c = mock_cfg("x");
    c.token = None;
    c.state_dir = Some(d.path().into());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(murmur_server::serve(c, vec![l]).await.is_err());
}

/// Raw `GET` (status, `Retry-After`, body).
async fn get_raw(
    addr: std::net::SocketAddr,
    path: &str,
    token: &str,
) -> (u16, Option<u64>, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    let retry = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case("retry-after")
            .then(|| v.trim().parse().unwrap())
    });
    (out[9..12].parse().unwrap(), retry, body.to_string())
}

/// Throttling on, for the loopback test client (D112).
fn throttled_cfg(max_failures: u32) -> murmur_server::Config {
    let mut c = mock_cfg("x");
    c.auth_throttle.max_failures = max_failures;
    c.auth_throttle.exempt_loopback = false;
    c
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_failures_lock_the_peer_out_of_every_route_with_429() {
    let s = start(throttled_cfg(3)).await;
    let a = s.addr();
    // Wrong keys on different routes all count: the third failure is still
    // a 401, but it locks the address.
    assert_eq!(status(connect(a, Some("wrong-0")).await.unwrap_err()), 401);
    assert_eq!(get_raw(a, "/v1/health", "wrong-1").await.0, 401);
    assert_eq!(get_raw(a, "/v1/whoami", "wrong-2").await.0, 401);
    // Now even the right key is refused, before it is compared.
    assert_eq!(status(connect(a, Some(TOKEN)).await.unwrap_err()), 429);
    for path in ["/v1/health", "/v1/models", "/v1/models/m/files/f.bin"] {
        let (st, retry, _) = get_raw(a, path, TOKEN).await;
        assert_eq!(st, 429, "{path}");
        assert!(matches!(retry, Some(1..=30)), "{path}: {retry:?}");
    }
    let (st, retry, body) = get_raw(a, "/v1/whoami", TOKEN).await;
    assert_eq!((st, retry.is_some()), (429, true));
    let e: murmur_protocol::admin::AdminError = serde_json::from_str(&body).unwrap();
    assert_eq!(e.code, murmur_protocol::admin::code::THROTTLED);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_success_clears_the_failure_count() {
    let s = start(throttled_cfg(3)).await;
    let a = s.addr();
    for _ in 0..5 {
        assert_eq!(get_raw(a, "/v1/health", "wrong").await.0, 401);
        assert_eq!(get_raw(a, "/v1/health", "wrong").await.0, 401);
        assert_eq!(get_raw(a, "/v1/health", TOKEN).await.0, 200);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn loopback_is_never_locked_out_by_default() {
    let s = start(mock_cfg("x")).await;
    for _ in 0..25 {
        assert_eq!(http_get(s.addr(), "/v1/health", Some("wrong")).await.0, 401);
    }
    assert_eq!(http_get(s.addr(), "/v1/health", Some(TOKEN)).await.0, 200);
    assert!(connect(s.addr(), Some(TOKEN)).await.is_ok());
}
