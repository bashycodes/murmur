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
    c.auth = flow_server::config::AuthMode::None;
    let s = start(c.clone()).await;
    assert!(connect(s.addr(), None).await.is_ok());
    c.bind = vec!["0.0.0.0:7313".into()];
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let e = flow_server::serve(c, vec![l]).await.err().unwrap();
    assert!(e.contains("loopback"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_start_without_a_token() {
    let mut c = mock_cfg("x");
    c.token = None;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(flow_server::serve(c, vec![l]).await.is_err());
}
