//! ASR engine abstraction + Whisper initial-prompt builder.
//!
//! Engines take the whole utterance (16 kHz mono f32) and return text; the
//! session worker owns buffering. Bias replaces Wispr's server-side
//! DynamicVocabulary with Whisper's `initial_prompt`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::dictionary::Dictionary;

/// Whisper's prompt window is ~224 tokens; 200 words is Wispr's dictionary cap.
pub const MAX_PROMPT_WORDS: usize = 200;

/// Cooperative cancellation for best-effort decodes (live partials): the
/// engine should stop as soon as it observes `true`.
#[derive(Debug, Clone, Default)]
pub struct AbortFlag(pub Option<Arc<AtomicBool>>);

impl AbortFlag {
    pub fn is_set(&self) -> bool {
        self.0.as_ref().is_some_and(|f| f.load(Ordering::Relaxed))
    }
}

impl PartialEq for AbortFlag {
    fn eq(&self, o: &Self) -> bool {
        match (&self.0, &o.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}
impl Eq for AbortFlag {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsrRequest {
    pub initial_prompt: String,
    /// `None` = auto-detect.
    pub language: Option<String>,
    pub abort: AbortFlag,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsrOutput {
    pub text: String,
    pub language: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error("model load failed: {0}")]
    Load(String),
    #[error("transcription failed: {0}")]
    Transcribe(String),
}

pub trait AsrEngine: Send + Sync {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError>;
}

/// Comma-separated vocabulary: dictionary first (user intent), then screen
/// words; case-insensitive dedupe; capped at [`MAX_PROMPT_WORDS`].
pub fn build_initial_prompt(dict: &Dictionary, screen_words: &[String]) -> String {
    let mut seen: Vec<String> = Vec::new();
    let mut phrases: Vec<&str> = Vec::new();
    let mut words = 0;
    let candidates = dict
        .vocabulary()
        .into_iter()
        .chain(screen_words.iter().map(String::as_str));
    for p in candidates {
        let p = p.trim();
        if p.is_empty() || seen.contains(&p.to_lowercase()) {
            continue;
        }
        let n = p.split_whitespace().count();
        if words + n > MAX_PROMPT_WORDS {
            break;
        }
        words += n;
        seen.push(p.to_lowercase());
        phrases.push(p);
    }
    if phrases.is_empty() {
        String::new()
    } else {
        format!("{}.", phrases.join(", "))
    }
}

/// Deterministic engine for tests and shell development.
pub struct MockAsr {
    text: String,
    calls: Mutex<(usize, Option<AsrRequest>)>,
    delay: std::time::Duration,
}

impl MockAsr {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            calls: Mutex::new((0, None)),
            delay: Default::default(),
        }
    }
    /// Simulate inference latency.
    pub fn with_delay(mut self, d: std::time::Duration) -> Self {
        self.delay = d;
        self
    }
    pub fn calls(&self) -> usize {
        self.calls.lock().unwrap().0
    }
    pub fn last_request(&self) -> Option<AsrRequest> {
        self.calls.lock().unwrap().1.clone()
    }
}

impl AsrEngine for MockAsr {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        {
            let mut c = self.calls.lock().unwrap();
            c.0 += 1;
            c.1 = Some(req.clone());
        }
        let end = std::time::Instant::now() + self.delay;
        while std::time::Instant::now() < end {
            if req.abort.is_set() {
                return Err(AsrError::Transcribe("aborted".into()));
            }
            std::thread::sleep(std::time::Duration::from_millis(5).min(self.delay));
        }
        let silent = pcm.iter().all(|s| s.abs() < 1e-4);
        Ok(AsrOutput {
            text: if silent {
                String::new()
            } else {
                self.text.clone()
            },
            language: Some(req.language.clone().unwrap_or_else(|| "en".into())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::{DictEntry, Dictionary};

    #[test]
    fn initial_prompt_lists_dictionary_then_screen_words() {
        let d = Dictionary {
            entries: vec![
                DictEntry::word("Kubernetes"),
                DictEntry::replacement("wispr", "Wispr"),
            ],
        };
        let p = build_initial_prompt(&d, &["Alice".into(), "Wispr".into()]);
        assert_eq!(p, "Kubernetes, Wispr, Alice.");
    }

    #[test]
    fn initial_prompt_empty_when_nothing_known() {
        assert_eq!(build_initial_prompt(&Dictionary::default(), &[]), "");
    }

    #[test]
    fn initial_prompt_caps_word_count() {
        let screen: Vec<String> = (0..500).map(|i| format!("w{i}")).collect();
        let p = build_initial_prompt(&Dictionary::default(), &screen);
        assert_eq!(p.split_whitespace().count(), MAX_PROMPT_WORDS);
    }

    #[test]
    fn mock_returns_text_for_audio_and_empty_for_silence() {
        let m = MockAsr::new("hello world");
        let out = m.transcribe(&[0.1; 1600], &AsrRequest::default()).unwrap();
        assert_eq!(out.text, "hello world");
        assert_eq!(m.calls(), 1);
        let silent = m.transcribe(&[0.0; 1600], &AsrRequest::default()).unwrap();
        assert_eq!(silent.text, "");
    }

    #[test]
    fn mock_records_last_request() {
        let m = MockAsr::new("x");
        let req = AsrRequest {
            initial_prompt: "Wispr.".into(),
            language: Some("de".into()),
            ..Default::default()
        };
        m.transcribe(&[0.5; 10], &req).unwrap();
        assert_eq!(m.last_request().unwrap(), req);
    }
}
