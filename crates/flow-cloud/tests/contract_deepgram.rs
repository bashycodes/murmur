//! deepgram adapter wire contract against a mock HTTP server.

mod common;

use common::*;
use flow_cloud::testing::{MockServer, Reply};

#[test]
fn request_shape_query_terms_and_response() {
    let srv = MockServer::fixed(Reply::json(200, fixture("deepgram/success.json")));
    let a = asr(provider("deepgram", &srv, ""));
    let text = run(
        &a,
        &req(&["Kubernetes", "Wispr Flow"], &["Alice"], Some("en")),
    )
    .unwrap();
    assert_eq!(text, "Ask not what your country can do for you.");
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/v1/listen");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Token {KEY}").as_str())
    );
    assert_eq!(r.header("content-type"), Some("audio/flac"));
    assert_eq!(&r.body[..4], b"fLaC");
    assert_eq!(r.query_all("model"), ["nova-3"]);
    assert_eq!(r.query_all("language"), ["en"]);
    assert_eq!(r.query_all("keyterm"), ["Kubernetes", "Wispr Flow"]);
    assert_eq!(r.query_all("mip_opt_out"), ["true"]);
    assert_eq!(r.query_all("punctuate"), ["true"]);
    assert!(r.query_all("detect_language").is_empty());
    assert_no_key_in(r, "authorization");
    let info = a.last_call().unwrap();
    assert_eq!(info.billed_audio_ms, 3000);
    assert_eq!(
        info.request_id.as_deref(),
        Some("5d8e3a1c-0000-4000-8000-123456789abc")
    );
}

#[test]
fn auto_language_and_no_opt_out() {
    let srv = MockServer::fixed(Reply::json(200, fixture("deepgram/success.json")));
    let mut p = provider("deepgram", &srv, "");
    p.zero_retention = false;
    let a = asr(p);
    use flow_core::asr::AsrEngine;
    let out = a.transcribe(&speech(1000), &req(&[], &[], None)).unwrap();
    assert_eq!(out.language.as_deref(), Some("en"), "detected language");
    let r = &srv.requests()[0];
    assert_eq!(r.query_all("detect_language"), ["true"]);
    assert!(r.query_all("mip_opt_out").is_empty());
    assert!(r.query_all("keyterm").is_empty());
}

#[test]
fn keyterms_are_capped_at_100() {
    let srv = MockServer::fixed(Reply::json(200, fixture("deepgram/success.json")));
    let a = asr(provider("deepgram", &srv, ""));
    let many: Vec<String> = (0..150).map(|i| format!("term{i}")).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    run(&a, &req(&refs, &[], None)).unwrap();
    let k = srv.requests()[0].query_all("keyterm");
    assert_eq!(k.len(), 100);
    assert_eq!(k[0], "term0", "dictionary order kept");
}

#[test]
fn real_error_body() {
    let srv = MockServer::fixed(Reply::json(401, fixture("deepgram/error_auth.json")));
    let e = run(&asr(provider("deepgram", &srv, "")), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::AuthFailed);
}

#[test]
fn error_mapping() {
    check_error_mapping("deepgram", "", "/v1/listen");
}

#[test]
fn transport_errors() {
    check_transport_errors("deepgram", "");
}
