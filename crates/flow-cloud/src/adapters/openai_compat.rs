//! Any OpenAI-compatible `POST {base}/audio/transcriptions` (OpenAI, Groq,
//! speaches, LocalAI, custom). Multipart; `Authorization: Bearer` (omitted
//! for an empty key on self-hosted servers).
//!
//! Dialects (§2.2): `whisper` sends `language` + a `prompt`
//! ("Vocabulary: a, b."); `gpt_transcribe` (OpenAI `gpt-transcribe*`,
//! `gpt-live-transcribe*`) sends `languages[]` + `keywords[]`.

use reqwest::blocking::multipart::{Form, Part};

use flow_router::{CloudProviderConfig, SecretString};

use super::{secs_to_ms, text_of, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
use crate::encode::AudioFormat;
use crate::http::{self, Client, CloudError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Whisper,
    GptTranscribe,
}

impl Dialect {
    /// `""` = from the model id.
    pub fn resolve(dialect: &str, model: &str) -> Dialect {
        match dialect {
            "whisper" => Dialect::Whisper,
            "gpt_transcribe" => Dialect::GptTranscribe,
            _ => {
                let m = model.rsplit('/').next().unwrap_or(model);
                if m.starts_with("gpt-transcribe") || m.starts_with("gpt-live-transcribe") {
                    Dialect::GptTranscribe
                } else {
                    Dialect::Whisper
                }
            }
        }
    }
}

pub struct OpenAiCompat {
    base_url: String,
    model: String,
    dialect: Dialect,
    format: AudioFormat,
}

impl OpenAiCompat {
    pub fn new(p: &CloudProviderConfig) -> Self {
        // OpenAI lists no FLAC; everyone else takes it.
        let host = p.host();
        let default = if host == "api.openai.com" {
            AudioFormat::Wav
        } else {
            AudioFormat::Flac
        };
        Self {
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            dialect: Dialect::resolve(&p.dialect, &p.model),
            format: match p.format_override.as_str() {
                "wav" => AudioFormat::Wav,
                "flac" => AudioFormat::Flac,
                _ => default,
            },
        }
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }
}

impl CloudAdapter for OpenAiCompat {
    fn id(&self) -> &'static str {
        "openai_compat"
    }
    fn preferred_format(&self) -> AudioFormat {
        self.format
    }
    fn max_upload_bytes(&self) -> usize {
        25 * 1000 * 1000
    }
    fn max_chunk_ms(&self) -> u32 {
        // gpt-transcribe/whisper-1 take ~25 min per file; stay below.
        20 * 60 * 1000
    }
    fn term_budget(&self) -> TermBudget {
        match self.dialect {
            // whisper `prompt` ≤ 224 tokens ≈ 120 words.
            Dialect::Whisper => TermBudget {
                max_terms: 120,
                max_chars: 64,
                max_words: 6,
                max_total_words: 120,
            },
            Dialect::GptTranscribe => TermBudget {
                max_terms: 200,
                max_chars: 64,
                max_words: 6,
                max_total_words: 600,
            },
        }
    }

    fn transcribe(
        &self,
        http: &Client,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        let f = req.audio.format;
        let build = || {
            let file = Part::bytes(req.audio.bytes.clone())
                .file_name(format!("audio.{}", f.ext()))
                .mime_str(f.mime())
                .expect("static mime");
            let mut form = Form::new()
                .part("file", file)
                .text("model", self.model.clone())
                .text("response_format", "json");
            match self.dialect {
                Dialect::Whisper => {
                    if let Some(l) = req.language {
                        form = form.text("language", l.to_string());
                    }
                    if !req.terms.is_empty() {
                        form =
                            form.text("prompt", format!("Vocabulary: {}.", req.terms.join(", ")));
                    }
                }
                Dialect::GptTranscribe => {
                    if let Some(l) = req.language {
                        form = form.text("languages[]", l.to_string());
                    }
                    for t in req.terms {
                        form = form.text("keywords[]", t.clone());
                    }
                }
            }
            let mut rb = http
                .post(http::url(&self.base_url, "/audio/transcriptions"))
                .multipart(form);
            if !key.is_empty() {
                rb = rb.bearer_auth(key.expose());
            }
            rb
        };
        let v = http::with_retry(req.deadline, || {
            http::json(http::send(build(), req.deadline)?)
        })?;
        Ok(CloudTranscript {
            text: text_of(&v, "/text")?,
            language: v
                .get("language")
                .and_then(|l| l.as_str())
                .map(str::to_string),
            billed_audio_ms: secs_to_ms(v.pointer("/usage/seconds"))
                .or_else(|| secs_to_ms(v.get("duration"))),
            cost_usd: None,
            request_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_from_model() {
        assert_eq!(
            Dialect::resolve("", "gpt-transcribe"),
            Dialect::GptTranscribe
        );
        assert_eq!(
            Dialect::resolve("", "openai/gpt-live-transcribe-2"),
            Dialect::GptTranscribe
        );
        assert_eq!(
            Dialect::resolve("", "whisper-large-v3-turbo"),
            Dialect::Whisper
        );
        assert_eq!(Dialect::resolve("", "gpt-4o-transcribe"), Dialect::Whisper);
        assert_eq!(
            Dialect::resolve("gpt_transcribe", "whisper-1"),
            Dialect::GptTranscribe
        );
    }
}
