//! H6 / D34: auto-detect sessions identify their language and take the
//! language route, against MockAsr engines and a MockLid (no model).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use flow_core::asr::MockAsr;
use flow_core::config::{AsrBackend, LanguageModel, ModelConfig, SessionConfig};
use flow_core::lid::{MockLid, LID_SAMPLES};
use flow_core::{CommitResult, CoreError, FlowCore};

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn dictate_ms(core: &FlowCore, cfg: SessionConfig, ms: usize) -> CommitResult {
    core.start_session(cfg).unwrap();
    for c in speech(ms).chunks(640) {
        core.push_audio(c);
    }
    core.commit()
}

fn dictate(core: &FlowCore, cfg: SessionConfig) -> CommitResult {
    dictate_ms(core, cfg, 800)
}

fn lang(l: &str) -> SessionConfig {
    SessionConfig {
        languages: vec![l.into()],
        ..Default::default()
    }
}

struct Rig {
    core: FlowCore,
    en: Arc<MockAsr>,
    he: Arc<MockAsr>,
    lid: Arc<MockLid>,
}

fn rig(heard: &str) -> Rig {
    let (en, he) = (
        Arc::new(MockAsr::new("hello there")),
        Arc::new(MockAsr::new("שלום לכולם")),
    );
    let lid = Arc::new(MockLid::new(heard));
    let core = FlowCore::from_language_engines_with_lid(
        en.clone(),
        vec![("he".into(), he.clone())],
        Some(lid.clone()),
        None,
    );
    Rig { core, en, he, lid }
}

#[test]
fn auto_hebrew_speech_routes_to_the_hebrew_engine() {
    let r = rig("he");
    for cfg in [SessionConfig::default(), lang("auto")] {
        let out = dictate(&r.core, cfg);
        assert_eq!(out.asr_text, "שלום לכולם", "{out:?}");
        assert_eq!(out.language.as_deref(), Some("he"));
        assert!(out.timings.lid_ms > 0, "{:?}", out.timings);
    }
    assert_eq!((r.en.calls(), r.he.calls(), r.lid.calls()), (0, 2, 2));
    // A short dictation is identified from all of its speech, at commit,
    // between English (the default) and the routes.
    for (n, cands) in r.lid.heard() {
        assert!(n > 0 && n <= 800 * 16, "{n}");
        assert_eq!(cands, ["en", "he"]);
    }
}

#[test]
fn auto_english_speech_stays_on_the_default_engine() {
    let r = rig("en");
    let out = dictate(&r.core, SessionConfig::default());
    assert_eq!(out.asr_text, "hello there");
    assert_eq!((r.en.calls(), r.he.calls(), r.lid.calls()), (1, 0, 1));
}

#[test]
fn english_auto_result_is_byte_identical_apart_from_lid_ms() {
    let text = "um so the meeting is at three thirty comma right new line thanks";
    let plain = FlowCore::from_language_engines(
        Arc::new(MockAsr::new(text)),
        vec![("he".into(), Arc::new(MockAsr::new("שלום")))],
        None,
    );
    let lid = FlowCore::from_language_engines_with_lid(
        Arc::new(MockAsr::new(text)),
        vec![("he".into(), Arc::new(MockAsr::new("שלום")))],
        Some(Arc::new(MockLid::new("en"))),
        None,
    );
    let mut a = dictate(&plain, SessionConfig::default());
    let mut b = dictate(&lid, SessionConfig::default());
    assert_eq!(a.timings.lid_ms, 0);
    assert!(b.timings.lid_ms > 0);
    a.timings = Default::default();
    b.timings = Default::default();
    assert_eq!(a, b);
}

#[test]
fn forced_languages_skip_language_id() {
    let r = rig("he");
    assert_eq!(dictate(&r.core, lang("en")).asr_text, "hello there");
    assert_eq!(dictate(&r.core, lang("he-IL")).asr_text, "שלום לכולם");
    let out = dictate(&r.core, lang("de"));
    assert_eq!(out.asr_text, "hello there");
    assert_eq!(out.timings.lid_ms, 0);
    assert_eq!(r.lid.calls(), 0);
}

#[test]
fn no_language_engines_means_no_language_id() {
    let lid = Arc::new(MockLid::new("he"));
    let core = FlowCore::from_language_engines_with_lid(
        Arc::new(MockAsr::new("hello there")),
        Vec::new(),
        Some(lid.clone()),
        None,
    );
    let out = dictate(&core, SessionConfig::default());
    assert_eq!(out.asr_text, "hello there");
    assert_eq!((lid.calls(), out.timings.lid_ms), (0, 0));
}

#[test]
fn a_listed_language_set_is_what_the_identifier_chooses_from() {
    let r = rig("he");
    let out = dictate(
        &r.core,
        SessionConfig {
            languages: vec!["en-US".into(), "iw".into(), "fr".into()],
            ..Default::default()
        },
    );
    assert_eq!(out.asr_text, "שלום לכולם");
    assert_eq!(r.lid.heard()[0].1, ["en", "he", "fr"]);
}

#[test]
fn a_long_dictation_is_identified_while_recording_from_its_first_seconds() {
    let r = rig("he");
    r.core.start_session(SessionConfig::default()).unwrap();
    for c in speech(5000).chunks(640) {
        r.core.push_audio(c);
    }
    let t = std::time::Instant::now();
    while r.lid.calls() == 0 && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(r.lid.calls(), 1, "identified before commit");
    let out = r.core.commit();
    assert_eq!(out.asr_text, "שלום לכולם");
    assert_eq!(r.lid.calls(), 1);
    assert_eq!(r.lid.heard()[0].0, LID_SAMPLES);
    assert_eq!(
        r.en.calls(),
        0,
        "nothing decoded on English before the switch"
    );
}

#[test]
fn silence_is_not_identified() {
    let r = rig("he");
    r.core.start_session(SessionConfig::default()).unwrap();
    r.core.push_audio(&vec![0i16; 16_000]);
    let out = r.core.commit();
    assert_eq!(out.empty_reason.as_deref(), Some("no_speech"), "{out:?}");
    assert_eq!(r.lid.calls(), 0);
}

fn mock_config(lid_path: Option<&str>, with_he: bool) -> ModelConfig {
    let mut languages = BTreeMap::new();
    if with_he {
        languages.insert(
            "he".to_string(),
            LanguageModel {
                asr_backend: AsrBackend::Mock {
                    text: "מילים".into(),
                },
                ..Default::default()
            },
        );
    }
    ModelConfig {
        asr_backend: AsrBackend::Mock {
            text: "english words".into(),
        },
        languages,
        lid_path: lid_path.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn lid_path_is_ignored_without_language_models_and_omitted_when_unset() {
    // Not loaded (not even looked for) without a language model.
    let core = FlowCore::load_models(&mock_config(Some("/nonexistent/ggml-tiny.bin"), false));
    assert!(core.is_ok());
    let j = serde_json::to_value(mock_config(None, true)).unwrap();
    assert!(j.get("lid_path").is_none(), "{j}");
    let c: ModelConfig = serde_json::from_str(r#"{"lid_path":"x.bin"}"#).unwrap();
    assert_eq!(c.lid_path.as_deref(), Some("x.bin"));
    assert_eq!(c.lid_model(), None);
}

#[test]
fn a_missing_lid_model_is_a_load_error() {
    let e = FlowCore::load_models(&mock_config(Some("/nonexistent/ggml-tiny.bin"), true));
    match e {
        #[cfg(feature = "whisper")]
        Err(CoreError::ModelNotFound { path }) => assert_eq!(path, "/nonexistent/ggml-tiny.bin"),
        #[cfg(not(feature = "whisper"))]
        Err(CoreError::Load(m)) => assert!(m.contains("whisper"), "{m}"),
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn segmented_dictation_holds_its_pieces_until_identified() {
    let r = rig("he");
    r.core
        .start_session(SessionConfig {
            segment_pause_ms: Some(400),
            segment_min_ms: Some(500),
            ..Default::default()
        })
        .unwrap();
    // Speech, a pause the segmenter cuts at, more speech.
    let mut pcm = speech(1500);
    pcm.extend(std::iter::repeat_n(0i16, 16 * 900));
    pcm.extend(speech(3000));
    for c in pcm.chunks(640) {
        r.core.push_audio(c);
        std::thread::sleep(Duration::from_millis(2));
    }
    let out = r.core.commit();
    assert!(out.asr_text.contains("שלום לכולם"), "{out:?}");
    assert!(out.timings.segments >= 2, "{:?}", out.timings);
    assert_eq!(r.en.calls(), 0, "no piece decoded on English");
    assert_eq!(r.lid.calls(), 1);
}
