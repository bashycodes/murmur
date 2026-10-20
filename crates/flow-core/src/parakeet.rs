//! Parakeet-TDT backend (feature `parakeet`, crate `flow-asr-sherpa`).
//!
//! English-only NeMo transducer through sherpa-onnx. `ModelConfig.asr_path`
//! is the model *directory*. Bias: the vocabulary that the whisper backend
//! receives as `initial_prompt` is fed to sherpa as per-decode hotwords
//! (modified beam search + context graph); see docs/core-architecture.md.

use flow_asr_sherpa::{ParakeetConfig, ParakeetEngine};

use crate::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};

pub struct ParakeetAsr {
    engine: ParakeetEngine,
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
        Ok(Self { engine })
    }
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
        let text = self
            .engine
            .transcribe(pcm, &hotwords_from_prompt(&req.initial_prompt))
            .map_err(|e| AsrError::Transcribe(e.to_string()))?;
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
