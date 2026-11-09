//! Cost estimates from the catalogue (D28 §2.6), replacing the static
//! per-hour price table. Exact cost always comes from `usage.cost` (or the
//! deferred `/generation` lookup); these numbers are for the pre-flight
//! budget check, the picker's price labels and `provider.max_price`.

use serde::{Deserialize, Serialize};

use crate::catalog::{ModelInfo, AUDIO_TOKENS_PER_SECOND};
use crate::settings::SttPath;

/// Headroom over the model (cheapest-endpoint) price when the ZDR endpoint
/// prices are unknown: endpoint prices can be several × the model price.
pub const UNKNOWN_ENDPOINT_HEADROOM: f64 = 3.0;
/// `provider.max_price` = this × the catalogue price (§2.6 rule 2).
pub const MAX_PRICE_FACTOR: f64 = 4.0;
/// Prompt v2 size for the picker's "per dictation" label (≈ 1 k tokens).
pub const TYPICAL_PROMPT_TOKENS: u32 = 1000;
/// A typical transcript, in tokens, for the label's output budget.
pub const TYPICAL_TRANSCRIPT_TOKENS: u32 = 40;

/// Rough token count (≈ 4 chars per token, rounded up).
pub fn est_tokens(text: &str) -> u32 {
    (text.chars().count() as u32).div_ceil(4)
}

/// Cleanup output budget: `max(16, 2 × tokens(transcript) + 32)`
/// (some providers enforce a minimum of 16).
pub fn cleanup_max_tokens(transcript: &str) -> u32 {
    (2 * est_tokens(transcript) + 32).max(16)
}

/// Path-B output budget: `ceil(audio_s × 8) + 32`.
pub fn chat_audio_max_tokens(audio_ms: u32) -> u32 {
    (audio_ms as u64 * 8).div_ceil(1000) as u32 + 32
}

/// The price used for estimates: the highest ZDR endpoint price when
/// known, else the model price × [`UNKNOWN_ENDPOINT_HEADROOM`].
fn worst_price(m: &ModelInfo) -> (crate::catalog::Price, f64) {
    match m.zdr_price_max {
        Some(p) => (p, 1.0),
        None => (m.price, UNKNOWN_ENDPOINT_HEADROOM),
    }
}

/// Pre-flight estimate for `audio_s` seconds of STT; `None` = unknown
/// (unknown unit, free-text id, router).
pub fn estimate_stt(m: &ModelInfo, audio_s: f64) -> Option<f64> {
    if m.router || !m.price_unit_ok {
        return None;
    }
    let (p, k) = worst_price(m);
    let v = match m.stt_path? {
        SttPath::Transcriptions => p.per_audio_second.or(m.price.per_audio_second)? * audio_s,
        SttPath::ChatAudio => {
            let audio = p.per_audio_token.or(p.prompt)? * AUDIO_TOKENS_PER_SECOND * audio_s;
            let out = p.completion.unwrap_or(0.0)
                * chat_audio_max_tokens((audio_s * 1000.0) as u32) as f64;
            audio + out
        }
    };
    Some((v + p.request.unwrap_or(0.0)) * k)
}

/// Pre-flight estimate for one cleanup call.
pub fn estimate_cleanup(m: &ModelInfo, prompt_tokens: u32, max_tokens: u32) -> Option<f64> {
    if m.router || !m.is_cleanup() {
        return None;
    }
    let (p, k) = worst_price(m);
    let v = p.prompt? * prompt_tokens as f64
        + p.completion.unwrap_or(0.0) * max_tokens as f64
        + p.request.unwrap_or(0.0);
    Some(v * k)
}

/// The picker's "≈ $x per dictation" (model price, no headroom).
pub fn cleanup_per_dictation(m: &ModelInfo) -> Option<f64> {
    if m.router {
        return None;
    }
    let out = 2 * TYPICAL_TRANSCRIPT_TOKENS + 32;
    Some(
        m.price.prompt? * TYPICAL_PROMPT_TOKENS as f64
            + m.price.completion.unwrap_or(0.0) * out as f64
            + m.price.request.unwrap_or(0.0),
    )
}

/// `provider.max_price` for chat requests, USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaxPrice {
    pub prompt: f64,
    pub completion: f64,
}

impl MaxPrice {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"prompt": self.prompt, "completion": self.completion})
    }
}

/// [`MAX_PRICE_FACTOR`] × the worst known price, per million tokens.
/// `None` for free/unknown-price models (no ceiling is better than a zero
/// ceiling that refuses every endpoint).
pub fn max_price(m: &ModelInfo) -> Option<MaxPrice> {
    let p = m.zdr_price_max.unwrap_or(m.price);
    let prompt = p.prompt.or(m.price.prompt)?;
    let completion = p.completion.or(m.price.completion).unwrap_or(0.0);
    if prompt <= 0.0 && completion <= 0.0 {
        return None;
    }
    let per_m = |v: f64| (v * 1e6 * MAX_PRICE_FACTOR * 1e6).round() / 1e6;
    Some(MaxPrice {
        prompt: per_m(prompt),
        completion: per_m(completion),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::settings::{DEFAULT_CLEANUP_MODEL, DEFAULT_STT_MODEL};

    #[test]
    fn budgets() {
        assert_eq!(cleanup_max_tokens(""), 32);
        assert_eq!(cleanup_max_tokens("abcdefgh"), 36);
        assert_eq!(chat_audio_max_tokens(10_000), 112);
        assert_eq!(chat_audio_max_tokens(1), 33);
    }

    #[test]
    fn estimates_use_the_highest_zdr_price() {
        let c = Catalog::baked();
        let stt = c.get(DEFAULT_STT_MODEL).unwrap();
        // Groq's ZDR endpoint (0.0000111/s) is dearer than DeepInfra's.
        let e = estimate_stt(stt, 3600.0).unwrap();
        assert!((e - 0.04).abs() < 0.001, "{e}");
        let cl = c.get(DEFAULT_CLEANUP_MODEL).unwrap();
        let e = estimate_cleanup(cl, 1000, 100).unwrap();
        assert!(e > 0.0 && e < 0.01, "{e}");
        let mp = max_price(cl).unwrap();
        assert!(mp.prompt > 0.59 * 3.9 && mp.completion > 0.0, "{mp:?}");
    }
}
