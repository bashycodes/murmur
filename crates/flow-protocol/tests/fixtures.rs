//! Golden wire fixtures (`fixtures/*.json`): every file parses and
//! re-serialises to the same JSON; the canonical messages built here equal
//! the files. Regenerate after an intentional change with
//! `FLOW_REGEN_FIXTURES=1 cargo test -p flow-protocol --test fixtures`.

use std::path::PathBuf;
use std::sync::Arc;

use flow_core::asr::MockAsr;
use flow_core::config::{AppCategory, Platform};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{CommitResult, FlowCore};
use flow_protocol::*;
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn session_config() -> SessionConfig {
    SessionConfig {
        languages: vec!["en".into()],
        dictionary: Dictionary {
            entries: vec![
                DictEntry::replacement("wispr", "Wispr"),
                DictEntry::snippet("my address", "1 Main St"),
            ],
        },
        platform: Platform::Android,
        partial_interval_ms: Some(500),
        ..Default::default()
    }
}

fn context() -> FieldContext {
    FieldContext {
        app_id: "com.slack".into(),
        app_category: AppCategory::WorkChat,
        before: "Hi team, ".into(),
        screen_words: vec!["Kubernetes".into()],
        ..Default::default()
    }
}

/// A real core result with deterministic timings.
fn commit_result() -> CommitResult {
    let core = FlowCore::from_engines(Arc::new(MockAsr::new("um ask not what wispr can do")), None);
    core.start_session(session_config()).unwrap();
    core.set_context(context());
    core.push_audio(&speech(1000));
    let mut r = core.commit();
    r.timings = flow_core::Timings {
        audio_ms: 1000,
        asr_ms: 812,
        llm_ms: 0,
        postprocess_ms: 1,
        total_ms: 830,
        dropped_samples: 0,
        asr_audio_ms: 1000,
        ..Default::default()
    };
    r
}

/// A text-only cleanup request (D36), as the client's prompt builder
/// records it.
fn clean_request() -> CleanRequest {
    let p = flow_core::cleanup::build_prompt_scoped(
        "the demo is on thursday right",
        flow_core::config::CleanupLevel::Light,
        flow_core::config::Style::Default,
        &context(),
        &session_config().dictionary,
        flow_core::cleanup::PromptScope::FULL,
    )
    .with_timeout(1500);
    (*p.source.unwrap()).clone()
}

fn hello_ok() -> HelloOk {
    HelloOk {
        proto: 1,
        server: "flow-server 0.1.0".into(),
        core: "0.1.0".into(),
        postprocess_rev: "2026-10-21.1".into(),
        asr_backend: "parakeet".into(),
        asr_model: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8".into(),
        llm: false,
        workers: 1,
        busy: 0,
        max_samples: 19_200_000,
        features: vec![],
        languages: vec![],
    }
}

fn client_cases() -> Vec<(&'static str, ClientMsg)> {
    vec![
        (
            "client_hello",
            ClientMsg::Hello(Hello {
                proto_min: 1,
                proto_max: 1,
                client: "flow-android 0.2.0".into(),
                core: "0.1.0".into(),
                postprocess_rev: "2026-10-21.1".into(),
            }),
        ),
        (
            "client_start",
            ClientMsg::Start {
                sid: 7,
                config: session_config(),
            },
        ),
        (
            "client_context",
            ClientMsg::Context {
                sid: 7,
                ctx: context(),
            },
        ),
        (
            "client_start_segmented",
            ClientMsg::Start {
                sid: 8,
                config: SessionConfig {
                    segment_pause_ms: Some(800),
                    endpoint_silence_ms: Some(1500),
                    ..session_config()
                },
            },
        ),
        (
            "client_clean",
            ClientMsg::Clean {
                id: 4,
                req: Box::new(clean_request()),
            },
        ),
        ("client_commit", ClientMsg::Commit { sid: 7 }),
        ("client_cancel", ClientMsg::Cancel { sid: 7 }),
        ("client_ping", ClientMsg::Ping { n: 3 }),
    ]
}

fn server_cases() -> Vec<(&'static str, ServerMsg)> {
    vec![
        ("server_hello_ok", ServerMsg::HelloOk(hello_ok())),
        (
            "server_hello_ok_features",
            ServerMsg::HelloOk(HelloOk {
                features: vec![FEATURE_SEGMENTS.into()],
                ..hello_ok()
            }),
        ),
        (
            "server_hello_ok_languages",
            ServerMsg::HelloOk(HelloOk {
                features: vec![FEATURE_SEGMENTS.into()],
                languages: vec!["he".into()],
                ..hello_ok()
            }),
        ),
        (
            "server_result_segmented",
            ServerMsg::Result {
                sid: 8,
                result: Box::new({
                    let mut r = commit_result();
                    r.timings.wait_ms = 120;
                    r.timings.segments = 3;
                    r.timings.pre_asr_ms = 2400;
                    r
                }),
                server_timings: ServerTimings {
                    queue_ms: 0,
                    recv_audio_ms: 1000,
                },
            },
        ),
        (
            "server_started",
            ServerMsg::Started {
                sid: 7,
                queued: false,
            },
        ),
        (
            "server_started_queued",
            ServerMsg::Started {
                sid: 7,
                queued: true,
            },
        ),
        (
            "server_partial",
            ServerMsg::Partial {
                sid: 7,
                text: "ask not".into(),
            },
        ),
        (
            "server_result",
            ServerMsg::Result {
                sid: 7,
                result: Box::new(commit_result()),
                server_timings: ServerTimings {
                    queue_ms: 0,
                    recv_audio_ms: 1000,
                },
            },
        ),
        (
            "server_hello_ok_clean",
            ServerMsg::HelloOk(HelloOk {
                llm: true,
                features: vec![FEATURE_SEGMENTS.into(), FEATURE_CLEAN.into()],
                ..hello_ok()
            }),
        ),
        (
            "server_cleaned",
            ServerMsg::Cleaned(Cleaned {
                id: 4,
                text: Some("The demo is on Thursday, right?".into()),
                error: None,
                llm_ms: 640,
                queue_ms: 0,
                model: Some("qwen2.5-1.5b-instruct-q4_k_m".into()),
            }),
        ),
        (
            "server_cleaned_timeout",
            ServerMsg::Cleaned(Cleaned {
                id: 5,
                text: None,
                error: Some("llm_timeout".into()),
                llm_ms: 1500,
                queue_ms: 12,
                model: None,
            }),
        ),
        ("server_cancelled", ServerMsg::Cancelled { sid: 7 }),
        ("server_pong", ServerMsg::Pong { n: 3, busy: 0 }),
        (
            "server_error_busy",
            ServerMsg::error(Some(7), code::BUSY, "all workers busy", false),
        ),
        (
            "server_error_proto_mismatch",
            ServerMsg::error(None, code::PROTO_MISMATCH, "client 2..3, server 1..1", true),
        ),
    ]
}

fn pretty(v: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(v).unwrap() + "\n"
}

fn check_or_regen(name: &str, json: String) {
    let p = dir().join(format!("{name}.json"));
    if std::env::var_os("FLOW_REGEN_FIXTURES").is_some() {
        std::fs::write(&p, &json).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let a: Value = serde_json::from_str(&on_disk).unwrap();
    let b: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(a, b, "fixture {name} differs from the canonical message");
}

#[test]
fn canonical_messages_equal_fixtures() {
    for (n, m) in client_cases() {
        check_or_regen(n, pretty(&m));
    }
    for (n, m) in server_cases() {
        check_or_regen(n, pretty(&m));
    }
}

#[test]
fn every_fixture_round_trips() {
    let mut n = 0;
    for e in std::fs::read_dir(dir()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let name = p.file_stem().unwrap().to_str().unwrap().to_string();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        let back = if name.starts_with("client_") {
            let m: ClientMsg = serde_json::from_value(v.clone()).unwrap();
            serde_json::to_value(m).unwrap()
        } else if name.starts_with("server_") {
            let m: ServerMsg = serde_json::from_value(v.clone()).unwrap();
            serde_json::to_value(m).unwrap()
        } else {
            panic!("fixture {name}: name must start with client_ or server_");
        };
        assert_eq!(back, v, "fixture {name} does not round-trip");
        n += 1;
    }
    assert!(n >= 15, "only {n} fixtures");
}

#[test]
fn unknown_fields_are_ignored() {
    let m: ClientMsg =
        serde_json::from_str(r#"{"t":"commit","sid":9,"future_field":{"x":1}}"#).unwrap();
    assert_eq!(m, ClientMsg::Commit { sid: 9 });
    let m: ServerMsg =
        serde_json::from_str(r#"{"t":"hello_ok","proto":1,"server":"s","new_thing":true}"#)
            .unwrap();
    match m {
        ServerMsg::HelloOk(h) => {
            assert_eq!(h.proto, 1);
            assert_eq!(h.workers, 0);
        }
        other => panic!("{other:?}"),
    }
    // A start with a partial config gets core defaults.
    let m: ClientMsg =
        serde_json::from_str(r#"{"t":"start","sid":1,"config":{"raw_mode":true}}"#).unwrap();
    match m {
        ClientMsg::Start { config, .. } => {
            assert!(config.raw_mode);
            assert!(config.vad);
        }
        other => panic!("{other:?}"),
    }
    // A result whose CommitResult lacks newer fields still parses.
    let m: ServerMsg = serde_json::from_str(
        r#"{"t":"result","sid":1,"result":{"status":"EMPTY","asr_text":"","final_text":"",
            "language":null,"timings":{"audio_ms":0,"asr_ms":0,"llm_ms":0,"postprocess_ms":0,
            "total_ms":0,"dropped_samples":0},"press_enter":false,"fallback_reason":null,
            "truncated":false,"error":null}}"#,
    )
    .unwrap();
    assert!(matches!(m, ServerMsg::Result { .. }));
}

/// D36 is additive: an old server's hello (no `clean` feature) parses with
/// no features, and a `clean` with a partial request gets core defaults.
#[test]
fn d36_clean_is_additive() {
    let hello: ServerMsg =
        serde_json::from_str(&std::fs::read_to_string(dir().join("server_hello_ok.json")).unwrap())
            .unwrap();
    let ServerMsg::HelloOk(h) = hello else {
        panic!()
    };
    assert!(!h.features.iter().any(|f| f == FEATURE_CLEAN));
    let m: ClientMsg =
        serde_json::from_str(r#"{"t":"clean","id":2,"req":{"text":"hi","later":1}}"#).unwrap();
    let ClientMsg::Clean { id, req } = m else {
        panic!()
    };
    assert_eq!((id, req.text.as_str()), (2, "hi"));
    assert_eq!(req.timeout_ms, 0);
    let c: ServerMsg = serde_json::from_str(r#"{"t":"cleaned","id":2,"error":"busy"}"#).unwrap();
    let ServerMsg::Cleaned(c) = c else { panic!() };
    assert_eq!((c.text, c.error.as_deref()), (None, Some("busy")));
}

#[test]
fn unknown_message_type_is_an_error() {
    assert!(serde_json::from_str::<ClientMsg>(r#"{"t":"teleport","sid":1}"#).is_err());
    assert!(serde_json::from_str::<ClientMsg>(r#"{"sid":1}"#).is_err());
}

#[test]
fn server_result_never_carries_route() {
    let v: Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("server_result.json")).unwrap())
            .unwrap();
    assert!(v["result"].get("route").is_none());
}

#[test]
fn proto_negotiation_table() {
    // (client_min, client_max, server_min, server_max) -> chosen
    let table = [
        ((1, 1, 1, 1), Some(1)),
        ((1, 3, 1, 1), Some(1)),
        ((1, 3, 1, 2), Some(2)),
        ((2, 3, 1, 1), None),
        ((1, 1, 2, 3), None),
        ((2, 2, 1, 3), Some(2)),
        ((3, 1, 1, 3), None),
    ];
    for ((a, b, c, d), want) in table {
        assert_eq!(negotiate(a, b, c, d), want, "{a}..{b} vs {c}..{d}");
    }
    assert_eq!(
        negotiate(PROTO_MIN, PROTO_MAX, PROTO_MIN, PROTO_MAX),
        Some(1)
    );
}

/// D22 compatibility, both directions. A new client's `start` with the
/// segment fields still parses where the receiver does not know them (no
/// `deny_unknown_fields` anywhere: shown with a field nobody knows), and
/// the pre-D22 fixtures (no `features`, no new timings) parse into the new
/// types with the defaults that mean "off".
#[test]
fn d22_fields_are_additive_both_ways() {
    let start = std::fs::read_to_string(dir().join("client_start_segmented.json")).unwrap();
    let mut v: Value = serde_json::from_str(&start).unwrap();
    v["config"]["some_future_field"] = serde_json::json!(1);
    let m: ClientMsg = serde_json::from_value(v).unwrap();
    let ClientMsg::Start { config, .. } = m else {
        panic!()
    };
    assert_eq!(config.segment_pause_ms, Some(800));
    // An old client's start: segmentation off.
    let old: ClientMsg =
        serde_json::from_str(&std::fs::read_to_string(dir().join("client_start.json")).unwrap())
            .unwrap();
    let ClientMsg::Start { config, .. } = old else {
        panic!()
    };
    assert_eq!(config.segment_pause_ms, None);
    // An old server's hello and result.
    let hello: ServerMsg =
        serde_json::from_str(&std::fs::read_to_string(dir().join("server_hello_ok.json")).unwrap())
            .unwrap();
    let ServerMsg::HelloOk(h) = hello else {
        panic!()
    };
    assert!(h.features.is_empty());
    let res: ServerMsg =
        serde_json::from_str(&std::fs::read_to_string(dir().join("server_result.json")).unwrap())
            .unwrap();
    let ServerMsg::Result { result, .. } = res else {
        panic!()
    };
    assert_eq!(
        (
            result.timings.wait_ms,
            result.timings.segments,
            result.timings.pre_asr_ms
        ),
        (0, 0, 0)
    );
    let old_json = std::fs::read_to_string(dir().join("server_result.json")).unwrap();
    assert!(!old_json.contains("wait_ms") && !old_json.contains("segments\": 3"));
}
