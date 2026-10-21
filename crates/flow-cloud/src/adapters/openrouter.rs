//! OpenRouter `POST {base}/audio/transcriptions` (§2.1), JSON body with
//! base64 `input_audio`. Reports exact `usage.cost`; `provider.zdr` routes
//! only to zero-data-retention endpoints. OpenRouter ignores a multipart
//! `prompt`, so v1 sends no biasing terms (provider-specific
//! `provider.options.<slug>.prompt` is not wired yet).

use base64::Engine;
use serde_json::json;

use flow_router::{CloudProviderConfig, SecretString};

use super::{secs_to_ms, text_of, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
use crate::encode::AudioFormat;
use crate::http::{self, Client, CloudError};

pub struct OpenRouter {
    base_url: String,
    model: String,
    zdr: bool,
    format: AudioFormat,
}

impl OpenRouter {
    pub fn new(p: &CloudProviderConfig) -> Self {
        Self {
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            zdr: p.zero_retention,
            format: if p.format_override == "wav" {
                AudioFormat::Wav
            } else {
                AudioFormat::Flac
            },
        }
    }
}

impl CloudAdapter for OpenRouter {
    fn id(&self) -> &'static str {
        "openrouter"
    }
    fn preferred_format(&self) -> AudioFormat {
        self.format
    }
    fn max_upload_bytes(&self) -> usize {
        // 25 MB inline after base64 (+33 %).
        18 * 1000 * 1000
    }
    fn max_chunk_ms(&self) -> u32 {
        // Upstream timeout is 60 s per request.
        55_000
    }
    fn term_budget(&self) -> TermBudget {
        TermBudget::NONE
    }

    fn transcribe(
        &self,
        http: &Client,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        let mut body = json!({
            "model": self.model,
            "input_audio": {
                "data": base64::engine::general_purpose::STANDARD.encode(&req.audio.bytes),
                "format": req.audio.format.ext(),
            },
            "response_format": "json",
        });
        if let Some(l) = req.language {
            body["language"] = json!(l);
        }
        if self.zdr {
            body["provider"] = json!({"zdr": true, "data_collection": "deny"});
        }
        let (v, gen_id) = http::with_retry(req.deadline, || {
            let resp = http::send(
                http.post(http::url(&self.base_url, "/audio/transcriptions"))
                    .bearer_auth(key.expose())
                    .json(&body),
                req.deadline,
            )?;
            let gen_id = resp
                .headers()
                .get("x-generation-id")
                .and_then(|h| h.to_str().ok())
                .map(str::to_string);
            Ok((http::json(resp)?, gen_id))
        })?;
        Ok(CloudTranscript {
            text: text_of(&v, "/text")?,
            language: v
                .get("language")
                .and_then(|l| l.as_str())
                .map(str::to_string),
            billed_audio_ms: secs_to_ms(v.pointer("/usage/seconds")),
            cost_usd: v.pointer("/usage/cost").and_then(|c| c.as_f64()),
            request_id: gen_id.or_else(|| v.get("id").and_then(|i| i.as_str()).map(str::to_string)),
        })
    }
}
