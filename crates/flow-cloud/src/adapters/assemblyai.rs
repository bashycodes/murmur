//! AssemblyAI (§2.2): `POST {base}/v2/upload` (raw bytes) →
//! `POST {base}/v2/transcript` → poll `GET {base}/v2/transcript/{id}`;
//! header `authorization: <key>`. `keyterms_prompt` for the dictionary.
//! Zero retention = `DELETE /v2/transcript/{id}` after the text is read.
//! The slowest preset (polling adds round trips).

use std::time::{Duration, Instant};

use serde_json::json;

use flow_router::{CloudProviderConfig, SecretString};

use super::{secs_to_ms, text_of, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
use crate::encode::AudioFormat;
use crate::http::{self, Client, CloudError};

const POLL: Duration = Duration::from_millis(250);

pub struct AssemblyAi {
    base_url: String,
    model: String,
    delete_after: bool,
    format: AudioFormat,
}

impl AssemblyAi {
    pub fn new(p: &CloudProviderConfig) -> Self {
        Self {
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            delete_after: p.zero_retention,
            format: if p.format_override == "wav" {
                AudioFormat::Wav
            } else {
                AudioFormat::Flac
            },
        }
    }
}

impl CloudAdapter for AssemblyAi {
    fn id(&self) -> &'static str {
        "assemblyai"
    }
    fn preferred_format(&self) -> AudioFormat {
        self.format
    }
    fn max_upload_bytes(&self) -> usize {
        2_000_000_000
    }
    fn max_chunk_ms(&self) -> u32 {
        60 * 60 * 1000
    }
    fn term_budget(&self) -> TermBudget {
        TermBudget {
            max_terms: 200,
            max_chars: 64,
            max_words: 6,
            max_total_words: 1200,
        }
    }

    fn transcribe(
        &self,
        http: &Client,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        let auth = |rb: reqwest::blocking::RequestBuilder| rb.header("authorization", key.expose());
        let up = http::with_retry(req.deadline, || {
            let rb = auth(http.post(http::url(&self.base_url, "/v2/upload")))
                .header("Content-Type", "application/octet-stream")
                .body(req.audio.bytes.clone());
            http::json(http::send(rb, req.deadline)?)
        })?;
        let audio_url = text_of(&up, "/upload_url")?;
        let mut body = json!({
            "audio_url": audio_url,
            "speech_model": self.model,
            "punctuate": true,
            "format_text": true,
        });
        match req.language {
            Some(l) => body["language_code"] = json!(l),
            None => body["language_detection"] = json!(true),
        }
        if !req.terms.is_empty() {
            body["keyterms_prompt"] = json!(req.terms);
        }
        let created = http::with_retry(req.deadline, || {
            let rb = auth(http.post(http::url(&self.base_url, "/v2/transcript"))).json(&body);
            http::json(http::send(rb, req.deadline)?)
        })?;
        let id = text_of(&created, "/id")?;
        if !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(CloudError::Malformed);
        }
        let item = http::url(&self.base_url, &format!("/v2/transcript/{id}"));
        let mut v = created;
        let result = loop {
            match v.get("status").and_then(|s| s.as_str()) {
                Some("completed") => break Ok(v),
                Some("error") => {
                    break Err(CloudError::BadRequest {
                        status: 200,
                        code: Some("transcript_error".into()),
                    })
                }
                Some(_) => {}
                None => break Err(CloudError::Malformed),
            }
            if Instant::now() + POLL >= req.deadline {
                break Err(CloudError::Timeout);
            }
            std::thread::sleep(POLL);
            v = match http::send(auth(http.get(&item)), req.deadline).and_then(http::json) {
                Ok(v) => v,
                Err(e) => break Err(e),
            };
        };
        if self.delete_after {
            // Best effort; a failed delete still leaves the 24-48 h policy.
            let _ = http::send(
                auth(http.delete(&item)),
                Instant::now() + Duration::from_secs(3),
            );
        }
        let v = result?;
        Ok(CloudTranscript {
            text: v
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim()
                .to_string(),
            language: v
                .get("language_code")
                .and_then(|l| l.as_str())
                .map(str::to_string),
            billed_audio_ms: secs_to_ms(v.get("audio_duration")),
            cost_usd: None,
            request_id: Some(id),
        })
    }
}
