//! CloudBackend (+ Router) against a mock OpenRouter: the pipeline runs
//! locally, silence is never uploaded, failures are typed for fallback.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use flow_cloud::testing::Reply;
use flow_cloud::CloudBackend;

type Srv = Mock;
use flow_core::config::SessionConfig;
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::CommitStatus;
use flow_router::{Backend, Failure, Route, Router, RouterConfig, SecretString};

fn speech_i16(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn backend(_srv: &Srv) -> CloudBackend {
    CloudBackend::new(
        settings(flow_cloud::DEFAULT_STT_MODEL),
        SecretString::new(KEY),
        None,
    )
}

fn say(b: &dyn Backend, pcm: &[i16]) {
    for c in pcm.chunks(1600) {
        assert!(b.push_audio(c));
    }
}

fn reply(text: &str) -> Reply {
    Reply::json(200, serde_json::json!({ "text": text }))
}

#[test]
fn silence_makes_no_request() {
    let srv = mock_fixed(reply("thank you"));
    let b = backend(&srv);
    b.start_session(&SessionConfig::default()).unwrap();
    say(&b, &vec![0i16; 16_000 * 2]);
    let r = b.commit(Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(r.status, CommitStatus::Empty);
    assert_eq!(r.empty_reason.as_deref(), Some("no_speech"));
    assert_eq!(srv.count(), 0, "C6: silence never leaves the device");
    let route = r.route.unwrap();
    assert_eq!(route.engine, "cloud");
    assert_eq!(route.billed_audio_ms, None, "nothing billed");
}

#[test]
fn provider_text_goes_through_the_local_pipeline() {
    let srv = mock_fixed(reply("um so i use wispr every day"));
    let b = backend(&srv);
    let cfg = SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::replacement("wispr", "Wispr")],
        },
        ..Default::default()
    };
    b.start_session(&cfg).unwrap();
    say(&b, &speech_i16(1500));
    let r = b.commit(Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(r.status, CommitStatus::RawTranscript);
    assert_eq!(r.asr_text, "um so i use wispr every day");
    // Same as a local engine returning that text.
    let local = flow_core::FlowCore::from_engines(
        Arc::new(flow_core::asr::MockAsr::new("um so i use wispr every day")),
        None,
    );
    local.start_session(cfg).unwrap();
    for c in speech_i16(1500).chunks(1600) {
        local.push_audio(c);
    }
    let want = local.commit();
    assert_eq!(r.final_text, want.final_text);
    assert!(r.final_text.contains("Wispr"), "{}", r.final_text);
    let route = r.route.unwrap();
    assert_eq!(route.provider.as_deref(), Some("openrouter"));
    assert_eq!(
        route.model.as_deref(),
        Some("openai/whisper-large-v3-turbo")
    );
    assert!(route.billed_audio_ms.unwrap() > 0);
    // No usage.cost: the catalogue estimate.
    assert_eq!(route.cost_estimated, Some(true));
    assert!(route.cost_usd.is_some());
    // The dictionary went to Groq whisper as vocabulary.
    assert_eq!(
        srv.requests()[0].json()["provider"]["options"]["groq"]["prompt"],
        "Vocabulary: Wispr."
    );
}

#[test]
fn no_partials_and_no_upload_before_commit() {
    let srv = mock_fixed(reply("hello"));
    let b = backend(&srv);
    b.start_session(&SessionConfig {
        partial_interval_ms: Some(50),
        ..Default::default()
    })
    .unwrap();
    say(&b, &speech_i16(2000));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(b.partial(), None);
    assert_eq!(srv.count(), 0, "no audio leaves before commit");
    b.commit(Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(srv.count(), 1);
}

#[test]
fn transport_error_is_a_typed_err_for_the_router() {
    let srv = mock_fixed(Reply::json(401, serde_json::json!({})));
    let b = backend(&srv);
    b.start_session(&SessionConfig::default()).unwrap();
    say(&b, &speech_i16(1000));
    let e = b
        .commit(Instant::now() + Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(e.kind, Failure::AuthFailed);
    assert!(!e.message.contains(KEY));
    let info = b.last_call().unwrap();
    assert_eq!(info.error.as_deref(), Some("auth_failed"));
}

#[test]
fn commit_deadline_is_honoured() {
    let srv = mock_fixed(reply("late").delayed(Duration::from_secs(4)));
    let b = backend(&srv);
    b.start_session(&SessionConfig {
        llm_timeout_ms: 0,
        ..Default::default()
    })
    .unwrap();
    say(&b, &speech_i16(1000));
    let t = Instant::now();
    let e = b
        .commit(Instant::now() + Duration::from_millis(1200))
        .unwrap_err();
    assert_eq!(e.kind, Failure::Timeout);
    assert!(
        t.elapsed() < Duration::from_millis(2500),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn empty_provider_answer_is_final_blank_asr() {
    let srv = mock_fixed(reply(""));
    let b = backend(&srv);
    b.start_session(&SessionConfig::default()).unwrap();
    say(&b, &speech_i16(1000));
    let r = b.commit(Instant::now() + Duration::from_secs(5)).unwrap();
    assert_eq!(r.status, CommitStatus::Empty);
    assert_eq!(r.empty_reason.as_deref(), Some("blank_asr"));
}

#[test]
fn probe_transcribes_the_test_clip() {
    let srv = mock_fixed(reply("And so, my fellow Americans,"));
    let b = backend(&srv);
    let h = b.probe();
    assert!(h.ok, "{h:?}");
    assert_eq!(
        h.transcript.as_deref(),
        Some("And so, my fellow Americans,")
    );
    assert_eq!(h.provider.as_deref(), Some("openrouter"));
    assert!(h.latency_ms.is_some() && h.cost_usd.is_some());
    let r = &srv.requests()[0];
    assert!(
        r.json()["provider"].get("options").is_none(),
        "no dictionary in a test call"
    );
    drop(srv);
    let bad = mock_fixed(Reply::json(401, serde_json::json!({})));
    let h = backend(&bad).probe();
    assert!(!h.ok);
    assert_eq!(h.error.as_deref(), Some("auth_failed"));
}

#[test]
fn router_with_cloud_attached_end_to_end() {
    let _srv = mock_fixed(reply("hello from the cloud"));
    // A production-shaped entry: the mock is reached only through the
    // test base-URL override, never through config.
    let mut p = flow_cloud::presets().remove(0);
    p.id = "openrouter".into();
    p.consent_host = "openrouter.ai".into();
    p.consent_at_ms = 1;
    let cfg = RouterConfig {
        allow_cloud: true,
        allow_server: false,
        order: vec![Route::Cloud, Route::Client],
        cloud: flow_router::CloudConfig {
            active: p.id.clone(),
            providers: vec![p],
            monthly_budget_usd: None,
        },
        ..Default::default()
    };
    cfg.validate().unwrap();
    let router = flow_cloud::attach(Router::new(cfg, None, None), None);
    assert!(router.cloud_capabilities().cloud);
    assert_eq!(router.cloud_capabilities().adapters, ["openrouter"]);
    router.set_cloud_key("openrouter", Some(SecretString::new(KEY)));
    router.start_session(SessionConfig::default()).unwrap();
    for c in speech_i16(1000).chunks(1600) {
        router.push_audio(c);
    }
    let r = router.commit();
    assert_eq!(r.asr_text, "hello from the cloud");
    let route = r.route.unwrap();
    assert_eq!(route.engine, "cloud");
    assert_eq!(route.provider.as_deref(), Some("openrouter"));
    let u = router.cloud_usage();
    assert_eq!(u.providers[0].requests, 1);
    assert!(u.providers[0].cost_estimated);
}
