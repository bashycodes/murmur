//! ElevenLabs Scribe `POST {base}/v1/speech-to-text` (§2.2), multipart,
//! header `xi-api-key`. Raw `pcm_s16le_16` upload (no container) by
//! default; `keyterms` for the dictionary; `enable_logging=false` when
//! zero retention is on (enterprise plans only).

use reqwest::blocking::multipart::{Form, Part};

use flow_router::{CloudProviderConfig, SecretString};

use super::{text_of, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
use crate::encode::AudioFormat;
use crate::http::{self, Client, CloudError};

pub struct ElevenLabs {
    base_url: String,
    model: String,
    no_logging: bool,
    format: AudioFormat,
}

impl ElevenLabs {
    pub fn new(p: &CloudProviderConfig) -> Self {
        Self {
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            no_logging: p.zero_retention,
            format: match p.format_override.as_str() {
                "wav" => AudioFormat::Wav,
                "flac" => AudioFormat::Flac,
                _ => AudioFormat::Pcm16,
            },
        }
    }
}

impl CloudAdapter for ElevenLabs {
    fn id(&self) -> &'static str {
        "elevenlabs"
    }
    fn preferred_format(&self) -> AudioFormat {
        self.format
    }
    fn max_upload_bytes(&self) -> usize {
        1_000_000_000
    }
    fn max_chunk_ms(&self) -> u32 {
        60 * 60 * 1000
    }
    fn term_budget(&self) -> TermBudget {
        TermBudget {
            max_terms: 200,
            max_chars: 49,
            max_words: 5,
            max_total_words: 1000,
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
                .text("model_id", self.model.clone())
                .part("file", file)
                .text("tag_audio_events", "false")
                .text("diarize", "false");
            if f == AudioFormat::Pcm16 {
                form = form.text("file_format", "pcm_s16le_16");
            }
            if let Some(l) = req.language {
                form = form.text("language_code", l.to_string());
            }
            for t in req.terms {
                form = form.text("keyterms", t.clone());
            }
            if self.no_logging {
                form = form.text("enable_logging", "false");
            }
            http.post(http::url(&self.base_url, "/v1/speech-to-text"))
                .header("xi-api-key", key.expose())
                .multipart(form)
        };
        let v = http::with_retry(req.deadline, || {
            http::json(http::send(build(), req.deadline)?)
        })?;
        Ok(CloudTranscript {
            text: text_of(&v, "/text")?,
            language: v
                .get("language_code")
                .and_then(|l| l.as_str())
                .map(str::to_string),
            billed_audio_ms: None,
            cost_usd: None,
            request_id: v
                .get("transcription_id")
                .and_then(|i| i.as_str())
                .map(str::to_string),
        })
    }
}
