//! openrouter adapter wire contract against a mock HTTP server.

mod common;

use base64::Engine;
use common::*;
use flow_cloud::testing::{MockServer, Reply};
use flow_core::asr::AsrEngine;

#[test]
fn request_shape_and_reported_cost() {
    let srv = MockServer::fixed(
        Reply::json(200, fixture("openrouter/success.json"))
            .header("X-Generation-Id", "gen-abc123"),
    );
    let a = asr(provider("openrouter", &srv, "/api/v1"));
    let text = run(&a, &req(&["Kubernetes"], &["Alice"], Some("en"))).unwrap();
    assert_eq!(text, "Ask not what your country can do for you.");
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/api/v1/audio/transcriptions");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert!(r
        .header("content-type")
        .unwrap()
        .starts_with("application/json"));
    let j = r.json();
    assert_eq!(j["model"], "openai/gpt-transcribe");
    assert_eq!(j["language"], "en");
    assert_eq!(j["response_format"], "json");
    assert_eq!(j["input_audio"]["format"], "flac");
    let audio = base64::engine::general_purpose::STANDARD
        .decode(j["input_audio"]["data"].as_str().unwrap())
        .unwrap();
    assert_eq!(&audio[..4], b"fLaC");
    // Zero retention on by default for this preset.
    assert_eq!(j["provider"]["zdr"], true);
    assert_eq!(j["provider"]["data_collection"], "deny");
    // OpenRouter ignores prompts: no terms are sent at all.
    assert!(!r.raw_text().contains("Kubernetes") && !r.raw_text().contains("Alice"));
    assert_no_key_in(r, "authorization");
    let info = a.last_call().unwrap();
    assert_eq!(info.cost_usd, Some(0.000225));
    assert!(!info.cost_estimated);
    assert_eq!(info.billed_audio_ms, 3000);
    assert_eq!(info.request_id.as_deref(), Some("gen-abc123"));
}

#[test]
fn zero_retention_off_omits_the_provider_block() {
    let srv = MockServer::fixed(Reply::json(200, fixture("openrouter/success.json")));
    let mut p = provider("openrouter", &srv, "/api/v1");
    p.zero_retention = false;
    run(&asr(p), &req(&[], &[], None)).unwrap();
    let j = srv.requests()[0].json();
    assert!(j.get("provider").is_none() && j.get("language").is_none());
}

#[test]
fn long_audio_is_chunked_below_the_60_s_upstream_limit() {
    let srv = MockServer::start(|_, i| {
        Reply::json(
            200,
            serde_json::json!({"text": format!("part{i}"), "usage": {"seconds": 1.0, "cost": 0.001}}),
        )
    });
    let a = asr(provider("openrouter", &srv, "/api/v1"));
    let out = a.transcribe(&speech(70_000), &req(&[], &[], None)).unwrap();
    assert_eq!(out.text, "part0 part1");
    assert_eq!(srv.count(), 2);
    let info = a.last_call().unwrap();
    assert_eq!(info.requests, 2);
    assert!((info.cost_usd.unwrap() - 0.002).abs() < 1e-12, "costs sum");
    assert_eq!(info.billed_audio_ms, 2000);
}

#[test]
fn credits_error_is_quota() {
    let srv = MockServer::fixed(Reply::json(402, fixture("openrouter/error_credits.json")));
    let e = run(
        &asr(provider("openrouter", &srv, "/api/v1")),
        &req(&[], &[], None),
    )
    .unwrap_err();
    assert_eq!(e.kind, flow_router::Failure::QuotaExceeded);
    assert!(!e.message.contains("openrouter.ai/credits"));
}

#[test]
fn error_mapping() {
    check_error_mapping("openrouter", "/api/v1", "/audio/transcriptions");
}

#[test]
fn transport_errors() {
    check_transport_errors("openrouter", "/api/v1");
}
