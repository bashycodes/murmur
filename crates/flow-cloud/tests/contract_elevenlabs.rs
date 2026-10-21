//! elevenlabs adapter wire contract against a mock HTTP server.

mod common;

use common::*;
use flow_cloud::testing::{MockServer, Reply};

#[test]
fn request_shape_raw_pcm_and_keyterms() {
    let srv = MockServer::fixed(Reply::json(200, fixture("elevenlabs/success.json")));
    let a = asr(provider("elevenlabs", &srv, ""));
    let long = "a term that is far too long to be accepted by scribe ok";
    let six = "one two three four five six";
    let text = run(
        &a,
        &req(
            &["Kubernetes", long, six, "Wispr Flow"],
            &["Alice"],
            Some("en"),
        ),
    )
    .unwrap();
    assert_eq!(text, "Ask not what your country can do for you.");
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/v1/speech-to-text");
    assert_eq!(r.header("xi-api-key"), Some(KEY));
    assert!(r.header("authorization").is_none());
    assert_eq!(r.field("model_id").as_deref(), Some("scribe_v2"));
    assert_eq!(r.field("language_code").as_deref(), Some("en"));
    assert_eq!(r.field("file_format").as_deref(), Some("pcm_s16le_16"));
    // ≥ 50 chars and > 5 words are dropped; screen words off by default.
    assert_eq!(r.fields("keyterms"), ["Kubernetes", "Wispr Flow"]);
    // Zero retention is enterprise-only: off in the preset.
    assert!(r.field("enable_logging").is_none());
    let file = r.parts().into_iter().find(|p| p.name == "file").unwrap();
    assert_eq!(
        file.data.len(),
        16_000 * 2,
        "1 s of raw s16le, no container"
    );
    assert_no_key_in(r, "xi-api-key");
    let info = a.last_call().unwrap();
    assert_eq!(info.request_id.as_deref(), Some("tr_0123abcd"));
    assert!(info.cost_estimated);
    assert_eq!(
        info.billed_audio_ms, 1000,
        "no reported duration: uploaded audio"
    );
}

#[test]
fn zero_retention_and_no_terms() {
    let srv = MockServer::fixed(Reply::json(200, fixture("elevenlabs/success.json")));
    let mut p = provider("elevenlabs", &srv, "");
    p.zero_retention = true;
    p.format_override = "flac".into();
    run(&asr(p), &req(&[], &[], None)).unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.field("enable_logging").as_deref(), Some("false"));
    assert!(r.fields("keyterms").is_empty());
    assert!(r.field("language_code").is_none());
    assert!(r.field("file_format").is_none());
    let file = r.parts().into_iter().find(|p| p.name == "file").unwrap();
    assert_eq!(&file.data[..4], b"fLaC");
}

#[test]
fn real_error_bodies() {
    let srv = MockServer::fixed(Reply::json(401, fixture("elevenlabs/error_auth.json")));
    let e = run(&asr(provider("elevenlabs", &srv, "")), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::AuthFailed);
    let srv = MockServer::fixed(Reply::json(401, fixture("elevenlabs/error_quota.json")));
    let e = run(&asr(provider("elevenlabs", &srv, "")), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::QuotaExceeded);
}

#[test]
fn error_mapping() {
    check_error_mapping("elevenlabs", "", "/v1/speech-to-text");
}

#[test]
fn transport_errors() {
    check_transport_errors("elevenlabs", "");
}
