//! D33: per-language ASR routing and language-gated post-processing,
//! against MockAsr engines (no model needed).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::asr::MockAsr;
use flow_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use flow_core::config::{AsrBackend, CleanupLevel, LanguageModel, ModelConfig, SessionConfig};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::load_timings::{LoadBackend, ModelKind};
use flow_core::{CommitResult, CommitStatus, FlowCore};

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn dictate(core: &FlowCore, cfg: SessionConfig) -> CommitResult {
    core.start_session(cfg).unwrap();
    for c in speech(800).chunks(640) {
        core.push_audio(c);
    }
    core.commit()
}

fn lang(l: &str) -> SessionConfig {
    SessionConfig {
        languages: vec![l.into()],
        ..Default::default()
    }
}

fn routed(en: &str, he: &str) -> (FlowCore, Arc<MockAsr>, Arc<MockAsr>) {
    let (d, h) = (Arc::new(MockAsr::new(en)), Arc::new(MockAsr::new(he)));
    let core = FlowCore::from_language_engines(d.clone(), vec![("he".into(), h.clone())], None);
    (core, d, h)
}

#[test]
fn forced_hebrew_decodes_on_the_hebrew_engine() {
    let (core, en, he) = routed("hello there", "שלום לכולם");
    for l in ["he", "he-IL", "iw"] {
        let r = dictate(&core, lang(l));
        assert_eq!(r.asr_text, "שלום לכולם", "{l}: {r:?}");
        assert_eq!(r.language.as_deref(), Some(l), "{l}");
    }
    assert_eq!((en.calls(), he.calls()), (0, 3));
}

#[test]
fn auto_english_and_other_languages_use_the_default_engine() {
    let (core, en, he) = routed("hello there", "שלום");
    for cfg in [
        SessionConfig::default(),
        lang("auto"),
        lang("en"),
        lang("de"),
        SessionConfig {
            languages: vec!["en".into(), "he".into()],
            ..Default::default()
        },
    ] {
        let r = dictate(&core, cfg.clone());
        assert_eq!(r.asr_text, "hello there", "{cfg:?}");
    }
    assert_eq!((en.calls(), he.calls()), (5, 0));
}

#[test]
fn english_result_is_byte_identical_with_a_language_map() {
    let text =
        "um so the meeting is at three thirty comma right new line thanks scratch that thank you";
    let single = FlowCore::from_engines(Arc::new(MockAsr::new(text)), None);
    let (core, _, _) = routed(text, "שלום");
    for cfg in [SessionConfig::default(), lang("en")] {
        let mut a = dictate(&single, cfg.clone());
        let mut b = dictate(&core, cfg);
        a.timings = Default::default();
        b.timings = Default::default();
        assert_eq!(a, b);
        assert_ne!(a.final_text, text, "English rules ran: {a:?}");
    }
}

#[test]
fn hebrew_skips_english_rules_but_keeps_tidy_and_closing_period() {
    // English rules would drop "um", turn "comma" into ",", "twenty" into
    // "20" and drop the clause before "scratch that".
    let raw = "  שלום   um מה comma twenty נשמע scratch that היום ,";
    let (core, _, _) = routed("x", raw);
    let r = dictate(&core, lang("he"));
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    // (The trailing space after a sentence end is the cursor-fitting rule, as in English.)
    assert_eq!(
        r.final_text,
        "שלום um מה comma twenty נשמע scratch that היום. "
    );
    // Short Hebrew stays as spoken (close_sentence's 4-word minimum).
    let (core, _, _) = routed("x", "שלום  לכולם");
    assert_eq!(dictate(&core, lang("he")).final_text, "שלום לכולם ");
}

#[test]
fn hebrew_keeps_dictionary_and_press_enter() {
    let (core, _, _) = routed("x", "שלחתי את זה ב וויספר press enter");
    let r = dictate(
        &core,
        SessionConfig {
            dictionary: Dictionary {
                entries: vec![DictEntry::replacement("וויספר", "Wispr")],
            },
            ..lang("he")
        },
    );
    assert!(r.press_enter, "{r:?}");
    assert_eq!(r.final_text, "שלחתי את זה ב Wispr.");
}

/// Records the prompt; answers with a fixed text.
struct Recording(Mutex<Option<CleanPrompt>>, &'static str);
impl Cleaner for Recording {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        *self.0.lock().unwrap() = Some(p.clone());
        Ok(self.1.into())
    }
}

#[test]
fn hebrew_llm_prompt_names_the_language_and_guard_accepts_hebrew() {
    let raw = "אני רוצה לקבוע פגישה מחר בבוקר עם דני";
    let cl = Arc::new(Recording(
        Mutex::new(None),
        "אני רוצה לקבוע פגישה מחר בבוקר עם דני.",
    ));
    let core = FlowCore::from_language_engines(
        Arc::new(MockAsr::new("hello")),
        vec![("he".into(), Arc::new(MockAsr::new(raw)))],
        Some(cl.clone()),
    );
    let r = dictate(
        &core,
        SessionConfig {
            cleanup_level: CleanupLevel::Medium,
            llm_skip_clean: false,
            llm_prefill: false,
            ..lang("he")
        },
    );
    assert_eq!(r.status, CommitStatus::Formatted, "{r:?}");
    assert_eq!(r.final_text, "אני רוצה לקבוע פגישה מחר בבוקר עם דני. ");
    let p = cl.0.lock().unwrap().clone().unwrap();
    assert!(p.user.contains("LANGUAGE: Hebrew"), "{}", p.user);

    // English dictations get no LANGUAGE line.
    let r = dictate(
        &core,
        SessionConfig {
            cleanup_level: CleanupLevel::Medium,
            llm_skip_clean: false,
            llm_prefill: false,
            ..lang("en")
        },
    );
    assert_ne!(r.status, CommitStatus::Error);
    let p = cl.0.lock().unwrap().clone().unwrap();
    assert!(!p.user.contains("LANGUAGE:"), "{}", p.user);
}

fn mock_config() -> ModelConfig {
    ModelConfig {
        asr_backend: AsrBackend::Mock {
            text: "english words".into(),
        },
        languages: BTreeMap::from([(
            "he".to_string(),
            LanguageModel {
                asr_backend: AsrBackend::Mock {
                    text: "מילים בעברית".into(),
                },
                ..Default::default()
            },
        )]),
        ..Default::default()
    }
}

#[test]
fn load_models_routes_by_the_config_map() {
    let core = FlowCore::load_models(&mock_config()).unwrap();
    assert_eq!(dictate(&core, lang("he")).asr_text, "מילים בעברית");
    assert_eq!(dictate(&core, lang("en")).asr_text, "english words");
    assert_eq!(
        dictate(&core, SessionConfig::default()).asr_text,
        "english words"
    );
}

#[test]
fn loads_report_each_model_they_loaded() {
    let cfg = mock_config();
    let he = FlowCore::load_models_for(&cfg, Some("he")).unwrap();
    let t = he.load_timings();
    assert_eq!(t.models.len(), 1, "{t:?}");
    assert_eq!(
        (t.models[0].kind, t.models[0].backend),
        (ModelKind::Asr, LoadBackend::Mock)
    );
    assert_eq!(t.models[0].language.as_deref(), Some("he"));
    assert!(t.llm().is_none());
    let all = FlowCore::load_models(&cfg).unwrap();
    let langs: Vec<_> = all
        .load_timings()
        .models
        .iter()
        .map(|m| m.language.as_deref())
        .collect();
    assert_eq!(langs, [Some("he"), None]);
    assert!(all.load_timings().to_json().starts_with("{\"total_ms\":"));
}

#[test]
fn load_models_for_keeps_one_model() {
    let cfg = mock_config();
    // Hebrew only: every session decodes on it until the shell reloads.
    let he = FlowCore::load_models_for(&cfg, Some("he")).unwrap();
    assert_eq!(dictate(&he, lang("he")).asr_text, "מילים בעברית");
    assert_eq!(dictate(&he, lang("en")).asr_text, "מילים בעברית");
    for l in [None, Some("en"), Some("auto"), Some("de")] {
        let core = FlowCore::load_models_for(&cfg, l).unwrap();
        assert_eq!(
            dictate(&core, lang("en")).asr_text,
            "english words",
            "{l:?}"
        );
        assert_eq!(cfg.language_route(l), None);
    }
    assert_eq!(cfg.language_route(Some("he-IL")), Some("he"));
}
