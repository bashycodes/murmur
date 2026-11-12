//! D84: cloud speech-to-text on the user's LiteLLM proxy, against a fake
//! OpenAI-compatible server (plain http on loopback; the URL is user-typed,
//! so no base-URL override): the multipart `/audio/transcriptions` wire,
//! the optional key, the LiteLLM cost headers, the WAV retry, speech-model
//! detection, and whole dictations through the router. Audio is synthetic.

use std::time::{Duration, Instant};

use murmur_cloud::custom::{self, COST_HEADER, COST_HEADER_ORIGINAL};
use murmur_cloud::testing::{MockServer, Recorded, Reply};
use murmur_cloud::CloudAsr;
use murmur_core::asr::{AsrEngine, AsrPurpose, AsrRequest};
use murmur_core::config::SessionConfig;
use murmur_router::cloud::CustomCleanupConfig;
use murmur_router::{CloudProvider, Route, Router, RouterConfig, SecretString};

const KEY: &str = "sk-litellm-LEAKCHECK-stt-0001";
const MODEL: &str = "local-whisper";

fn cfg(base: &str) -> CustomCleanupConfig {
    let a = murmur_router::cloud::custom_server_address(base).unwrap();
    CustomCleanupConfig {
        base_url: a.url,
        model: "local-cleanup".into(),
        stt_model: MODEL.into(),
        stt_consent_host: a.host,
        stt_consent_at_ms: 1,
        ..Default::default()
    }
}

/// A made-up tone, loud enough for the VAD (never anyone's voice).
fn tone_f32(ms: usize) -> Vec<f32> {
    (0..ms * 16)
        .map(|i| (i as f32 * 0.05).sin() * 0.25)
        .collect()
}

fn tone_i16(ms: usize) -> Vec<i16> {
    tone_f32(ms).iter().map(|s| (s * 32767.0) as i16).collect()
}

fn req(dict: &[&str], lang: Option<&str>) -> AsrRequest {
    AsrRequest {
        language: lang.map(str::to_string),
        purpose: AsrPurpose::Final,
        dictionary_terms: dict.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn text(t: &str) -> Reply {
    Reply::json(200, serde_json::json!({ "text": t }))
}

fn no_key_in(r: &Recorded) {
    for (k, v) in &r.headers {
        if k != "authorization" {
            assert!(!v.contains(KEY), "key in header {k}");
        }
    }
    assert!(!r.raw_text().replace(KEY, "").is_empty() || r.body.is_empty());
    let body = String::from_utf8_lossy(&r.body);
    assert!(!body.contains(KEY), "key in the body");
}

#[test]
fn multipart_transcription_with_key_language_and_terms() {
    let srv = MockServer::fixed(text(" Ship the Zephyr build. ").header(COST_HEADER, "0.0006"));
    let asr = CloudAsr::litellm(&cfg(&srv.url("")), Some(SecretString::new(KEY)));
    let out = asr
        .transcribe(&tone_f32(1500), &req(&["Zephyr"], Some("en")))
        .unwrap();
    assert_eq!(out.text, "Ship the Zephyr build.");
    let r = &srv.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path(), "/v1/audio/transcriptions", "typed without /v1");
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert!(r
        .header("content-type")
        .is_some_and(|c| c.starts_with("multipart/form-data")));
    assert_eq!(r.field("model").as_deref(), Some(MODEL));
    assert_eq!(r.field("response_format").as_deref(), Some("json"));
    assert_eq!(r.field("temperature").as_deref(), Some("0"));
    assert_eq!(r.field("language").as_deref(), Some("en"));
    assert_eq!(r.field("prompt").as_deref(), Some("Vocabulary: Zephyr."));
    let file = r.parts().into_iter().find(|p| p.name == "file").unwrap();
    assert_eq!(file.filename.as_deref(), Some("audio.flac"));
    assert_eq!(file.content_type.as_deref(), Some("audio/flac"));
    assert_eq!(&file.data[..4], b"fLaC");
    // None of OpenRouter's extensions.
    for k in ["provider", "input_audio", "stream"] {
        assert!(r.field(k).is_none(), "{k}");
    }
    no_key_in(r);
    let info = asr.last_call().unwrap();
    assert_eq!(info.provider_id, "custom");
    assert_eq!(info.adapter, "litellm");
    assert_eq!(info.model, MODEL);
    assert_eq!(info.stt_path, "transcriptions");
    assert_eq!(info.cost_usd, Some(0.0006), "the proxy's cost header");
    assert!(!info.cost_estimated);
    assert_eq!(info.billed_audio_ms, 1500, "no duration in json: the chunk");
}

#[test]
fn no_key_no_terms_no_language_and_the_original_cost_header() {
    let srv = MockServer::fixed(
        Reply::json(200, serde_json::json!({"text": "hello", "duration": 1.25}))
            .header(COST_HEADER_ORIGINAL, "0.0002"),
    );
    let asr = CloudAsr::litellm(&cfg(&srv.url("/v1")), None);
    asr.transcribe(&tone_f32(1500), &req(&[], None)).unwrap();
    let r = &srv.requests()[0];
    assert_eq!(r.header("authorization"), None, "no key: no Authorization");
    assert!(r.field("language").is_none());
    assert!(r.field("prompt").is_none());
    let info = asr.last_call().unwrap();
    assert_eq!(info.cost_usd, Some(0.0002));
    assert_eq!(info.billed_audio_ms, 1250, "the proxy's duration");
}

#[test]
fn no_cost_header_is_unknown_and_nothing_is_looked_up() {
    let srv = MockServer::fixed(text("hello"));
    let asr = CloudAsr::litellm(&cfg(&srv.url("")), None)
        .with_cost_lookup_delay(Duration::from_millis(1))
        .with_cost_sink(std::sync::Arc::new(|_| {}));
    asr.transcribe(&tone_f32(1000), &req(&[], None)).unwrap();
    let info = asr.last_call().unwrap();
    assert_eq!(info.cost_usd, None);
    assert!(info.cost_estimated);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(srv.count(), 1, "no /generation lookup on a proxy");
}

#[test]
fn a_format_400_retries_once_with_wav() {
    let srv = MockServer::sequence(vec![
        Reply::json(
            400,
            serde_json::json!({"error": {"message": "Unsupported audio format: flac", "code": "400"}}),
        ),
        text("hello"),
    ]);
    let asr = CloudAsr::litellm(&cfg(&srv.url("")), None);
    assert_eq!(
        asr.transcribe(&tone_f32(1000), &req(&[], None))
            .unwrap()
            .text,
        "hello"
    );
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    let file = reqs[1]
        .parts()
        .into_iter()
        .find(|p| p.name == "file")
        .unwrap();
    assert_eq!(file.filename.as_deref(), Some("audio.wav"));
    assert_eq!(&file.data[..4], b"RIFF");
}

#[test]
fn litellm_errors_are_typed() {
    let srv = MockServer::fixed(Reply::json(
        400,
        serde_json::json!({"error": {"message": "/audio/transcriptions: Invalid model name passed in model=local-whisper.", "type": "invalid_request_error", "code": "400"}}),
    ));
    let asr = CloudAsr::litellm(&cfg(&srv.url("")), None);
    let e = asr
        .transcribe(&tone_f32(1000), &req(&[], None))
        .unwrap_err();
    assert!(!e.to_string().contains("OpenRouter"), "{e}");
    let info = asr.last_call().unwrap();
    assert_eq!(info.error.as_deref(), Some("provider_error"));
    assert_eq!(
        info.message.as_deref(),
        Some("the server has no model by that name: pick one from its list")
    );
    let auth = MockServer::fixed(Reply::json(
        401,
        serde_json::json!({"error": {"message": "Authentication Error", "type": "auth_error", "code": "401"}}),
    ));
    let asr = CloudAsr::litellm(&cfg(&auth.url("")), Some(SecretString::new(KEY)));
    asr.transcribe(&tone_f32(1000), &req(&[], None))
        .unwrap_err();
    let info = asr.last_call().unwrap();
    assert_eq!(info.error.as_deref(), Some("auth_failed"));
    assert_eq!(
        info.message.as_deref(),
        Some("the server rejected the key, or needs one")
    );
}

#[test]
fn speech_models_from_model_info_else_by_name() {
    let srv = MockServer::start(|r, _| match r.path() {
        "/v1/models" => Reply::json(
            200,
            serde_json::json!({"data": [{"id": "local-cleanup"}, {"id": "local-whisper"}, {"id": "voice-x"}]}),
        ),
        "/v1/model/info" => Reply::json(
            200,
            serde_json::json!({"data": [
                {"model_name": "local-cleanup", "model_info": {"mode": "chat"}},
                {"model_name": "voice-x", "model_info": {"mode": "audio_transcription"}},
            ]}),
        ),
        _ => Reply::json(404, serde_json::json!({"detail": "Not Found"})),
    });
    let j = custom::models_with_speech_json(&srv.url(""), None, Duration::from_secs(3));
    assert_eq!(
        j,
        serde_json::json!({
            "models": ["local-cleanup", "local-whisper", "voice-x"],
            "speech_models": ["voice-x"],
        }),
        "LiteLLM's mode wins over the name"
    );
    // A server without /model/info (vLLM, Ollama): by name.
    let plain = MockServer::start(|r, _| match r.path() {
        "/v1/models" => Reply::json(
            200,
            serde_json::json!({"data": [{"id": "gpt-4o-mini"}, {"id": "whisper-1"}]}),
        ),
        _ => Reply::json(404, serde_json::json!({"detail": "Not Found"})),
    });
    let j = custom::models_with_speech_json(&plain.url(""), None, Duration::from_secs(3));
    assert_eq!(j["speech_models"], serde_json::json!(["whisper-1"]));
    // A failed /models is the plain error JSON.
    let down = MockServer::fixed(Reply::json(401, serde_json::json!({})));
    let j = custom::models_with_speech_json(&down.url(""), None, Duration::from_secs(3));
    assert_eq!(j["error"], "auth_failed");
    assert!(j.get("speech_models").is_none());
}

fn router_cfg(base: &str) -> RouterConfig {
    let mut c = RouterConfig {
        allow_cloud: true,
        allow_server: false,
        order: vec![Route::Cloud, Route::Client],
        ..Default::default()
    };
    c.cloud.provider = CloudProvider::LiteLlm;
    c.cloud.custom_cleanup = cfg(base);
    c.validate().unwrap();
    c
}

fn dictate(router: &Router) -> murmur_core::CommitResult {
    router.start_session(SessionConfig::default()).unwrap();
    for c in tone_i16(1200).chunks(1600) {
        router.push_audio(c);
    }
    router.commit()
}

#[test]
fn a_litellm_only_dictation_goes_to_the_proxy_end_to_end() {
    let srv = MockServer::fixed(text("hello from the proxy").header(COST_HEADER, "0.0004"));
    let router = murmur_cloud::attach(Router::new(router_cfg(&srv.url("")), None, None), None);
    router.set_cloud_key("custom", Some(SecretString::new(KEY)));
    let r = dictate(&router);
    assert_eq!(r.asr_text, "hello from the proxy");
    let route = r.route.unwrap();
    assert_eq!(route.engine, "cloud");
    assert_eq!(route.provider.as_deref(), Some("custom"));
    assert_eq!(route.model.as_deref(), Some(MODEL));
    assert_eq!(route.cost_usd.map(|c| c.0), Some(0.0004));
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path(), "/v1/audio/transcriptions");
    assert_eq!(
        reqs[0].header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let u = router.cloud_usage();
    assert_eq!(u.providers.len(), 1);
    assert_eq!(u.providers[0].provider_id, "custom");
    assert_eq!(u.providers[0].requests, 1);
    assert!((u.custom_cost() - 0.0004).abs() < 1e-12);
    assert_eq!(
        u.budget_cost(),
        0.0,
        "the OpenRouter budget never counts it"
    );
    let checks = router.route_checks();
    assert!(checks[0].skip.is_none(), "{checks:?}");
}

#[test]
fn without_audio_consent_or_a_speech_model_nothing_goes_to_the_proxy() {
    let srv = MockServer::fixed(text("never"));
    // Text consent only: not audio consent.
    let mut c = router_cfg(&srv.url(""));
    let host = c.cloud.custom_cleanup.stt_consent_host.clone();
    c.cloud.custom_cleanup.stt_consent_at_ms = 0;
    c.cloud.custom_cleanup.consent_host = host;
    c.cloud.custom_cleanup.consent_at_ms = 1;
    let router = murmur_cloud::attach(Router::new(c, None, None), None);
    let checks = router.route_checks();
    assert_eq!(
        checks[0].skip.as_ref().map(|e| e.kind),
        Some(murmur_router::Failure::CloudNotConfigured)
    );
    assert!(router.start_session(SessionConfig::default()).is_err());
    // Another host voids the audio consent.
    let mut c = router_cfg(&srv.url(""));
    c.cloud.custom_cleanup.base_url = "http://127.0.0.2:9/v1".into();
    let router = murmur_cloud::attach(Router::new(c, None, None), None);
    assert!(router.route_checks()[0].skip.is_some());
    assert_eq!(srv.count(), 0);
}

#[test]
fn litellm_without_a_speech_model_keeps_openrouter_speech() {
    // D78 Android pairing: LiteLLM cleanup, OpenRouter speech.
    let mut c = router_cfg("http://127.0.0.1:9");
    c.cloud.custom_cleanup.stt_model = String::new();
    c.cloud.openrouter.stt.consent_host = "openrouter.ai".into();
    c.cloud.openrouter.stt.consent_at_ms = 1;
    let router = murmur_cloud::attach(Router::new(c, None, None), None);
    // No OpenRouter key: the skip names OpenRouter, not the proxy.
    let skip = router.route_checks()[0].skip.clone().unwrap();
    assert!(skip.message.contains("OpenRouter"), "{skip:?}");
}

#[test]
fn the_speech_test_uses_the_proxy() {
    let srv = MockServer::fixed(text("and so my fellow americans"));
    let c = cfg(&srv.url(""));
    let router = murmur_cloud::attach(Router::new(router_cfg(&srv.url("")), None, None), None);
    router.set_cloud_key("custom", Some(SecretString::new(KEY)));
    let h = router.probe_custom_stt(&c, None);
    assert!(h.ok, "{h:?}");
    assert_eq!(h.provider.as_deref(), Some("custom"));
    assert_eq!(h.model.as_deref(), Some(MODEL));
    assert_eq!(
        srv.requests()[0].header("authorization"),
        Some(format!("Bearer {KEY}").as_str()),
        "the saved key goes to the consented host"
    );
    let mut none = c.clone();
    none.stt_model = String::new();
    let h = router.probe_custom_stt(&none, None);
    assert!(!h.ok);
    assert_eq!(h.error.as_deref(), Some("cloud_not_configured"));
}

#[test]
fn a_new_proxy_key_rebuilds_the_speech_backend() {
    let srv = MockServer::fixed(text("hi"));
    let router = murmur_cloud::attach(Router::new(router_cfg(&srv.url("")), None, None), None);
    router.set_cloud_key("custom", Some(SecretString::new(KEY)));
    dictate(&router);
    router.set_cloud_key("custom", None);
    dictate(&router);
    let reqs = srv.requests();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].header("authorization").is_some());
    assert_eq!(reqs[1].header("authorization"), None);
    let _ = Instant::now();
}
