//! Additive API surface requested by the shells (docs/core-change-requests.md):
//! distinct load errors (CR-1) and structured paste segments (CR-2).

use std::sync::Arc;

use flow_core::asr::MockAsr;
#[cfg(any(feature = "whisper", feature = "llm"))]
use flow_core::config::ModelConfig;
use flow_core::config::{FieldContext, SessionConfig};
use flow_core::paste::{PasteKey, PasteSegment};
use flow_core::FlowCore;

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn run_text(asr_text: &str, cfg: SessionConfig, ctx: FieldContext) -> flow_core::CommitResult {
    let core = FlowCore::from_engines(Arc::new(MockAsr::new(asr_text)), None);
    core.start_session(cfg).unwrap();
    core.set_context(ctx);
    core.push_audio(&speech(300));
    core.commit()
}

fn text(s: &str) -> PasteSegment {
    PasteSegment::Text { text: s.into() }
}
fn key(k: PasteKey) -> PasteSegment {
    PasteSegment::Key { key: k }
}

#[cfg(feature = "whisper")]
#[test]
fn missing_whisper_model_is_model_not_found() {
    let cfg = ModelConfig {
        asr_path: "/nonexistent/ggml-x.bin".into(),
        ..Default::default()
    };
    match FlowCore::load_models(&cfg) {
        Err(flow_core::CoreError::ModelNotFound { path }) => {
            assert_eq!(path, "/nonexistent/ggml-x.bin")
        }
        Err(e) => panic!("wrong error: {e:?}"),
        Ok(_) => panic!("loaded a missing model"),
    }
}

#[cfg(feature = "llm")]
#[test]
fn missing_llm_model_is_model_not_found() {
    let cfg = ModelConfig {
        asr_backend: flow_core::config::AsrBackend::Mock { text: "x".into() },
        llm_path: Some("/nonexistent/q.gguf".into()),
        ..Default::default()
    };
    match FlowCore::load_models(&cfg) {
        Err(flow_core::CoreError::ModelNotFound { path }) => {
            assert_eq!(path, "/nonexistent/q.gguf")
        }
        Err(e) => panic!("wrong error: {e:?}"),
        Ok(_) => panic!("loaded a missing model"),
    }
}

#[cfg(feature = "llm")]
#[test]
fn missing_llm_plugin_is_llm_lib_not_found() {
    let model = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let cfg = ModelConfig {
        asr_backend: flow_core::config::AsrBackend::Mock { text: "x".into() },
        llm_path: Some(model.into()),
        llm_lib: Some("/nonexistent/libflow_llm.so".into()),
        ..Default::default()
    };
    match FlowCore::load_models(&cfg) {
        Err(e @ flow_core::CoreError::LlmLibNotFound { .. }) => {
            assert!(e.to_string().contains("/nonexistent/libflow_llm.so"), "{e}")
        }
        Err(e) => panic!("wrong error: {e:?}"),
        Ok(_) => panic!("loaded without a plugin"),
    }
}

#[test]
fn error_kind_is_stable_snake_case() {
    use flow_core::CoreError;
    assert_eq!(
        CoreError::ModelNotFound { path: "p".into() }.kind(),
        "model_not_found"
    );
    assert_eq!(
        CoreError::LlmLibNotFound { tried: vec![] }.kind(),
        "llm_lib_not_found"
    );
    assert_eq!(CoreError::Load("x".into()).kind(), "load");
}

#[test]
fn plain_commit_is_one_text_segment() {
    let r = run_text(
        "hello world",
        SessionConfig::default(),
        FieldContext::default(),
    );
    assert_eq!(r.segments, vec![text(&r.final_text)]);
    assert_eq!(r.plain_text, r.final_text);
}

#[test]
fn ide_tagging_becomes_break_and_tab_keys_while_final_text_keeps_markers() {
    let ctx = FieldContext {
        app_id: "cursor".into(),
        file_names: vec!["index.ts".into(), "app.tsx".into()],
        ..Default::default()
    };
    let r = run_text(
        "Look at index dot ts and fix it.",
        SessionConfig::default(),
        ctx,
    );
    assert!(r.final_text.contains("@{{BREAK}}index.ts{{TAB}}"), "{r:?}");
    let i = r.final_text.find('@').unwrap();
    let head = &r.final_text[..=i];
    let tail = &r.final_text[r.final_text.find("{{TAB}}").unwrap() + 7..];
    assert_eq!(
        r.segments,
        vec![
            text(head),
            key(PasteKey::Break),
            text("index.ts"),
            key(PasteKey::Tab),
            text(tail),
        ]
    );
    assert!(!r.plain_text.contains("{{"), "{r:?}");
    assert!(r.plain_text.contains("@index.ts"), "{r:?}");
}

#[test]
fn press_enter_is_a_trailing_enter_segment() {
    let r = run_text(
        "Send it press enter",
        SessionConfig::default(),
        FieldContext::default(),
    );
    assert!(r.press_enter);
    assert_eq!(r.segments, vec![text("Send it"), key(PasteKey::Enter)]);
}

#[test]
fn empty_commit_has_no_segments() {
    let core = FlowCore::from_engines(Arc::new(MockAsr::new("x")), None);
    core.start_session(SessionConfig::default()).unwrap();
    let r = core.commit();
    assert!(r.segments.is_empty());
}

#[test]
fn segments_serialize_as_text_or_key_objects() {
    let r = run_text(
        "Send it press enter",
        SessionConfig::default(),
        FieldContext::default(),
    );
    let j = serde_json::to_value(&r).unwrap();
    assert_eq!(
        j["segments"],
        serde_json::json!([{"text": "Send it"}, {"key": "Enter"}])
    );
    // Old JSON without the new fields still parses (shells built earlier).
    let mut old = j.clone();
    old.as_object_mut().unwrap().remove("segments");
    old.as_object_mut().unwrap().remove("plain_text");
    let back: flow_core::CommitResult = serde_json::from_value(old).unwrap();
    assert!(back.segments.is_empty());
}

#[test]
fn segment_splitter_handles_adjacent_markers() {
    use flow_core::paste::segments;
    assert_eq!(
        segments("{{TAB}}{{BREAK}}", false),
        vec![key(PasteKey::Tab), key(PasteKey::Break)]
    );
    assert_eq!(segments("", true), vec![key(PasteKey::Enter)]);
    assert!(segments("", false).is_empty());
}

// --- Hybrid (D17): route info is the router's, never the core's ---

#[test]
fn core_never_sets_route_and_omits_it_from_json() {
    let r = run_text("hello", SessionConfig::default(), FieldContext::default());
    assert!(r.route.is_none());
    let j = serde_json::to_value(&r).unwrap();
    assert!(
        j.get("route").is_none(),
        "route must be skipped when None: {j}"
    );
}

#[test]
fn route_info_round_trips_and_old_json_still_parses() {
    let mut r = run_text("hello", SessionConfig::default(), FieldContext::default());
    r.route = Some(flow_core::RouteInfo {
        engine: "remote".into(),
        fallback: None,
        server: Some("100.64.0.10:7313".into()),
        rtt_ms: Some(4),
        queue_ms: Some(0),
        server_core: Some("0.1.0".into()),
        ..Default::default()
    });
    let j = serde_json::to_string(&r).unwrap();
    let back: flow_core::CommitResult = serde_json::from_str(&j).unwrap();
    assert_eq!(back, r);
    // A CommitResult serialised before `route` existed still parses.
    let mut v = serde_json::to_value(&r).unwrap();
    v.as_object_mut().unwrap().remove("route");
    let old: flow_core::CommitResult = serde_json::from_value(v).unwrap();
    assert!(old.route.is_none());
}

#[test]
fn postprocess_rev_and_version_are_exposed() {
    assert!(!flow_core::POSTPROCESS_REV.is_empty());
    assert_eq!(flow_core::VERSION, env!("CARGO_PKG_VERSION"));
}

// ---- D18 (cloud STT) additions -------------------------------------------

/// Records every request; answers "x".
struct Spy(std::sync::Mutex<Vec<flow_core::asr::AsrRequest>>);
impl flow_core::asr::AsrEngine for Spy {
    fn transcribe(
        &self,
        _pcm: &[f32],
        req: &flow_core::asr::AsrRequest,
    ) -> Result<flow_core::asr::AsrOutput, flow_core::asr::AsrError> {
        self.0.lock().unwrap().push(req.clone());
        Ok(flow_core::asr::AsrOutput {
            text: "x".into(),
            ..Default::default()
        })
    }
}

#[test]
fn asr_request_carries_dictionary_and_screen_terms_separately() {
    use flow_core::asr::AsrPurpose;
    use flow_core::dictionary::{DictEntry, Dictionary};
    let spy = Arc::new(Spy(Default::default()));
    let core = FlowCore::from_engines(spy.clone(), None);
    let cfg = SessionConfig {
        dictionary: Dictionary {
            entries: vec![
                DictEntry::word("Kubernetes"),
                DictEntry::replacement("wispr", "Wispr"),
                DictEntry::word("kubernetes"),
            ],
        },
        ..Default::default()
    };
    core.start_session(cfg).unwrap();
    core.set_context(FieldContext {
        screen_words: vec!["Alice".into(), "WISPR".into(), " ".into()],
        ..Default::default()
    });
    core.push_audio(&speech(300));
    core.commit();
    let reqs = spy.0.lock().unwrap();
    let req = reqs.last().unwrap();
    assert_eq!(req.dictionary_terms, ["Kubernetes", "Wispr"]);
    assert_eq!(req.screen_terms, ["Alice"]);
    assert_eq!(req.purpose, AsrPurpose::Final);
    // The whisper prompt is unchanged.
    assert_eq!(req.initial_prompt, "Kubernetes, Wispr, Alice.");
}

#[test]
fn partial_requests_are_marked_partial() {
    use flow_core::asr::AsrPurpose;
    let spy = Arc::new(Spy(Default::default()));
    let core = FlowCore::from_engines(spy.clone(), None);
    core.start_session(SessionConfig {
        partial_interval_ms: Some(100),
        ..Default::default()
    })
    .unwrap();
    for c in speech(1000).chunks(1600) {
        core.push_audio(c);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while core.partial().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    core.commit();
    let reqs = spy.0.lock().unwrap();
    assert!(reqs.iter().any(|r| r.purpose == AsrPurpose::Partial));
    assert_eq!(reqs.last().unwrap().purpose, AsrPurpose::Final);
}

#[test]
fn cleaner_accessor_returns_the_shared_cleaner() {
    struct C;
    impl flow_core::cleanup::Cleaner for C {
        fn clean(
            &self,
            _p: &flow_core::cleanup::CleanPrompt,
            _d: std::time::Instant,
        ) -> Result<String, flow_core::cleanup::CleanError> {
            Ok(String::new())
        }
    }
    let c: Arc<dyn flow_core::cleanup::Cleaner> = Arc::new(C);
    let core = FlowCore::from_engines(Arc::new(MockAsr::new("x")), Some(c.clone()));
    assert!(Arc::ptr_eq(&core.cleaner().unwrap(), &c));
    let bare = FlowCore::from_engines(Arc::new(MockAsr::new("x")), None);
    assert!(bare.cleaner().is_none());
}

#[test]
fn route_info_cloud_fields_round_trip_and_are_omitted_when_none() {
    let r = flow_core::RouteInfo {
        engine: "cloud".into(),
        provider: Some("openrouter-1".into()),
        model: Some("openai/gpt-transcribe".into()),
        billed_audio_ms: Some(3200),
        cost_usd: Some(flow_core::Usd(0.00024)),
        cost_estimated: Some(false),
        ..Default::default()
    };
    let j = serde_json::to_value(&r).unwrap();
    assert_eq!(j["cost_usd"], serde_json::json!(0.00024));
    assert_eq!(j["billed_audio_ms"], 3200);
    let back: flow_core::RouteInfo = serde_json::from_value(j).unwrap();
    assert_eq!(back, r);
    let local = serde_json::to_value(flow_core::RouteInfo::default()).unwrap();
    for k in [
        "provider",
        "model",
        "billed_audio_ms",
        "cost_usd",
        "cost_estimated",
    ] {
        assert!(local.get(k).is_none(), "{k} must be omitted: {local}");
    }
}
