//! The STT adapter seam (docs/cloud-stt.md). D28: OpenRouter is the only
//! provider, so [`openrouter::OpenRouterStt`] is the only implementation;
//! the trait stays so the chunking / encoding driver in `lib.rs` is
//! testable on its own.

use std::time::Instant;

use flow_router::SecretString;

use crate::encode::{AudioFormat, EncodedAudio};
use crate::http::CloudError;

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
    /// `X-Generation-Id` (or the body's `id`), for the deferred cost lookup.
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
    /// `"openrouter"`.
    fn id(&self) -> &'static str;
    fn preferred_format(&self) -> AudioFormat;
    /// Chunk above this many encoded bytes.
    fn max_upload_bytes(&self) -> usize;
    /// Chunk above this much audio (e.g. OpenRouter's 60 s upstream limit).
    fn max_chunk_ms(&self) -> u32;
    fn term_budget(&self) -> TermBudget;
    fn transcribe(
        &self,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError>;
    /// A format-400 was answered for `format`: use WAV from now on.
    /// Returns whether a retry with WAV makes sense.
    fn format_rejected(&self, _format: AudioFormat) -> bool {
        false
    }
}

/// Seconds (number) → ms.
pub(crate) fn secs_to_ms(v: Option<&serde_json::Value>) -> Option<u32> {
    v.and_then(|v| v.as_f64())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(|s| (s * 1000.0).round() as u32)
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
            "Zephyr",
            "a very long phrase here",
            "Kubernetes1234",
            "line\nbreak",
            "<tag>",
            "zephyr",
            "x y",
            "z",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(b.cap(&terms), ["Zephyr", "line break", "tag"]);
        let b2 = TermBudget {
            max_total_words: 3,
            ..b
        };
        assert_eq!(b2.cap(&terms), ["Zephyr", "line break"]);
        assert!(TermBudget::NONE.cap(&terms).is_empty());
    }
}
