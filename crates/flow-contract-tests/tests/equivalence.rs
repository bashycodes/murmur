//! I2: a session gives the same result locally (FlowCore directly) and
//! remotely (Router{allow_server only} → flow-remote → in-process
//! flow-server), compared as JSON minus `timings` and `route`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use flow_contract_tests::*;
use flow_core::asr::MockAsr;
#[cfg(any(feature = "whisper", feature = "parakeet"))]
use flow_core::config::FieldContext;
#[cfg(any(feature = "whisper", feature = "parakeet", feature = "llm"))]
use flow_core::config::ModelConfig;
use flow_core::config::SessionConfig;
use flow_core::{CommitStatus, FlowCore};

fn mock_core(text: &str) -> FlowCore {
    FlowCore::from_engines(Arc::new(MockAsr::new(text)), None)
}

/// Run `case` both ways with mock ASR; returns (local, remote).
fn both(case: &Case) -> (flow_core::CommitResult, flow_core::CommitResult) {
    let text = case.mock_text().expect("mock case");
    let pcm = audio(&case.audio);
    let local = run_case(&mock_core(text), case, &pcm);
    let server = spawn_mock_server(text);
    let router = remote_router(server.addr());
    let remote = run_case(&router, case, &pcm);
    let route = remote.route.clone().expect("route");
    assert_eq!(
        route.engine, "remote",
        "{}: {route:?} {:?}",
        case.name, remote.error
    );
    assert_eq!(route.fallback, None, "{}", case.name);
    (local, remote)
}

fn case(name: &str) -> Case {
    load_case(&fixtures_dir().join(format!("{name}.json")))
}

#[test]
fn equivalence_session_fixtures_mock_asr() {
    let cases = load_cases();
    assert!(cases.len() >= 40, "only {} fixtures", cases.len());
    let mut n = 0;
    for c in cases.iter().filter(|c| c.mock_text().is_some()) {
        let (l, r) = both(c);
        assert_eq!(normalize(&l), normalize(&r), "case {}", c.name);
        n += 1;
    }
    eprintln!("{n} mock fixtures equivalent");
    assert!(n >= 40);
}

#[test]
fn equivalence_empty_reasons() {
    for (name, reason) in [
        ("p1_no_audio", "no_audio"),
        ("p1_no_speech_silence", "no_speech"),
        ("p1_no_speech_room_noise", "no_speech"),
        ("p1_blank_asr", "blank_asr"),
        ("p1_hallucination_thank_you", "hallucination"),
        ("p1_only_filler", "only_filler"),
    ] {
        let (l, r) = both(&case(name));
        assert_eq!(l.status, CommitStatus::Empty, "{name}");
        assert_eq!(l.empty_reason.as_deref(), Some(reason), "{name}");
        assert_eq!(normalize(&l), normalize(&r), "{name}");
    }
}

#[test]
fn equivalence_segments_and_press_enter() {
    let (l, r) = both(&case("p1_press_enter_segments"));
    assert!(l.press_enter);
    assert!(!l.segments.is_empty());
    assert_eq!(l.segments, r.segments);
    assert_eq!(l.plain_text, r.plain_text);
    assert_eq!(l.press_enter, r.press_enter);
    let (l, r) = both(&case("p1_break_and_tab_markers"));
    assert_eq!(normalize(&l), normalize(&r));
}

#[test]
fn equivalence_late_context() {
    // Context arrives after all audio, just before commit.
    let mut c = case("p1_two_contexts_last_wins");
    for x in &mut c.contexts {
        x.at_ms = 10_000;
    }
    let (l, r) = both(&c);
    assert_eq!(normalize(&l), normalize(&r));
    let early = both(&case("p1_two_contexts_last_wins")).1;
    assert_eq!(
        normalize(&early),
        normalize(&r),
        "last context wins either way"
    );
}

#[test]
fn equivalence_cancel_then_commit_is_error() {
    let local = mock_core("x");
    local.start_session(SessionConfig::default()).unwrap();
    local.push_audio(&speech(300));
    local.cancel();
    let l = local.commit();
    let server = spawn_mock_server("x");
    let router = remote_router(server.addr());
    router.start_session(SessionConfig::default()).unwrap();
    router.push_audio(&speech(300));
    router.cancel();
    let r = router.commit();
    assert_eq!(l.status, CommitStatus::Error);
    assert_eq!(r.status, CommitStatus::Error);
}

#[test]
fn equivalence_max_samples_truncated() {
    let c = Case {
        name: "truncated".into(),
        asr: serde_json::json!({"mock": "long dictation"}),
        audio: "speech:2000".into(),
        session_config: SessionConfig {
            max_samples: 16_000,
            ..Default::default()
        },
        contexts: vec![],
        learn: None,
    };
    let (l, r) = both(&c);
    assert!(l.truncated && r.truncated);
    assert_eq!(l.timings.audio_ms, 1000);
    assert_eq!(r.timings.audio_ms, 1000);
    assert_eq!(l.timings.dropped_samples, r.timings.dropped_samples);
    assert_eq!(normalize(&l), normalize(&r));
}

/// Push 2 s at real-time pace, polling `partial` every 50 ms.
fn partials(d: &dyn Dictation, poll: &dyn Fn() -> Option<String>) -> Vec<String> {
    d.d_start(SessionConfig {
        partial_interval_ms: Some(300),
        ..Default::default()
    });
    let mut seen: Vec<String> = Vec::new();
    for chunk in speech(2000).chunks(FRAME) {
        d.d_push(chunk);
        std::thread::sleep(Duration::from_millis(100));
        if let Some(p) = poll() {
            if seen.last() != Some(&p) {
                seen.push(p);
            }
        }
    }
    let t = Instant::now();
    while seen.is_empty() && t.elapsed() < Duration::from_secs(2) {
        if let Some(p) = poll() {
            seen.push(p);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    d.d_commit();
    seen
}

#[test]
fn equivalence_partials_mock() {
    let local = mock_core("ask not");
    let l = partials(&local, &|| local.partial());
    let server = spawn_mock_server("ask not");
    let router = remote_router(server.addr());
    let r = partials(&router, &|| router.partial());
    assert_eq!(l, vec!["ask not".to_string()]);
    assert_eq!(l, r);
}

#[test]
fn equivalence_learn_edit() {
    let c = case("p1_learn_edit");
    let learn = c.learn.clone().unwrap();
    let local = mock_core(c.mock_text().unwrap());
    run_case(&local, &c, &audio(&c.audio));
    let server = spawn_mock_server(c.mock_text().unwrap());
    let router = remote_router(server.addr());
    run_case(&router, &c, &audio(&c.audio));
    let l = local.learn_edit(&learn.pasted, &learn.edited);
    let r = router.learn_edit(&learn.pasted, &learn.edited);
    assert!(!l.is_empty());
    assert_eq!(l, r);
}

// ---- real models (skip when absent) ---------------------------------------

#[cfg(any(feature = "whisper", feature = "parakeet"))]
fn real_equivalence(model: ModelConfig, wav: &str, cfg: SessionConfig) -> Option<()> {
    let Some(_) = find_model(wav) else {
        eprintln!("SKIP: {wav} not found");
        return None;
    };
    let pcm = audio(wav);
    let c = Case {
        name: wav.into(),
        asr: serde_json::Value::Null,
        audio: wav.into(),
        session_config: cfg,
        contexts: vec![CtxAt {
            at_ms: 0,
            ctx: FieldContext {
                app_id: "notepad".into(),
                ..Default::default()
            },
        }],
        learn: None,
    };
    let local = local_backend(&model);
    let l = run_case(local.as_ref(), &c, &pcm);
    drop(local);
    let server = spawn_server(server_cfg(model));
    let router = remote_router(server.addr());
    let r = run_case(&router, &c, &pcm);
    eprintln!(
        "local: asr {} ms total {} ms | remote: total {} ms | {:?}",
        l.timings.asr_ms, l.timings.total_ms, r.timings.total_ms, r.final_text
    );
    assert_eq!(r.route.as_ref().unwrap().engine, "remote");
    assert_eq!(normalize(&l), normalize(&r));
    assert!(!l.final_text.is_empty());
    Some(())
}

#[cfg(feature = "whisper")]
#[test]
fn equivalence_whisper_tiny_jfk_if_present() {
    let Some(m) = find_model("ggml-tiny.en.bin") else {
        eprintln!("SKIP: ggml-tiny.en.bin not found");
        return;
    };
    let model = ModelConfig {
        asr_path: m.to_string_lossy().into(),
        threads: 4,
        ..Default::default()
    };
    real_equivalence(model, "jfk.wav", SessionConfig::default());
}

#[cfg(feature = "parakeet")]
#[test]
fn equivalence_parakeet_if_present() {
    let dir = std::env::var_os("FLOW_PARAKEET_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| find_model("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"));
    let Some(dir) = dir.filter(|d| d.join("tokens.txt").is_file()) else {
        eprintln!("SKIP: parakeet model not found (FLOW_PARAKEET_DIR)");
        return;
    };
    let model: ModelConfig = serde_json::from_value(serde_json::json!({
        "asr_engine": "parakeet", "asr_path": dir, "threads": 4
    }))
    .unwrap();
    real_equivalence(model, "jfk.wav", SessionConfig::default());
}

#[cfg(feature = "llm")]
#[test]
fn equivalence_llm_if_present() {
    let Some(gguf) = find_model("qwen2.5-0.5b-instruct-q4_k_m.gguf") else {
        eprintln!("SKIP: qwen2.5-0.5b gguf not found");
        return;
    };
    if !flow_core::llama::plugin_candidates(None)
        .iter()
        .any(|p| p.is_absolute() && p.is_file())
    {
        eprintln!("SKIP: libflow_llm plugin not built");
        return;
    }
    let model = ModelConfig {
        llm_path: Some(gguf.to_string_lossy().into()),
        threads: 4,
        ..mock_model("so um i think we should meet tomorrow at noon and uh bring the slides")
    };
    // Generous budget: greedy decoding is deterministic once it finishes.
    let cfg = SessionConfig {
        llm_timeout_ms: 60_000,
        llm_prefill: false,
        ..Default::default()
    };
    let c = Case {
        name: "llm".into(),
        asr: serde_json::Value::Null,
        audio: "speech:1500".into(),
        session_config: cfg,
        contexts: vec![],
        learn: None,
    };
    let pcm = audio(&c.audio);
    let local = local_backend(&model);
    let l = run_case(local.as_ref(), &c, &pcm);
    drop(local);
    let server = spawn_server(server_cfg(model));
    let r = run_case(&remote_router(server.addr()), &c, &pcm);
    eprintln!(
        "llm local {:?}: {} | remote: {}",
        l.status, l.final_text, r.final_text
    );
    assert_eq!(normalize(&l), normalize(&r));
}
