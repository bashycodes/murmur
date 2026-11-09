//! D64: the AI cleanup sees the dictionary terms a dictation plausibly
//! misspells, in the per-dictation user message only; a tidy dictation
//! with such a misspelling wakes the on-device model; the rules keep a
//! repeated word that is an entry.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use flow_core::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use flow_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use flow_core::config::SessionConfig;
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::FlowCore;

/// Reports a fixed English transcript and keeps the hotwords it was given.
struct Said(&'static str, Arc<Mutex<Vec<String>>>);

impl AsrEngine for Said {
    fn transcribe(&self, _pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        *self.1.lock().unwrap() = req.dictionary_terms.clone();
        Ok(AsrOutput {
            text: self.0.into(),
            language: Some("en".into()),
            ..Default::default()
        })
    }
}

/// Echoes its transcript and keeps every prompt; reports `engine`.
struct Seen {
    engine: &'static str,
    prompts: Mutex<Vec<CleanPrompt>>,
}

impl Cleaner for Seen {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.prompts.lock().unwrap().push(p.clone());
        Ok(p.transcript.clone())
    }
    fn engine(&self) -> &'static str {
        self.engine
    }
}

struct Run {
    prompts: Vec<CleanPrompt>,
    hotwords: Vec<String>,
    fallback_reason: Option<String>,
}

fn dictate(text: &'static str, engine: &'static str, cfg: SessionConfig) -> Run {
    let seen = Arc::new(Seen {
        engine,
        prompts: Mutex::new(Vec::new()),
    });
    let hot = Arc::new(Mutex::new(Vec::new()));
    let core = FlowCore::from_engines(Arc::new(Said(text, hot.clone())), Some(seen.clone()));
    core.start_session(cfg).unwrap();
    let tone: Vec<i16> = (0..16_000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    core.push_audio(&tone);
    let r = core.commit();
    let prompts = seen.prompts.lock().unwrap().clone();
    let hotwords = hot.lock().unwrap().clone();
    Run {
        prompts,
        hotwords,
        fallback_reason: r.fallback_reason,
    }
}

fn with_dict(words: &[&str]) -> SessionConfig {
    SessionConfig {
        languages: vec!["en".into()],
        dictionary: Dictionary {
            entries: words.iter().map(|w| DictEntry::word(w)).collect(),
        },
        ..Default::default()
    }
}

fn dictionary_line(p: &CleanPrompt) -> Option<&str> {
    p.user.lines().find_map(|l| l.strip_prefix("DICTIONARY: "))
}

#[test]
fn a_tidy_dictation_with_a_misspelled_entry_wakes_the_on_device_model() {
    let t = "The Zentroba invoice went out this morning.";
    let r = dictate(t, "local", with_dict(&["Zentrova", "Quillmate"]));
    assert_eq!(r.prompts.len(), 1, "{:?}", r.fallback_reason);
    assert_eq!(dictionary_line(&r.prompts[0]), Some("Zentrova"));
    // Without the entry the same line is "already clean".
    let r = dictate(t, "local", with_dict(&["Quillmate"]));
    assert!(r.prompts.is_empty());
    assert_eq!(r.fallback_reason.as_deref(), Some("llm_skipped_clean"));
}

#[test]
fn a_repeated_word_that_is_an_entry_reaches_the_on_device_model_whole() {
    let t = "Put the dentist in bop bop for Monday.";
    let r = dictate(t, "local", with_dict(&["BopBop"]));
    assert_eq!(r.prompts.len(), 1, "{:?}", r.fallback_reason);
    assert_eq!(dictionary_line(&r.prompts[0]), Some("BopBop"));
    assert!(
        r.prompts[0].transcript.contains("bop bop"),
        "{}",
        r.prompts[0].transcript
    );
    assert_eq!(r.hotwords, vec!["BopBop".to_string()]);
    // Without the entry it is a stutter, and the tidy line skips the model.
    let r = dictate(t, "local", with_dict(&["Quillmate"]));
    assert!(r.prompts.is_empty());
}

#[test]
fn ai_cleanup_names_the_entry_in_the_user_message_and_keeps_the_cached_prefix() {
    let t = "so um I sent the quill mate notes to the team";
    let with = dictate(t, "cloud", with_dict(&["Quillmate", "Marbleford"]));
    let without = dictate(t, "cloud", with_dict(&[]));
    let (a, b) = (&with.prompts[0], &without.prompts[0]);
    assert_eq!(dictionary_line(a), Some("Quillmate"));
    assert_eq!(dictionary_line(b), None);
    assert_eq!(a.system, b.system, "system prompt unchanged");
    assert_eq!(a.examples, b.examples, "few-shots unchanged");
    // The request a server hop rebuilds from carries the dictionary.
    let src = a.source.as_ref().unwrap();
    assert_eq!(src.dictionary.vocabulary(), ["Quillmate", "Marbleford"]);
}

#[test]
fn a_spelled_right_entry_adds_nothing_to_the_prompt() {
    let t = "so the Quillmate notes went out";
    let r = dictate(t, "cloud", with_dict(&["Quillmate"]));
    assert_eq!(dictionary_line(&r.prompts[0]), None);
}
