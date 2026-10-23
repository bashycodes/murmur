//! Parakeet-TDT backend (feature `parakeet`, crate `flow-asr-sherpa`).
//!
//! English-only NeMo transducer through sherpa-onnx. `ModelConfig.asr_path`
//! is the model *directory*. Bias: the vocabulary that the whisper backend
//! receives as `initial_prompt` is fed to sherpa as per-decode hotwords
//! (modified beam search + context graph); see docs/core-architecture.md.

use std::sync::atomic::{AtomicU64, Ordering};

use flow_asr_sherpa::{ParakeetConfig, ParakeetEngine};

use crate::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use crate::config::SAMPLE_RATE;
use crate::{segment, vad};

/// Pieces of the empty-result fallback are at most this long.
pub const FALLBACK_PIECE_SAMPLES: usize = SAMPLE_RATE * 20;
/// The fallback runs only when the VAD heard at least this much speech.
pub const FALLBACK_MIN_SPEECH_MS: u64 = 1000;

pub struct ParakeetAsr {
    engine: ParakeetEngine,
    /// Empty-on-speech re-decodes (tests, logs).
    fallbacks: AtomicU64,
}

impl ParakeetAsr {
    pub fn load(dir: &str, threads: usize) -> Result<Self, AsrError> {
        let mut cfg = ParakeetConfig::new(dir, threads);
        // Tuning knob for field tests; the default is the measured one.
        if let Some(s) = std::env::var("FLOW_PARAKEET_HOTWORDS_SCORE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
        {
            cfg.hotwords = s > 0.0;
            cfg.hotwords_score = s;
        }
        let engine = ParakeetEngine::load(&cfg).map_err(|e| AsrError::Load(e.to_string()))?;
        Ok(Self {
            engine,
            fallbacks: AtomicU64::new(0),
        })
    }

    /// How many decodes came back empty on speech and were re-decoded in
    /// pieces.
    pub fn fallbacks(&self) -> u64 {
        self.fallbacks.load(Ordering::Relaxed)
    }

    fn decode(&self, pcm: &[f32], hotwords: &[String]) -> Result<String, AsrError> {
        self.engine
            .transcribe(pcm, hotwords)
            .map_err(|e| AsrError::Transcribe(e.to_string()))
    }
}

/// Whether an empty decode of `pcm` deserves the piecewise fallback: more
/// than 2 s of audio with at least [`FALLBACK_MIN_SPEECH_MS`] of speech.
pub fn needs_fallback(pcm: &[f32]) -> bool {
    pcm.len() > 2 * SAMPLE_RATE
        && vad::detect(pcm).is_some_and(|sp| sp.speech_ms() >= FALLBACK_MIN_SPEECH_MS)
}

/// Fallback pieces: ≤ 20 s, and at least two (an empty clip shorter than
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
        let mut text = self.decode(pcm, &hotwords)?;
        // Beam search sometimes returns nothing for a long clip with plenty
        // of speech (41.7 s LibriSpeech join, 3/3 runs; greedy and the same
        // clip cut at its pauses are fine). Losing a whole dictation is the
        // worst outcome, so re-decode it in pieces cut at pauses
        // (docs/proposals/streaming-asr-latency.md §1.3).
        if text.is_empty() && needs_fallback(pcm) {
            self.fallbacks.fetch_add(1, Ordering::Relaxed);
            let mut parts = Vec::new();
            for r in fallback_pieces(pcm) {
                if req.abort.is_set() {
                    return Err(AsrError::Transcribe("aborted".into()));
                }
                let t = self.decode(&pcm[r], &hotwords)?;
                if !t.is_empty() {
                    parts.push(t);
                }
            }
            text = parts.join(" ");
        }
        // `..Default::default()` keeps this compiling when AsrOutput grows
        // fields (branch core-p1 adds `no_speech_prob`).
        #[allow(clippy::needless_update)]
        Ok(AsrOutput {
            text,
            // parakeet-tdt-0.6b-v2 is English-only.
            language: Some("en".into()),
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
                DictEntry::replacement("wispr", "Wispr Flow"),
            ],
        };
        let p = build_initial_prompt(&d, &["Alice".into()]);
        assert_eq!(
            hotwords_from_prompt(&p),
            ["Kubernetes", "Wispr Flow", "Alice"]
        );
        assert!(hotwords_from_prompt("").is_empty());
    }
}
