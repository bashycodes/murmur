//! D85: speech to text on the user's LiteLLM proxy (or any OpenAI-compatible
//! server): `POST {base}/audio/transcriptions`, the OpenAI multipart form
//! (`file`, `model`, `response_format=json`, `temperature`, optional
//! `language` and `prompt`). No OpenRouter extensions (`provider`,
//! `input_audio` JSON, `/generation`). The base URL is the one the user
//! typed (`CustomCleanupConfig::api_base`); plain http is allowed, as for
//! D52 cleanup. The key is optional: without one no `Authorization` header
//! is sent. Cost: LiteLLM's `x-litellm-response-cost` header (else its
//! `-original`), else unknown.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use murmur_router::cloud::CustomCleanupConfig;
use murmur_router::SecretString;

use super::{secs_to_ms, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
use crate::encode::AudioFormat;
use crate::http::{self, CloudError};

/// Adapter id (`CallInfo::adapter`).
pub const ID: &str = "litellm";

/// OpenAI's `prompt` is 224 tokens for whisper; the same budget as
/// OpenRouter's Groq whisper prompt.
const PROMPT_BUDGET: TermBudget = TermBudget {
    max_terms: 50,
    max_chars: 40,
    max_words: 4,
    max_total_words: 120,
};

/// Speech to text on the proxy.
pub struct LiteLlmStt {
    /// Normalised API root, or why the configured URL is unusable.
    base: Result<String, String>,
    model: String,
    http: http::Client,
    force_wav: AtomicBool,
}

impl LiteLlmStt {
    pub fn new(cfg: &CustomCleanupConfig) -> Self {
        Self {
            base: cfg.api_base(),
            model: cfg.stt_model.trim().to_string(),
            http: http::client_with(
                false,
                Duration::from_millis(cfg.connect_timeout_ms.clamp(200, 10_000)),
            ),
            force_wav: AtomicBool::new(false),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn form(&self, req: &CloudRequest) -> reqwest::blocking::multipart::Form {
        let f = req.audio.format;
        let file = reqwest::blocking::multipart::Part::bytes(req.audio.bytes.clone())
            .file_name(format!("audio.{}", f.ext()))
            .mime_str(f.mime())
            .expect("static mime type");
        let mut form = reqwest::blocking::multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "json")
            .text("temperature", "0");
        if let Some(l) = req.language {
            form = form.text("language", l.to_string());
        }
        if !req.terms.is_empty() {
            form = form.text("prompt", format!("Vocabulary: {}.", req.terms.join(", ")));
        }
        form
    }
}

impl CloudAdapter for LiteLlmStt {
    fn id(&self) -> &'static str {
        ID
    }

    fn preferred_format(&self) -> AudioFormat {
        if self.force_wav.load(Ordering::Relaxed) {
            AudioFormat::Wav
        } else {
            AudioFormat::Flac
        }
    }

    fn max_upload_bytes(&self) -> usize {
        // OpenAI's transcription limit is 25 MB per file.
        24 * 1000 * 1000
    }

    fn max_chunk_ms(&self) -> u32 {
        // Keep each request short enough for a proxy's default timeouts.
        55_000
    }

    fn term_budget(&self) -> TermBudget {
        PROMPT_BUDGET
    }

    fn transcribe(
        &self,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        let base = self
            .base
            .as_ref()
            .map_err(|_| CloudError::Connect("bad_base_url".into()))?;
        if !murmur_router::cloud::valid_custom_model_id(&self.model) {
            return Err(CloudError::BadRequest {
                status: 0,
                code: Some("bad_model_id".into()),
            });
        }
        let url = format!("{base}/audio/transcriptions");
        let (v, cost, call_id) = http::with_retry(req.deadline, || {
            let mut rb = self.http.post(&url).multipart(self.form(req));
            if !key.is_empty() {
                rb = rb.bearer_auth(key.expose());
            }
            let resp = http::send(rb, req.deadline)?;
            let cost = crate::custom::cost_header(resp.headers());
            let call_id = resp
                .headers()
                .get("x-litellm-call-id")
                .and_then(|h| h.to_str().ok())
                .filter(|s| crate::generation::valid_id(s))
                .map(str::to_string);
            Ok((http::json(resp)?, cost, call_id))
        })?;
        let text = v
            .get("text")
            .and_then(Value::as_str)
            .ok_or(CloudError::Malformed)?;
        Ok(CloudTranscript {
            text: text.trim().to_string(),
            language: v
                .get("language")
                .and_then(Value::as_str)
                .map(str::to_string),
            // `verbose_json` / gpt-4o-transcribe report it; plain whisper
            // `json` does not (the chunk length is used then).
            billed_audio_ms: secs_to_ms(v.get("duration"))
                .or_else(|| secs_to_ms(v.pointer("/usage/seconds"))),
            cost_usd: cost,
            request_id: call_id,
        })
    }

    fn format_rejected(&self, format: AudioFormat) -> bool {
        if format != AudioFormat::Flac {
            return false;
        }
        self.force_wav.store(true, Ordering::Relaxed);
        true
    }
}
