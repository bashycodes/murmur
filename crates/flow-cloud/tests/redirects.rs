//! Review S3: provider calls never follow a redirect. A 3xx answer is a
//! `provider_error`, and nothing (key header, audio) reaches the target.

mod common;

use common::*;
use flow_cloud::testing::{MockServer, Reply};
use flow_router::Failure;

#[test]
fn no_adapter_follows_a_redirect() {
    let cases = [
        ("groq", "/openai/v1"),
        ("openrouter", "/api/v1"),
        ("elevenlabs", ""),
        ("deepgram", ""),
        ("assemblyai", ""),
    ];
    for status in [302u16, 307, 308] {
        for (preset, base) in cases {
            let target = MockServer::fixed(Reply::json(200, serde_json::json!({"text": "x"})));
            let loc = target.url("/steal");
            let srv = MockServer::fixed(Reply::raw(status, "").header("location", &loc));
            let e = run(
                &asr(provider(preset, &srv, base)),
                &req(&["term"], &[], None),
            )
            .unwrap_err();
            assert_eq!(e.kind, Failure::ProviderError, "{preset} {status}: {e}");
            assert_eq!(srv.count(), 1, "{preset} {status}: not retried");
            assert_eq!(
                target.count(),
                0,
                "{preset} {status}: redirect followed (key/audio leaked)"
            );
        }
    }
}

/// Review S6 for http:// providers: a name counts by what it resolves to.
#[test]
fn plain_http_names_are_checked_by_resolved_address() {
    let srv = MockServer::fixed(Reply::json(200, serde_json::json!({"text": "ok"})));
    let mut p = provider("groq", &srv, "/openai/v1");
    // Loopback by name: allowed.
    p.base_url = p.base_url.replace("127.0.0.1", "localhost");
    assert_eq!(run(&asr(p.clone()), &req(&[], &[], None)).unwrap(), "ok");
    // A single-label name that does not resolve: refused before any request.
    p.base_url = p.base_url.replace("localhost", "flow-no-such-host-zz9");
    assert!(p.validate().is_ok(), "names pass the config-time check");
    let e = run(&asr(p), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, Failure::ConnectFailed, "{e}");
    assert_eq!(srv.count(), 1, "only the localhost call reached a server");
}

/// The http:// check uses the resolved addresses, and the request goes to
/// exactly those (no second lookup a rebinding DNS could answer).
#[test]
fn plain_http_requests_are_pinned_to_checked_private_addresses() {
    let srv = MockServer::fixed(Reply::json(200, serde_json::json!({"text": "ok"})));
    let mut p = provider("groq", &srv, "/openai/v1");
    let port: u16 = p
        .base_url
        .rsplit(':')
        .next()
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    p.base_url = format!("http://box.local:{port}/openai/v1");
    let to = |ip: &'static str| {
        move |_h: &str, port: u16| -> std::io::Result<Vec<std::net::SocketAddr>> {
            Ok(vec![std::net::SocketAddr::new(ip.parse().unwrap(), port)])
        }
    };
    let private = asr(p.clone()).with_resolver(to("127.0.0.1"));
    assert_eq!(run(&private, &req(&[], &[], None)).unwrap(), "ok", "pinned");
    let public = asr(p).with_resolver(to("8.8.8.8"));
    let e = run(&public, &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, Failure::ConnectFailed, "{e}");
    assert_eq!(srv.count(), 1, "nothing sent towards the public address");
}
