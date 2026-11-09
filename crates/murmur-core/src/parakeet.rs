//! Parakeet-TDT backend (feature `parakeet`, crate `murmur-asr-sherpa`).
//!
//! NeMo transducer through sherpa-onnx (English Parakeet, or a
//! language fine-tune such as Parakeet-v3-he; see [`ParakeetAsr::with_language`]). `ModelConfig.asr_path`
//! is the model *directory*. Bias: the vocabulary that the whisper backend
//! receives as `initial_prompt` is fed to sherpa as per-decode hotwords
//! (modified beam search + context graph); see docs/core-architecture.md.
//! Decodes without hotwords use greedy search, and an empty or implausibly
//! short beam result is re-decoded greedy, then in pieces
//! (`ModelConfig.parakeet_decoding`, docs/proposals/stt-model-survey.md).

use std::sync::atomic::{AtomicU64, Ordering};

use murmur_asr_sherpa::{ParakeetConfig, ParakeetEngine, Search};

use crate::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use crate::config::{ParakeetDecoding, SAMPLE_RATE};
use crate::{segment, vad};

/// Pieces of the empty-result fallback are at most this long.
pub const FALLBACK_PIECE_SAMPLES: usize = SAMPLE_RATE * 20;
/// The fallbacks run only when the VAD heard at least this much speech.
pub const FALLBACK_MIN_SPEECH_MS: u64 = 1000;
/// A beam result with fewer words per second of VAD speech than this is
/// re-decoded greedy (survey: "implausibly short").
pub const MIN_WORDS_PER_SPEECH_S: f32 = 0.5;

pub struct ParakeetAsr {
    engine: ParakeetEngine,
    /// Beam results re-decoded greedy (tests, logs).
    greedy_retries: AtomicU64,
    /// Empty-on-speech re-decodes in pieces (tests, logs).
    fallbacks: AtomicU64,
    /// The language the model decodes, reported in `AsrOutput::language`
    /// (D33). `"en"` unless [`ParakeetAsr::with_language`] says otherwise.
    language: String,
}

impl ParakeetAsr {
    pub fn load(dir: &str, threads: usize) -> Result<Self, AsrError> {
        Self::load_with(dir, threads, ParakeetDecoding::Auto)
    }

    pub fn load_with(dir: &str, threads: usize, d: ParakeetDecoding) -> Result<Self, AsrError> {
        let mut cfg = ParakeetConfig::new(dir, threads);
        match d {
            ParakeetDecoding::Auto => {}
            ParakeetDecoding::Beam => cfg.greedy_fallback = false,
            ParakeetDecoding::Greedy => cfg.hotwords = false,
        }
        // Tuning knob for field tests; the default is the measured one.
        // `0` = greedy only (the pre-fix workaround for the beam-4 empty
        // results; a systemd drop-in may still set it).
        if let Some(s) = murmur_build_info::env::var("MURMUR_PARAKEET_HOTWORDS_SCORE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
        {
            cfg.hotwords = s > 0.0;
            cfg.hotwords_score = s;
        }
        // Same, for the beam width (D68's latency probe; the default is D12's 4).
        if let Some(b) = murmur_build_info::env::var("MURMUR_PARAKEET_BEAM")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&b| b > 0)
        {
            cfg.max_active_paths = b;
        }
        let engine = ParakeetEngine::load(&cfg).map_err(|e| AsrError::Load(e.to_string()))?;
        Ok(Self {
            engine,
            greedy_retries: AtomicU64::new(0),
            fallbacks: AtomicU64::new(0),
            language: "en".into(),
        })
    }

    /// The language this model decodes (ISO 639-1, e.g. `"he"` for
    /// Parakeet-v3-he); reported as `AsrOutput::language`. Default `"en"`.
    pub fn with_language(mut self, language: &str) -> Self {
        self.language = language.to_string();
        self
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// How long each step of the load took (sherpa sessions etc.).
    pub fn load_phases(&self) -> &[murmur_asr_sherpa::LoadPhase] {
        self.engine.load_phases()
    }

    /// How many beam results were re-decoded with greedy search.
    pub fn greedy_retries(&self) -> u64 {
        self.greedy_retries.load(Ordering::Relaxed)
    }

    /// How many decodes came back empty on speech and were re-decoded in
    /// pieces.
    pub fn fallbacks(&self) -> u64 {
        self.fallbacks.load(Ordering::Relaxed)
    }

    fn decode(&self, pcm: &[f32], hotwords: &[String], s: Search) -> Result<String, AsrError> {
        self.engine
            .transcribe_with(pcm, hotwords, s)
            .map_err(|e| AsrError::Transcribe(e.to_string()))
    }

    /// Greedy without hotwords, beam with them; a beam result that is
    /// empty or implausibly short on VAD speech is re-decoded greedy and
    /// the longer text wins.
    fn decode_robust(&self, pcm: &[f32], hotwords: &[String]) -> Result<String, AsrError> {
        let search = self.engine.default_search(hotwords);
        let text = self.decode(pcm, hotwords, search)?;
        if search == Search::Beam && self.engine.has_greedy() && implausibly_short(&text, pcm) {
            self.greedy_retries.fetch_add(1, Ordering::Relaxed);
            let g = self.decode(pcm, hotwords, Search::Greedy)?;
            if g.split_whitespace().count() > text.split_whitespace().count() {
                return Ok(g);
            }
        }
        Ok(text)
    }
}

impl ParakeetAsr {
    fn decode_pieces(
        &self,
        pcm: &[f32],
        hotwords: &[String],
        req: &AsrRequest,
    ) -> Result<String, AsrError> {
        let pieces = fallback_pieces(pcm);
        let mut parts = Vec::with_capacity(pieces.len());
        for r in &pieces {
            if req.abort.is_set() {
                return Err(AsrError::Transcribe("aborted".into()));
            }
            parts.push(self.decode_robust(&pcm[r.clone()], hotwords)?);
        }
        // Punctuate across each cut with a decode of the audio around it
        // (see `segment::splice`).
        for k in 0..pieces.len().saturating_sub(1) {
            if parts[k].is_empty() || parts[k + 1].is_empty() {
                continue;
            }
            if req.abort.is_set() {
                return Err(AsrError::Transcribe("aborted".into()));
            }
            let b = segment::bridge_range(pieces[k].end, pieces[k].start, pieces[k + 1].end);
            let bt = self.decode_robust(&pcm[b], hotwords)?;
            if let Some((l, r)) = segment::splice(&parts[k], &parts[k + 1], &bt) {
                parts[k] = l;
                parts[k + 1] = r;
            }
        }
        // Sentence-case repair is an English rule (D33).
        if crate::config::english_rules(Some(&self.language)) {
            segment::capitalize_joins(&mut parts);
        }
        Ok(parts
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" "))
    }
}

fn speech_ms(pcm: &[f32]) -> u64 {
    vad::detect(pcm).map_or(0, |sp| sp.speech_ms())
}

/// Fewer than [`MIN_WORDS_PER_SPEECH_S`] words per second of VAD speech
/// (and at least [`FALLBACK_MIN_SPEECH_MS`] of it).
pub fn implausibly_short(text: &str, pcm: &[f32]) -> bool {
    let ms = speech_ms(pcm);
    ms >= FALLBACK_MIN_SPEECH_MS
        && (text.split_whitespace().count() as f32) < MIN_WORDS_PER_SPEECH_S * ms as f32 / 1000.0
}

/// Whether an empty decode of `pcm` deserves the piecewise fallback: more
/// than 2 s of audio with at least [`FALLBACK_MIN_SPEECH_MS`] of speech.
pub fn needs_fallback(pcm: &[f32]) -> bool {
    pcm.len() > 2 * SAMPLE_RATE && speech_ms(pcm) >= FALLBACK_MIN_SPEECH_MS
}

/// Decode pieces: ≤ 20 s, and at least two (an empty clip shorter than
/// 20 s is halved at its longest pause).
pub fn fallback_pieces(pcm: &[f32]) -> Vec<std::ops::Range<usize>> {
    segment::split_long(pcm, FALLBACK_PIECE_SAMPLES.min(pcm.len().div_ceil(2)))
}

/// Recover the phrase list from [`crate::asr::build_initial_prompt`]'s
/// `"a, b c, d."` rendering, so this backend needs no new `AsrRequest` field.
pub fn hotwords_from_prompt(prompt: &str) -> Vec<String> {
    let p = prompt.trim();
    let p = p.strip_suffix('.').unwrap_or(p);
    p.split(", ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl AsrEngine for ParakeetAsr {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        // sherpa-onnx decodes cannot be interrupted; honour an abort that
        // was raised before the decode starts (stale live partial).
        if req.abort.is_set() {
            return Err(AsrError::Transcribe("aborted".into()));
        }
        let hotwords = hotwords_from_prompt(&req.initial_prompt);
        // One long decode loses whole utterances (v2 greedy: the 17 s third
        // sentence of a 30 s clip, 22 words at 42.8 s; sherpa-onnx PR
        // #4084), and beam search returns nothing for some (41.7 s 3/3,
        // survey 25.1 s 6/6). Clips over 20 s are therefore always decoded
        // in ≤ 20 s pieces cut at their longest pauses: same WER as whole
        // clips on the 2026-10-23 probe, and the cost stays linear. A
        // shorter clip that comes back empty on speech is halved once
        // (docs/proposals/streaming-asr-latency.md §1.3).
        let text = if pcm.len() > FALLBACK_PIECE_SAMPLES {
            self.decode_pieces(pcm, &hotwords, req)?
        } else {
            let t = self.decode_robust(pcm, &hotwords)?;
            if t.is_empty() && needs_fallback(pcm) {
                self.fallbacks.fetch_add(1, Ordering::Relaxed);
                self.decode_pieces(pcm, &hotwords, req)?
            } else {
                t
            }
        };
        // `..Default::default()` keeps this compiling when AsrOutput grows
        // fields (branch core-p1 adds `no_speech_prob`).
        #[allow(clippy::needless_update)]
        Ok(AsrOutput {
            text,
            // A Parakeet model decodes one language (D33).
            language: Some(self.language.clone()),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::build_initial_prompt;
    use crate::dictionary::{DictEntry, Dictionary};

    #[test]
    fn prompt_round_trips_to_hotwords() {
        let d = Dictionary {
            entries: vec![
                DictEntry::word("Kubernetes"),
                DictEntry::replacement("zephyr", "Zephyr Cloud"),
            ],
        };
        let p = build_initial_prompt(&d, &["Alice".into()]);
        assert_eq!(
            hotwords_from_prompt(&p),
            ["Kubernetes", "Zephyr Cloud", "Alice"]
        );
        assert!(hotwords_from_prompt("").is_empty());
    }
}
