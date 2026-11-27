//! D119: language ID on a one-model core (Android keeps one ASR model in
//! RAM): the session stays on the loaded model and reports the language
//! whose model it lacks, while recording (poll) and at commit.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use murmur_core::asr::MockAsr;
use murmur_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use murmur_core::config::SessionConfig;
use murmur_core::lid::{self, LanguageId, MockLid, LID_SAMPLES};
use murmur_core::{CommitResult, MurmurCore};

fn speech(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

fn push(core: &MurmurCore, ms: usize) {
    for c in speech(ms).chunks(640) {
        core.push_audio(c);
    }
}

fn dictate_ms(core: &MurmurCore, cfg: SessionConfig, ms: usize) -> CommitResult {
    core.start_session(cfg).unwrap();
    push(core, ms);
    core.commit()
}

/// A one-model core holding `loaded` (`None` = the English default) with a
/// Hebrew route configured, and an identifier that hears `heard`.
fn resident(loaded: Option<&str>, heard: &str) -> (MurmurCore, Arc<MockAsr>, Arc<MockLid>) {
    let asr = Arc::new(MockAsr::new(if loaded.is_some() {
        "שלום"
    } else {
        "hello"
    }));
    let lid = Arc::new(MockLid::new(heard));
    let core = MurmurCore::from_resident_engine_with_lid(
        asr.clone(),
        loaded.map(String::from),
        vec!["he".into()],
        lid.clone(),
        None,
    );
    (core, asr, lid)
}

#[test]
fn hebrew_on_the_english_model_wants_hebrew() {
    let (core, asr, lid) = resident(None, "he");
    let out = dictate_ms(&core, SessionConfig::default(), 800);
    // The loaded model still decodes (the shell may not switch).
    assert_eq!(out.asr_text, "hello", "{out:?}");
    assert_eq!(out.language_wanted.as_deref(), Some("he"));
    assert_eq!((asr.calls(), lid.calls()), (1, 1));
    for (_, cands) in lid.heard() {
        assert_eq!(cands, ["en", "he"]);
    }
}

#[test]
fn english_on_the_hebrew_model_wants_the_default() {
    let (core, _, _) = resident(Some("he"), "en");
    let out = dictate_ms(&core, SessionConfig::default(), 800);
    assert_eq!(out.language_wanted.as_deref(), Some("en"), "{out:?}");
}

#[test]
fn the_held_language_wants_nothing() {
    for (loaded, heard) in [(None, "en"), (Some("he"), "he")] {
        let (core, _, _) = resident(loaded, heard);
        let out = dictate_ms(&core, SessionConfig::default(), 800);
        assert_eq!(out.language_wanted, None, "{loaded:?} {heard}: {out:?}");
        assert!(out.timings.lid_ms > 0);
    }
}

#[test]
fn a_short_dictation_is_identified_at_commit() {
    let (core, _, lid) = resident(None, "he");
    core.start_session(SessionConfig::default()).unwrap();
    push(&core, 1000);
    std::thread::sleep(Duration::from_millis(150));
    // Under 3 s of speech: nothing asked for while recording.
    assert_eq!(core.poll().language_wanted, None);
    assert_eq!(lid.calls(), 0);
    let out = core.commit();
    assert_eq!(out.language_wanted.as_deref(), Some("he"));
    assert_eq!(lid.calls(), 1);
}

#[test]
fn a_long_dictation_asks_while_recording() {
    let (core, _, _) = resident(None, "he");
    core.start_session(SessionConfig::default()).unwrap();
    push(&core, LID_SAMPLES / 16 + 500);
    let t = Instant::now();
    let mut seen = None;
    while t.elapsed() < Duration::from_secs(5) {
        seen = core.poll().language_wanted;
        if seen.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(seen.as_deref(), Some("he"));
    let out = core.commit();
    assert_eq!(out.language_wanted.as_deref(), Some("he"));
    // After the commit the poll no longer asks.
    assert_eq!(core.poll().language_wanted, None);
}

#[test]
fn a_forced_language_skips_language_id() {
    let (core, _, lid) = resident(None, "he");
    let cfg = SessionConfig {
        languages: vec!["en".into()],
        ..Default::default()
    };
    let out = dictate_ms(&core, cfg, 800);
    assert_eq!(out.language_wanted, None);
    assert_eq!(lid.calls(), 0);
}

#[test]
fn a_core_without_an_identifier_never_asks() {
    let core = MurmurCore::from_engines(Arc::new(MockAsr::new("hello")), None);
    let out = dictate_ms(&core, SessionConfig::default(), 800);
    assert_eq!(out.language_wanted, None);
    let json = serde_json::to_value(&out).unwrap();
    assert!(json.get("language_wanted").is_none(), "{json}");
}

/// An identifier with fixed scores for every window.
struct ScoredLid(Vec<(&'static str, f32)>);

impl LanguageId for ScoredLid {
    fn scores(&self, _pcm: &[f32], candidates: &[String]) -> Result<lid::LidScores, String> {
        Ok(self
            .0
            .iter()
            .filter(|(l, _)| candidates.iter().any(|c| c == l))
            .map(|(l, p)| (l.to_string(), *p))
            .collect())
    }
}

fn scored(loaded: Option<&str>, scores: Vec<(&'static str, f32)>) -> MurmurCore {
    MurmurCore::from_resident_engine_with_lid(
        Arc::new(MockAsr::new("hello")),
        loaded.map(String::from),
        vec!["he".into()],
        Arc::new(ScoredLid(scores)),
        None,
    )
}

/// Hebrew above the routing floor but not the most likely: the server
/// would route it, but on the phone an uncertain answer must not cost a
/// reload.
#[test]
fn an_uncertain_answer_asks_for_nothing() {
    let core = scored(None, vec![("en", 0.7), ("he", 0.3)]);
    core.start_session(SessionConfig::default()).unwrap();
    push(&core, LID_SAMPLES / 16 + 500);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(core.poll().language_wanted, None);
    let out = core.commit();
    assert_eq!(out.language_wanted, None, "{out:?}");
    assert_eq!(out.asr_text, "hello");
}

/// Not Hebrew (English, or a language without its own model) heard surely
/// on the Hebrew model: the default model is wanted.
#[test]
fn a_sure_non_hebrew_answer_on_the_hebrew_model_wants_the_default() {
    let core = scored(Some("he"), vec![("en", 0.96), ("he", 0.04)]);
    let out = dictate_ms(&core, SessionConfig::default(), 800);
    assert_eq!(out.language_wanted.as_deref(), Some("en"), "{out:?}");
    // Hebrew still the likelier: the Hebrew model stays.
    let core = scored(Some("he"), vec![("en", 0.45), ("he", 0.55)]);
    assert_eq!(
        dictate_ms(&core, SessionConfig::default(), 800).language_wanted,
        None
    );
}

struct CountingCleaner(AtomicUsize);

impl Cleaner for CountingCleaner {
    fn clean(&self, p: &CleanPrompt, _d: Instant) -> Result<String, CleanError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(p.transcript.clone())
    }
}

/// The wrong model's text is re-run by the shell: no AI cleanup is spent on it.
#[test]
fn text_from_the_wrong_model_skips_ai_cleanup() {
    for (heard, cleans) in [("en", 1), ("he", 0)] {
        let cleaner = Arc::new(CountingCleaner(AtomicUsize::new(0)));
        let core = MurmurCore::from_resident_engine_with_lid(
            Arc::new(MockAsr::new("hello there how are you today")),
            None,
            vec!["he".into()],
            Arc::new(MockLid::new(heard)),
            Some(cleaner.clone()),
        );
        let out = dictate_ms(&core, SessionConfig::default(), 800);
        assert_eq!(cleaner.0.load(Ordering::SeqCst), cleans, "{heard}: {out:?}");
    }
}
