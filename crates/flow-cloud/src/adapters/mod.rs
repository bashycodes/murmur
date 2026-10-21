//! One adapter per wire format (docs/cloud-stt.md §2, §6.2, §7).

use std::time::Instant;

use flow_router::{CloudProviderConfig, SecretString};

use crate::encode::{AudioFormat, EncodedAudio};
use crate::http::{Client, CloudError};

pub mod assemblyai;
pub mod deepgram;
pub mod elevenlabs;
pub mod openai_compat;
pub mod openrouter;

/// One upload.
pub struct CloudRequest<'a> {
    pub audio: &'a EncodedAudio,
    /// ISO-639-1 / BCP-47, `None` = provider auto-detect.
    pub language: Option<&'a str>,
    /// Biasing terms, already capped with [`CloudAdapter::term_budget`].
    pub terms: &'a [String],
    pub deadline: Instant,
}

/// A provider's answer for one upload.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CloudTranscript {
    pub text: String,
    pub language: Option<String>,
    /// Audio the provider billed / reported, when it says.
    pub billed_audio_ms: Option<u32>,
    /// Provider-reported cost (OpenRouter `usage.cost`).
    pub cost_usd: Option<f64>,
    pub request_id: Option<String>,
}

/// How many biasing terms an adapter may send, and their shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermBudget {
    pub max_terms: usize,
    /// Per term (characters); longer terms are dropped.
    pub max_chars: usize,
    /// Per term (words); longer terms are dropped.
    pub max_words: usize,
    /// All terms together (words; a token-budget proxy).
    pub max_total_words: usize,
}

impl TermBudget {
    pub const NONE: TermBudget = TermBudget {
        max_terms: 0,
        max_chars: 0,
        max_words: 0,
        max_total_words: 0,
    };

    /// Keep terms in order (dictionary first, as given) while they fit.
    /// CR/LF and `<>` are stripped; empty results dropped.
    pub fn cap(&self, terms: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut words = 0;
        for t in terms {
            if out.len() >= self.max_terms {
                break;
            }
            let t: String = t
                .chars()
                .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
                .filter(|c| *c != '<' && *c != '>')
                .collect();
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            let n = t.split_whitespace().count();
            if n == 0 || n > self.max_words || t.chars().count() > self.max_chars {
                continue;
            }
            if words + n > self.max_total_words {
                break;
            }
            if out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
                continue;
            }
            words += n;
            out.push(t);
        }
        out
    }
}

pub trait CloudAdapter: Send + Sync {
    /// `"openai_compat"`, `"openrouter"`, …
    fn id(&self) -> &'static str;
    fn preferred_format(&self) -> AudioFormat;
    /// Chunk above this many encoded bytes.
    fn max_upload_bytes(&self) -> usize;
    /// Chunk above this much audio (e.g. OpenRouter's 60 s upstream limit).
    fn max_chunk_ms(&self) -> u32;
    fn term_budget(&self) -> TermBudget;
    fn transcribe(
        &self,
        http: &Client,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError>;
}

/// The adapter for a provider config (validated by the router).
pub fn for_provider(p: &CloudProviderConfig) -> Box<dyn CloudAdapter> {
    match p.adapter.as_str() {
        "openrouter" => Box::new(openrouter::OpenRouter::new(p)),
        "elevenlabs" => Box::new(elevenlabs::ElevenLabs::new(p)),
        "deepgram" => Box::new(deepgram::Deepgram::new(p)),
        "assemblyai" => Box::new(assemblyai::AssemblyAi::new(p)),
        _ => Box::new(openai_compat::OpenAiCompat::new(p)),
    }
}

/// Seconds (number) → ms.
pub(crate) fn secs_to_ms(v: Option<&serde_json::Value>) -> Option<u32> {
    v.and_then(|v| v.as_f64())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(|s| (s * 1000.0).round() as u32)
}

pub(crate) fn text_of(v: &serde_json::Value, ptr: &str) -> Result<String, CloudError> {
    v.pointer(ptr)
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .ok_or(CloudError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_keeps_order_and_limits() {
        let b = TermBudget {
            max_terms: 3,
            max_chars: 10,
            max_words: 2,
            max_total_words: 4,
        };
        let terms: Vec<String> = [
            "Wispr",
            "a very long phrase here",
            "Kubernetes1234",
            "line\nbreak",
            "<tag>",
            "wispr",
            "x y",
            "z",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(b.cap(&terms), ["Wispr", "line break", "tag"]);
        let b2 = TermBudget {
            max_total_words: 3,
            ..b
        };
        assert_eq!(b2.cap(&terms), ["Wispr", "line break"]);
        assert!(TermBudget::NONE.cap(&terms).is_empty());
    }
}
