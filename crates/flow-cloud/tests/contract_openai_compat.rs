//! openai_compat adapter wire contract (Groq / OpenAI / speaches / custom)
//! against a mock HTTP server.

mod common;

use common::*;
use flow_cloud::testing::{MockServer, Reply};

#[test]
fn whisper_dialect_request_shape_and_response() {
    let srv = MockServer::fixed(Reply::json(200, fixture("openai_compat/success.json")));
    let a = asr(provider("groq", &srv, "/openai/v1"));
    let text = run(&a, &req(&["Kubernetes", "Wispr"], &["Alice"], Some("en"))).unwrap();
    assert_eq!(text, "Ask not what your country can do for you.");
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/openai/v1/audio/transcriptions");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert!(r
        .header("content-type")
        .unwrap()
        .starts_with("multipart/form-data"));
    assert_eq!(r.field("model").as_deref(), Some("whisper-large-v3-turbo"));
    assert_eq!(r.field("language").as_deref(), Some("en"));
    assert_eq!(r.field("response_format").as_deref(), Some("json"));
    // Dictionary only: screen words are off by default.
    assert_eq!(
        r.field("prompt").as_deref(),
        Some("Vocabulary: Kubernetes, Wispr.")
    );
    let file = r.parts().into_iter().find(|p| p.name == "file").unwrap();
    assert_eq!(file.filename.as_deref(), Some("audio.flac"));
    assert_eq!(file.content_type.as_deref(), Some("audio/flac"));
    assert_eq!(&file.data[..4], b"fLaC");
    assert_no_key_in(r, "authorization");
    let info = a.last_call().unwrap();
    assert_eq!(
        info.billed_audio_ms, 3000,
        "usage.seconds is the billed audio"
    );
    assert!(info.cost_estimated, "Groq reports no cost: estimate");
    let est = info.cost_usd.unwrap();
    assert!((est - 0.04 * 3.0 / 3600.0).abs() < 1e-12, "{est}");
}

#[test]
fn gpt_transcribe_dialect_sends_keywords_and_languages() {
    let srv = MockServer::fixed(Reply::json(200, fixture("openai_compat/success.json")));
    let mut p = provider("openai", &srv, "/v1");
    p.send_screen_words = true;
    let a = asr(p);
    run(
        &a,
        &req(&["Kubernetes", "bad\nterm<x>"], &["Alice"], Some("de")),
    )
    .unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.field("model").as_deref(), Some("gpt-transcribe"));
    assert_eq!(r.fields("keywords[]"), ["Kubernetes", "bad termx", "Alice"]);
    assert_eq!(r.fields("languages[]"), ["de"]);
    assert!(r.field("prompt").is_none() && r.field("language").is_none());
}

#[test]
fn no_terms_no_language_sends_neither() {
    let srv = MockServer::fixed(Reply::json(200, fixture("openai_compat/success.json")));
    let mut p = provider("groq", &srv, "/openai/v1");
    p.send_dictionary = false;
    run(&asr(p), &req(&["Kubernetes"], &[], None)).unwrap();
    let r = &srv.requests()[0];
    assert!(r.field("prompt").is_none());
    assert!(r.field("language").is_none());
}

#[test]
fn provider_language_overrides_the_session() {
    let srv = MockServer::fixed(Reply::json(200, fixture("openai_compat/success.json")));
    let mut p = provider("groq", &srv, "/openai/v1");
    p.language = "fr".into();
    run(&asr(p), &req(&[], &[], Some("en"))).unwrap();
    assert_eq!(srv.requests()[0].field("language").as_deref(), Some("fr"));
}

#[test]
fn wav_override_and_empty_key_for_self_hosted() {
    let srv = MockServer::fixed(Reply::json(200, fixture("openai_compat/success.json")));
    let mut p = provider("custom", &srv, "/v1");
    p.model = "Systran/faster-whisper-small".into();
    p.format_override = "wav".into();
    let a = flow_cloud::CloudAsr::new(p, flow_router::SecretString::new(""));
    run(&a, &req(&[], &[], None)).unwrap();
    let r = &srv.requests()[0];
    assert!(
        r.header("authorization").is_none(),
        "no auth header without a key"
    );
    let file = r.parts().into_iter().find(|p| p.name == "file").unwrap();
    assert_eq!(file.filename.as_deref(), Some("audio.wav"));
    assert_eq!(&file.data[..4], b"RIFF");
}

#[test]
fn empty_transcript_is_a_final_empty_answer() {
    let srv = MockServer::fixed(Reply::json(200, fixture("openai_compat/empty.json")));
    let a = asr(provider("groq", &srv, "/openai/v1"));
    assert_eq!(run(&a, &req(&[], &[], None)).unwrap(), "");
    assert_eq!(srv.count(), 1);
}

#[test]
fn real_error_bodies_map_and_never_leak_text() {
    let srv = MockServer::fixed(Reply::json(429, fixture("openai_compat/error_quota.json")));
    let e = run(&asr(provider("openai", &srv, "/v1")), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::QuotaExceeded);
    let srv = MockServer::fixed(Reply::json(401, fixture("openai_compat/error_auth.json")));
    let e = run(&asr(provider("openai", &srv, "/v1")), &req(&[], &[], None)).unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::AuthFailed);
    assert!(!e.message.contains("Incorrect"), "{e}");
}

#[test]
fn error_mapping() {
    check_error_mapping("groq", "/openai/v1", "/audio/transcriptions");
}

#[test]
fn transport_errors() {
    check_transport_errors("groq", "/openai/v1");
}

#[test]
fn partial_requests_are_refused_without_a_request() {
    use flow_core::asr::{AsrEngine, AsrPurpose};
    let srv = MockServer::fixed(Reply::json(200, fixture("openai_compat/success.json")));
    let a = asr(provider("groq", &srv, "/openai/v1"));
    let mut r = req(&[], &[], None);
    r.purpose = AsrPurpose::Partial;
    assert!(a.transcribe(&speech(1000), &r).is_err());
    assert_eq!(srv.count(), 0);
}

#[test]
fn an_echoed_key_is_dropped_from_the_error_code() {
    let srv = MockServer::fixed(Reply::json(
        422,
        serde_json::json!({"error": {"code": KEY}}),
    ));
    let e = run(
        &asr(provider("groq", &srv, "/openai/v1")),
        &req(&[], &[], None),
    )
    .unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::ProviderError);
    assert!(!e.message.contains("LEAKCHECK"), "{e}");
    assert!(e.message.contains("HTTP 422"));
}
