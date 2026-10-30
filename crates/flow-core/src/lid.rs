//! Spoken language identification (H6, D34): which language is this
//! dictation in, so an auto-detect session can route to a per-language
//! model (D33) before decoding.
//!
//! The core asks only between a few candidates (English, the default
//! model's language, plus the `ModelConfig::languages` keys), never all of
//! whisper's 99: Hebrew speech scored against everything is often
//! "Arabic" or "Yiddish", which would route nowhere.
//!
//! Backend: whisper.cpp's language detection ([`WhisperLid`], feature
//! `whisper`) on a small multilingual model (`ggml-tiny`), with the encoder
//! window shrunk to the first [`LID_SAMPLES`] of speech. Built into every
//! shell already (server, Windows, Android); see D34 for the numbers.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::config::SAMPLE_RATE;

/// Audio the identifier hears: the first 3 s of speech (or the whole
/// utterance when shorter).
pub const LID_SAMPLES: usize = 3 * SAMPLE_RATE;

/// Least probability (among the candidates) at which the core routes a
/// dictation to a language model instead of the default one. Low on
/// purpose: the identifier errs towards English (noise, short clips) and
/// never gave English speech more than 0.02 for Hebrew (D34), while a
/// Hebrew clip often scores only 0.1–0.5.
pub const ROUTE_MIN_PROB: f32 = 0.1;

/// The identifier's answer: `(candidate, probability)` for every candidate
/// it knows, probabilities summing to 1 over them.
pub type LidScores = Vec<(String, f32)>;

/// A spoken-language identifier.
pub trait LanguageId: Send + Sync {
    /// Score `candidates` (ISO 639-1 codes) for 16 kHz mono `pcm` (callers
    /// pass at most [`LID_SAMPLES`] of speech).
    fn scores(&self, pcm: &[f32], candidates: &[String]) -> Result<LidScores, String>;
}

/// The language a dictation routes to: the most likely candidate other
/// than `default` when it reaches [`ROUTE_MIN_PROB`], else `None` (the
/// default model).
pub fn choose(scores: &[(String, f32)], default: &str) -> Option<String> {
    scores
        .iter()
        .filter(|(l, p)| l != default && *p >= ROUTE_MIN_PROB)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(l, _)| l.clone())
}

/// Test identifier: gives `language` probability 1 (when it is a
/// candidate) and records what it heard.
pub struct MockLid {
    language: String,
    calls: AtomicUsize,
    heard: Mutex<Vec<(usize, Vec<String>)>>,
}

impl MockLid {
    pub fn new(language: &str) -> Self {
        Self {
            language: language.into(),
            calls: AtomicUsize::new(0),
            heard: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// `(samples, candidates)` of each call, in order.
    pub fn heard(&self) -> Vec<(usize, Vec<String>)> {
        self.heard.lock().unwrap().clone()
    }
}

impl LanguageId for MockLid {
    fn scores(&self, pcm: &[f32], candidates: &[String]) -> Result<LidScores, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.heard
            .lock()
            .unwrap()
            .push((pcm.len(), candidates.to_vec()));
        Ok(candidates
            .iter()
            .map(|c| (c.clone(), if *c == self.language { 1.0 } else { 0.0 }))
            .collect())
    }
}

#[cfg(feature = "whisper")]
pub use whisper_lid::WhisperLid;

#[cfg(feature = "whisper")]
mod whisper_lid {
    use std::sync::Mutex;

    use whisper_rs::{
        FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    };

    use super::{LanguageId, LidScores, LID_SAMPLES};
    use crate::whisper::{audio_ctx_for, MIN_SAMPLES};

    /// whisper.cpp language detection on a multilingual ggml model.
    pub struct WhisperLid {
        state: Mutex<WhisperState>,
        threads: usize,
        _ctx: WhisperContext,
    }

    impl WhisperLid {
        /// Load `path` (a multilingual ggml model, e.g. `ggml-tiny.bin`).
        /// `fit_ctx` shrinks the encoder window to [`LID_SAMPLES`] + 1 s
        /// (≈ 8× less encoder work than the full 30 s window).
        pub fn load(path: &str, threads: usize, fit_ctx: bool) -> Result<Self, String> {
            let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
                .map_err(|e| format!("{path}: {e}"))?;
            if !ctx.is_multilingual() {
                return Err(format!("{path}: language ID needs a multilingual model"));
            }
            let mut state = ctx.create_state().map_err(|e| e.to_string())?;
            let threads = threads.max(1);
            if fit_ctx {
                // whisper.cpp's language detection encodes with the window
                // the state's last `whisper_full` used (`exp_n_audio_ctx`),
                // which has no setter: one tiny decode of silence sets it.
                let mut p = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                p.set_n_threads(threads as i32);
                p.set_audio_ctx(audio_ctx_for(LID_SAMPLES));
                p.set_language(Some("en"));
                p.set_max_tokens(1);
                p.set_single_segment(true);
                p.set_no_context(true);
                p.set_no_timestamps(true);
                p.set_print_special(false);
                p.set_print_progress(false);
                p.set_print_realtime(false);
                p.set_print_timestamps(false);
                state
                    .full(p, &vec![0.0; MIN_SAMPLES])
                    .map_err(|e| format!("language ID warm-up: {e}"))?;
            }
            Ok(Self {
                state: Mutex::new(state),
                threads,
                _ctx: ctx,
            })
        }
    }

    impl LanguageId for WhisperLid {
        fn scores(&self, pcm: &[f32], candidates: &[String]) -> Result<LidScores, String> {
            let pcm = &pcm[..pcm.len().min(LID_SAMPLES)];
            let mut st = self.state.lock().unwrap();
            st.pcm_to_mel(pcm, self.threads)
                .map_err(|e| e.to_string())?;
            let (_, probs) = st.lang_detect(0, self.threads).map_err(|e| e.to_string())?;
            let mut scored: LidScores = candidates
                .iter()
                .filter_map(|c| {
                    let id = whisper_rs::get_lang_id(c)?;
                    Some((c.clone(), probs.get(id as usize).copied().unwrap_or(0.0)))
                })
                .collect();
            let total: f32 = scored.iter().map(|(_, p)| p).sum();
            if total > 0.0 {
                scored.iter_mut().for_each(|(_, p)| *p /= total);
            }
            Ok(scored)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[(&str, f32)]) -> LidScores {
        v.iter().map(|(l, p)| (l.to_string(), *p)).collect()
    }

    #[test]
    fn choose_routes_only_above_the_floor_and_never_to_the_default() {
        assert_eq!(
            choose(&s(&[("en", 0.6), ("he", 0.4)]), "en"),
            Some("he".into())
        );
        assert_eq!(
            choose(&s(&[("en", 0.9), ("he", 0.1)]), "en"),
            Some("he".into())
        );
        assert_eq!(choose(&s(&[("en", 0.95), ("he", 0.05)]), "en"), None);
        assert_eq!(choose(&s(&[("en", 1.0)]), "en"), None);
        assert_eq!(choose(&s(&[]), "en"), None);
        assert_eq!(
            choose(&s(&[("en", 0.2), ("he", 0.3), ("ar", 0.5)]), "en"),
            Some("ar".into())
        );
    }

    #[test]
    fn mock_scores_its_language() {
        let m = MockLid::new("he");
        let c = vec!["en".to_string(), "he".to_string()];
        assert_eq!(
            m.scores(&[0.0; 10], &c).unwrap(),
            s(&[("en", 0.0), ("he", 1.0)])
        );
        assert_eq!(m.heard(), vec![(10, c)]);
    }
}
