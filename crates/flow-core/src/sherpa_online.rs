//! Streaming-transducer backend (`AsrBackend::SherpaOnline`, feature
//! `parakeet`, crate `flow-asr-sherpa`): icefall zipformer2 models such as
//! `ivrit-ai/zipformer2-66m-streaming-he`, the default Hebrew model
//! (docs/proposals/hebrew-stt.md decisions 2-3).
//!
//! The whole buffer is fed at commit (plus tail silence) and decoded, RTF ≈
//! 0.05. A streaming encoder has no long-clip problem, so Parakeet's
//! ≤ 20 s piece decoding is not needed. Dictionary words reach sherpa as
//! hotwords exactly as for Parakeet; on this engine they only re-rank what
//! the model already heard (docs/findings/2026-10-30-hebrew-model-pins.md).

use std::sync::atomic::{AtomicU64, Ordering};

use flow_asr_sherpa::{OnlineConfig, OnlineEngine, Search};

use crate::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};
use crate::parakeet::{hotwords_from_prompt, implausibly_short};

pub struct SherpaOnlineAsr {
    engine: OnlineEngine,
    /// The language the model is configured for (the `languages` map key),
    /// `None` for the default model.
    language: Option<String>,
    /// Beam results re-decoded greedy (tests, logs).
    greedy_retries: AtomicU64,
}

impl SherpaOnlineAsr {
    /// Load the model directory (`encoder*`, `decoder*`, `joiner*`,
    /// `tokens.txt`). `language`: what the model writes, when known.
    pub fn load(dir: &str, threads: usize, language: Option<&str>) -> Result<Self, AsrError> {
        let engine = OnlineEngine::load(&OnlineConfig::new(dir, threads, language))
            .map_err(|e| AsrError::Load(e.to_string()))?;
        Ok(Self {
            engine,
            language: language.map(str::to_string),
            greedy_retries: AtomicU64::new(0),
        })
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    pub fn greedy_retries(&self) -> u64 {
        self.greedy_retries.load(Ordering::Relaxed)
    }

    fn decode(&self, pcm: &[f32], hotwords: &[String], s: Search) -> Result<String, AsrError> {
        self.engine
            .transcribe_with(pcm, hotwords, s)
            .map_err(|e| AsrError::Transcribe(e.to_string()))
    }
}

impl AsrEngine for SherpaOnlineAsr {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        // sherpa-onnx decodes cannot be interrupted; honour an abort raised
        // before the decode starts.
        if req.abort.is_set() {
            return Err(AsrError::Transcribe("aborted".into()));
        }
        let hotwords = hotwords_from_prompt(&req.initial_prompt);
        let search = self.engine.default_search(&hotwords);
        let mut text = self.decode(pcm, &hotwords, search)?;
        // Same guard as Parakeet: a beam result that is empty or
        // implausibly short on speech is re-decoded greedy, longer wins.
        if search == Search::Beam && self.engine.has_greedy() && implausibly_short(&text, pcm) {
            self.greedy_retries.fetch_add(1, Ordering::Relaxed);
            let g = self.decode(pcm, &hotwords, Search::Greedy)?;
            if g.split_whitespace().count() > text.split_whitespace().count() {
                text = g;
            }
        }
        // The configured language (D33); as the default model, the
        // session's forced language, else unknown.
        let language = self.language.clone().or_else(|| {
            req.language
                .as_deref()
                .and_then(crate::config::primary_language)
        });
        #[allow(clippy::needless_update)]
        Ok(AsrOutput {
            text,
            language,
            ..Default::default()
        })
    }
}
