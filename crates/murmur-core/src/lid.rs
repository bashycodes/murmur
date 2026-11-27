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

use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

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

/// The candidate the identifier scored highest (`None` = `default`, or no
/// scores). D119: a one-model core asks for another model only when
/// [`choose`]'s answer is also this one, so a low-confidence pick (above
/// [`ROUTE_MIN_PROB`] but not the most likely) never costs a reload.
pub fn top(scores: &[(String, f32)], default: &str) -> Option<String> {
    scores
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(l, _)| l.clone())
        .filter(|l| l != default)
}

/// One window of a dictation's speech and the identifier's answer for it.
#[derive(Clone, Debug, PartialEq)]
pub struct LidWindow {
    pub scores: LidScores,
    /// Samples of speech the window covers.
    pub samples: usize,
}

/// Where the next window starts after `from` in speech that ends at `end`
/// (exclusive): a full [`LID_SAMPLES`] window when it fits, else, once the
/// speech is over (`ended`), the tail when it is at least a third of a
/// window (shorter is too little to tell). The engine and the
/// language-detect probe both window speech this way.
pub fn window_after(from: usize, end: usize, ended: bool) -> Option<Range<usize>> {
    if from + LID_SAMPLES <= end {
        Some(from..from + LID_SAMPLES)
    } else if ended && end.saturating_sub(from) >= LID_SAMPLES / 3 {
        Some(from..end)
    } else {
        None
    }
}

/// The first window of speech `speech`: its first [`LID_SAMPLES`], or all
/// of it when shorter (D34).
pub fn first_window(speech: Range<usize>) -> Range<usize> {
    speech.start..speech.end.min(speech.start + LID_SAMPLES)
}

/// The language of a whole dictation from the identifier's answers for its
/// windows of speech (D106): every window votes [`choose`]'s answer,
/// weighted by its samples, and the language with the most speech wins.
/// A tie goes to the routed language heard first, so the default model
/// wins only outright: the same lean towards routing as D34's low
/// per-window floor ([`ROUTE_MIN_PROB`]), and measured better than keeping
/// the current route (93.2 % vs 92.2 % on the probe). `None` = the
/// default model (also for no windows).
///
/// The first window alone decides wrongly when the dictation switches
/// language after it, or starts with a breath or a click the identifier
/// hears as English (`docs/findings/2026-11-24-language-detect.md`).
pub fn choose_whole(windows: &[LidWindow], default: &str) -> Option<String> {
    lead(speech_time(windows, default, choose))
}

/// [`choose_whole`] with [`top`] as each window's answer: the language most
/// likely over the dictation's speech (D119's confidence check).
pub fn top_whole(windows: &[LidWindow], default: &str) -> Option<String> {
    lead(speech_time(windows, default, top))
}

/// The answer with the most speech; a tie goes to a language over the default.
fn lead(time: Vec<(Option<String>, usize)>) -> Option<String> {
    let most = time.iter().map(|t| t.1).max()?;
    let leaders: Vec<&Option<String>> = time.iter().filter(|t| t.1 == most).map(|t| &t.0).collect();
    leaders
        .iter()
        .find(|l| l.is_some())
        .or(leaders.first())
        .and_then(|l| (*l).clone())
}

/// True when `remaining` more samples of speech cannot change
/// [`choose_whole`]'s answer: the leader is ahead of the runner-up by more.
pub fn decided_without(windows: &[LidWindow], default: &str, remaining: usize) -> bool {
    let mut time: Vec<usize> = speech_time(windows, default, choose)
        .into_iter()
        .map(|t| t.1)
        .collect();
    time.sort_unstable_by(|a, b| b.cmp(a));
    let second = time.get(1).copied().unwrap_or(0);
    time.first().is_some_and(|&t| t > second + remaining)
}

/// A per-window answer: [`choose`] or [`top`].
type Answer = fn(&[(String, f32)], &str) -> Option<String>;

/// Samples of speech per `answer` (`None` = the default), in the order
/// first heard.
fn speech_time(
    windows: &[LidWindow],
    default: &str,
    answer: Answer,
) -> Vec<(Option<String>, usize)> {
    let mut time: Vec<(Option<String>, usize)> = Vec::new();
    for w in windows {
        let l = answer(&w.scores, default);
        match time.iter_mut().find(|(k, _)| *k == l) {
            Some((_, t)) => *t += w.samples,
            None => time.push((l, w.samples)),
        }
    }
    time
}

/// Test identifier: gives `language` probability 1 (when it is a
/// candidate) and records what it heard.
pub struct MockLid {
    /// The answer of each call in turn; the last one repeats.
    languages: Vec<String>,
    /// Time each call takes (a slow model).
    delay: Duration,
    calls: AtomicUsize,
    heard: Mutex<Vec<(usize, Vec<String>)>>,
}

impl MockLid {
    pub fn new(language: &str) -> Self {
        Self::sequence(&[language])
    }

    /// Answers `languages[k]` on call `k` (the last one from then on): a
    /// dictation that switches language between windows.
    pub fn sequence(languages: &[&str]) -> Self {
        assert!(!languages.is_empty());
        Self {
            languages: languages.iter().map(|l| l.to_string()).collect(),
            delay: Duration::ZERO,
            calls: AtomicUsize::new(0),
            heard: Mutex::new(Vec::new()),
        }
    }

    /// Each call takes `delay` (a slow identifier).
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
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
        std::thread::sleep(self.delay);
        let k = self.calls.fetch_add(1, Ordering::SeqCst);
        let language = &self.languages[k.min(self.languages.len() - 1)];
        self.heard
            .lock()
            .unwrap()
            .push((pcm.len(), candidates.to_vec()));
        Ok(candidates
            .iter()
            .map(|c| (c.clone(), if c == language { 1.0 } else { 0.0 }))
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

    fn w(v: &[(&LidScores, usize)]) -> Vec<LidWindow> {
        v.iter()
            .map(|(sc, n)| LidWindow {
                scores: (*sc).clone(),
                samples: *n,
            })
            .collect()
    }

    #[test]
    fn choose_whole_takes_the_language_with_the_most_speech() {
        let en = s(&[("en", 0.95), ("he", 0.05)]);
        let he = s(&[("en", 0.8), ("he", 0.2)]);
        assert_eq!(choose_whole(&[], "en"), None);
        assert_eq!(choose_whole(&w(&[(&en, 48000)]), "en"), None);
        assert_eq!(choose_whole(&w(&[(&he, 48000)]), "en"), Some("he".into()));
        // English start, then mostly Hebrew.
        let v = w(&[(&en, 48000), (&he, 48000), (&he, 20000)]);
        assert_eq!(choose_whole(&v, "en"), Some("he".into()));
        // Hebrew start, then mostly English.
        let v = w(&[(&he, 48000), (&en, 48000), (&en, 20000)]);
        assert_eq!(choose_whole(&v, "en"), None);
    }

    #[test]
    fn a_tie_goes_to_the_routed_language_heard_first() {
        let en = s(&[("en", 0.9), ("he", 0.05), ("ar", 0.05)]);
        let he = s(&[("en", 0.1), ("he", 0.8), ("ar", 0.1)]);
        let ar = s(&[("en", 0.1), ("he", 0.1), ("ar", 0.8)]);
        // Three-way tie, in every order: never the default, always the
        // first route heard.
        for (order, want) in [
            ([&en, &he, &ar], "he"),
            ([&en, &ar, &he], "ar"),
            ([&he, &en, &ar], "he"),
            ([&ar, &he, &en], "ar"),
        ] {
            let v = w(&order.map(|sc| (sc, 48000)));
            assert_eq!(choose_whole(&v, "en"), Some(want.into()));
            // Ties are never decided early.
            assert!(!decided_without(&v, "en", 0));
        }
        // A tie between the default and one route.
        assert_eq!(
            choose_whole(&w(&[(&en, 48000), (&he, 48000)]), "en"),
            Some("he".into())
        );
        // An outright default win stays.
        assert_eq!(
            choose_whole(&w(&[(&en, 48000), (&en, 1), (&he, 48000)]), "en"),
            None
        );
    }

    #[test]
    fn decided_when_the_rest_cannot_outvote_the_leader() {
        let en = s(&[("en", 0.95), ("he", 0.05)]);
        let he = s(&[("en", 0.8), ("he", 0.2)]);
        let v = w(&[(&he, 48000), (&he, 48000), (&en, 48000)]);
        assert!(decided_without(&v, "en", 47999));
        assert!(!decided_without(&v, "en", 48000));
        assert!(decided_without(&w(&[(&he, 48000)]), "en", 16000));
        assert!(!decided_without(&[], "en", 0));
    }

    #[test]
    fn windows_are_full_until_the_tail_and_a_short_tail_is_dropped() {
        let n = LID_SAMPLES;
        assert_eq!(first_window(100..100 + 2 * n), 100..100 + n);
        assert_eq!(first_window(100..200), 100..200);
        assert_eq!(window_after(n, 3 * n, false), Some(n..2 * n));
        assert_eq!(window_after(n, 2 * n - 1, false), None);
        assert_eq!(window_after(n, n + n / 3, true), Some(n..n + n / 3));
        assert_eq!(window_after(n, n + n / 3 - 1, true), None);
    }

    #[test]
    fn mock_sequence_answers_in_turn_then_repeats() {
        let m = MockLid::sequence(&["en", "he"]);
        let c = vec!["en".to_string(), "he".to_string()];
        let top = |sc: LidScores| choose(&sc, "en");
        assert_eq!(top(m.scores(&[0.0; 4], &c).unwrap()), None);
        assert_eq!(top(m.scores(&[0.0; 4], &c).unwrap()), Some("he".into()));
        assert_eq!(top(m.scores(&[0.0; 4], &c).unwrap()), Some("he".into()));
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
