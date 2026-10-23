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

/// Why the core asks for a transcript (cloud engines refuse partials).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AsrPurpose {
    /// The utterance at commit.
    #[default]
    Final,
    /// A best-effort live preview while recording.
    Partial,
    /// A finished piece of a pause-segmented dictation (D22), decoded while
    /// the user keeps talking or at commit. Its text is part of the result,
    /// so engines treat it like `Final`.
    Segment,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsrRequest {
    pub initial_prompt: String,
    /// `None` = auto-detect.
    pub language: Option<String>,
    pub abort: AbortFlag,
    /// Dictionary vocabulary only (no snippets), trimmed, deduplicated
    /// case-insensitively, original order. For engines with a structured
    /// biasing field (cloud keyterms); local engines use `initial_prompt`.
    pub dictionary_terms: Vec<String>,
    /// On-screen words, same rules (minus anything already in
    /// `dictionary_terms`). Cloud engines send them only when the user
    /// opted in.
    pub screen_terms: Vec<String>,
    pub purpose: AsrPurpose,
    /// D22: the last words of the pieces before this one. Whisper appends
    /// it to the prompt after the dictionary terms (see [`compose_prompt`])
    /// so casing and punctuation carry across segments; other engines
    /// ignore it. `None` for the first piece and outside segmentation.
    pub prior_text: Option<String>,
}

/// Words of earlier pieces passed as `prior_text`.
pub const PRIOR_WORDS: usize = 30;
/// No `prior_text` when the dictionary/screen prompt already has this many
/// words (whisper keeps only the last ~224 prompt tokens, so the
/// dictionary terms, which come first, would be lost).
pub const PRIOR_MAX_PROMPT_WORDS: usize = 120;

/// The whisper prompt for a request: `initial_prompt`, then `prior_text`
/// when there is room for it.
pub fn compose_prompt(initial_prompt: &str, prior_text: Option<&str>) -> String {
    let prior = prior_text.map(str::trim).unwrap_or("");
    if prior.is_empty() || initial_prompt.split_whitespace().count() > PRIOR_MAX_PROMPT_WORDS {
        return initial_prompt.to_string();
    }
    if initial_prompt.is_empty() {
        prior.to_string()
    } else {
        format!("{initial_prompt} {prior}")
    }
}

/// The last [`PRIOR_WORDS`] words of `text` (`None` when empty).
pub fn prior_words(text: &str) -> Option<String> {
    let w: Vec<&str> = text.split_whitespace().collect();
    (!w.is_empty()).then(|| w[w.len().saturating_sub(PRIOR_WORDS)..].join(" "))
}

/// Trimmed, non-empty, case-insensitively deduplicated terms in order;
/// terms already in `seen` (lowercased) are skipped and added to it.
pub fn dedupe_terms<'a>(
    terms: impl IntoIterator<Item = &'a str>,
    seen: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for t in terms {
        let t = t.trim();
        let k = t.to_lowercase();
        if t.is_empty() || seen.contains(&k) {
            continue;
        }
        seen.push(k);
        out.push(t.to_string());
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AsrOutput {
    pub text: String,
    pub language: Option<String>,
    /// Highest per-segment no-speech probability, when the engine knows it
    /// (Whisper). Feeds the hallucination guard.
    pub no_speech_prob: Option<f32>,
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

    /// Whether the core may decode pieces of a dictation before commit
    /// (D22 `segment_pause_ms`). Engines that must not see audio before
    /// commit (cloud STT, D18: no upload before release, one billed call)
    /// return false and the core ignores segmentation. A future streaming
    /// engine (proposal Phase 4) would also return false: it decodes as
    /// audio arrives and needs no pause cuts.
    fn segmentable(&self) -> bool {
        true
    }
}

/// Test engine whose text is a function of the audio: one word `w<k>` per
/// burst of sound (20 ms frames with RMS ≥ 0.01; bursts closer than 300 ms
/// merge), `k` = the burst's peak amplitude in units of 1000/32768
/// (rounded). Any split of the audio at a quiet point gives the same words
/// in the same order, so segmented and whole decodes can be compared.
/// `with_rtf` simulates decode cost proportional to the audio length.
#[derive(Default)]
pub struct BurstEchoAsr {
    rtf: f32,
    calls: std::sync::atomic::AtomicUsize,
    requests: Mutex<Vec<(usize, AsrRequest)>>,
}

impl BurstEchoAsr {
    pub fn new() -> Self {
        Self::default()
    }
    /// Sleep `rtf × audio length` per decode (abortable).
    pub fn with_rtf(mut self, rtf: f32) -> Self {
        self.rtf = rtf;
        self
    }
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    /// `(samples, request)` of every decode, in order.
    pub fn requests(&self) -> Vec<(usize, AsrRequest)> {
        self.requests.lock().unwrap().clone()
    }
    /// The words for `pcm` (no delay, not recorded).
    pub fn words(pcm: &[f32]) -> String {
        const F: usize = 320;
        let mut words = Vec::new();
        let (mut peak, mut gap, mut in_burst) = (0f32, 0usize, false);
        let flush = |peak: f32, words: &mut Vec<String>| {
            let k = (peak * 32768.0 / 1000.0).round() as u32;
            if k > 0 {
                words.push(format!("w{k}"));
            }
        };
        for f in pcm.chunks(F) {
            let rms = (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt();
            if rms >= 0.01 {
                in_burst = true;
                gap = 0;
                peak = f.iter().fold(peak, |m, s| m.max(s.abs()));
            } else if in_burst {
                gap += 1;
                if gap * F >= 4800 {
                    flush(peak, &mut words);
                    (peak, in_burst) = (0.0, false);
                }
            }
        }
        if in_burst {
            flush(peak, &mut words);
        }
        words.join(" ")
    }
}

impl AsrEngine for BurstEchoAsr {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().unwrap().push((pcm.len(), req.clone()));
        let d = std::time::Duration::from_secs_f32(
            self.rtf * pcm.len() as f32 / crate::config::SAMPLE_RATE as f32,
        );
        let end = std::time::Instant::now() + d;
        while std::time::Instant::now() < end {
            if req.abort.is_set() {
                return Err(AsrError::Transcribe("aborted".into()));
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        Ok(AsrOutput {
            text: Self::words(pcm),
            language: Some("en".into()),
            no_speech_prob: None,
        })
    }
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
    last_len: std::sync::atomic::AtomicUsize,
    delay: std::time::Duration,
}

impl MockAsr {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            calls: Mutex::new((0, None)),
            last_len: Default::default(),
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
    /// Samples passed to the most recent `transcribe`.
    pub fn last_len(&self) -> usize {
        self.last_len.load(Ordering::Relaxed)
    }
}

impl AsrEngine for MockAsr {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        {
            let mut c = self.calls.lock().unwrap();
            c.0 += 1;
            c.1 = Some(req.clone());
        }
        self.last_len.store(pcm.len(), Ordering::Relaxed);
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
            no_speech_prob: None,
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
    fn dedupe_terms_keeps_order_and_skips_seen() {
        let mut seen = Vec::new();
        let d = dedupe_terms(["Kubernetes", " wispr ", "", "kubernetes"], &mut seen);
        assert_eq!(d, ["Kubernetes", "wispr"]);
        let s = dedupe_terms(["Alice", "WISPR"], &mut seen);
        assert_eq!(s, ["Alice"]);
    }

    #[test]
    fn prompt_carries_prior_words_after_the_dictionary() {
        assert_eq!(compose_prompt("Wispr.", None), "Wispr.");
        assert_eq!(compose_prompt("Wispr.", Some(" ")), "Wispr.");
        assert_eq!(compose_prompt("", Some("and then")), "and then");
        assert_eq!(
            compose_prompt("Wispr.", Some("and then")),
            "Wispr. and then"
        );
        let big: String = (0..150).map(|i| format!("w{i}, ")).collect();
        assert_eq!(compose_prompt(&big, Some("x")), big);
        let long: String = (0..40).map(|i| format!("t{i} ")).collect();
        let p = prior_words(&long).unwrap();
        assert_eq!(p.split(' ').count(), PRIOR_WORDS);
        assert!(p.starts_with("t10 ") && p.ends_with("t39"));
        assert_eq!(prior_words("  "), None);
    }

    #[test]
    fn burst_echo_words_survive_any_quiet_split() {
        let tone = |amp: f32, n: usize| -> Vec<f32> {
            (0..n).map(|i| amp * (i as f32 * 0.05).sin()).collect()
        };
        let mut pcm = Vec::new();
        for k in [3.0, 7.0, 12.0] {
            pcm.extend(tone(k * 1000.0 / 32768.0, 16_000));
            pcm.extend(vec![0.0; 16_000]);
        }
        assert_eq!(BurstEchoAsr::words(&pcm), "w3 w7 w12");
        let (a, b) = pcm.split_at(16_000 * 3 + 8000);
        assert_eq!(BurstEchoAsr::words(a), "w3 w7");
        assert_eq!(BurstEchoAsr::words(b), "w12");
        assert_eq!(BurstEchoAsr::words(&[0.0; 1000]), "");
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
