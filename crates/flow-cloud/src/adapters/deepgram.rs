//! Deepgram `POST {base}/v1/listen?model=…` (§2.2): raw audio body,
//! `Authorization: Token <key>`, repeated `keyterm=` query params (Nova-3),
//! `mip_opt_out=true` when zero retention is on (opts out of the model
//! improvement programme).

use flow_router::{CloudProviderConfig, SecretString};

use super::{secs_to_ms, text_of, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
use crate::encode::AudioFormat;
use crate::http::{self, Client, CloudError};

pub struct Deepgram {
    base_url: String,
    model: String,
    mip_opt_out: bool,
    format: AudioFormat,
}

impl Deepgram {
    pub fn new(p: &CloudProviderConfig) -> Self {
        Self {
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            mip_opt_out: p.zero_retention,
            format: if p.format_override == "wav" {
                AudioFormat::Wav
            } else {
                AudioFormat::Flac
            },
        }
    }
}

impl CloudAdapter for Deepgram {
    fn id(&self) -> &'static str {
        "deepgram"
    }
    fn preferred_format(&self) -> AudioFormat {
        self.format
    }
    fn max_upload_bytes(&self) -> usize {
        2_000_000_000
    }
    fn max_chunk_ms(&self) -> u32 {
        // 10 min processing limit per request.
        9 * 60 * 1000
    }
    fn term_budget(&self) -> TermBudget {
        // ≤ 500 tokens total (~100 terms).
        TermBudget {
            max_terms: 100,
            max_chars: 64,
            max_words: 6,
            max_total_words: 300,
        }
    }

    fn transcribe(
        &self,
        http: &Client,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        let mut q: Vec<(&str, String)> = vec![
            ("model", self.model.clone()),
            ("punctuate", "true".into()),
            ("smart_format", "false".into()),
        ];
        match req.language {
            Some(l) => q.push(("language", l.to_string())),
            None => q.push(("detect_language", "true".into())),
        }
        for t in req.terms {
            q.push(("keyterm", t.clone()));
        }
        if self.mip_opt_out {
            q.push(("mip_opt_out", "true".into()));
        }
        let v = http::with_retry(req.deadline, || {
            let rb = http
                .post(http::url(&self.base_url, "/v1/listen"))
                .query(&q)
                .header("Authorization", format!("Token {}", key.expose()))
                .header("Content-Type", req.audio.format.mime())
                .body(req.audio.bytes.clone());
            http::json(http::send(rb, req.deadline)?)
        })?;
        Ok(CloudTranscript {
            text: text_of(&v, "/results/channels/0/alternatives/0/transcript")?,
            language: v
                .pointer("/results/channels/0/detected_language")
                .and_then(|l| l.as_str())
                .map(str::to_string),
            billed_audio_ms: secs_to_ms(v.pointer("/metadata/duration")),
            cost_usd: None,
            request_id: v
                .pointer("/metadata/request_id")
                .and_then(|i| i.as_str())
                .map(str::to_string),
        })
    }
}
