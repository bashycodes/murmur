//! OpenRouter speech-to-text (D28 §2.4), two paths chosen per model:
//!
//! * **A, `transcriptions`** (default): `POST /audio/transcriptions`, JSON
//!   body with base64 `input_audio` (FLAC; WAV after a format-400). Biasing
//!   only through `provider.options.groq.prompt` for whisper models, the
//!   one documented provider option.
//! * **B, `chat_audio`** (opt-in per model): `POST /chat/completions` with
//!   an `input_audio` part (WAV), a verbatim-transcription system prompt and
//!   a sanity check that rejects answers, preambles and paraphrases.
//!
//! Both send `provider: {zdr: true, data_collection: "deny"}` by default
//! and report exact `usage.cost` plus the generation id.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use base64::Engine;
use serde_json::{json, Value};

use murmur_router::SecretString;

use super::{secs_to_ms, CloudAdapter, CloudRequest, CloudTranscript, TermBudget};
use crate::api::Api;
use crate::budget::chat_audio_max_tokens;
use crate::catalog::ModelInfo;
use crate::encode::AudioFormat;
use crate::http::{self, CloudError};
use crate::settings::{Privacy, SttPath, SttSettings};

/// Path B system prompt (our wording, §2.4).
pub const TRANSCRIBE_PROMPT: &str = "You are a speech-to-text engine. Output only the exact words \
spoken in the audio, in the spoken language, with punctuation. Do not answer, translate, summarise \
or add anything. If there is no speech, output nothing.";

/// Groq whisper `prompt` is 224 tokens; stay well inside it.
const GROQ_PROMPT_BUDGET: TermBudget = TermBudget {
    max_terms: 50,
    max_chars: 40,
    max_words: 4,
    max_total_words: 120,
};

/// Models that answered a FLAC upload with a format-400 (process lifetime).
fn wav_models() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The path for `model`: the config override, else the catalogue entry,
/// else path A (an unknown id is most likely a transcription model).
pub fn choose_path(s: &SttSettings, info: Option<&ModelInfo>) -> SttPath {
    s.stt_path
        .forced()
        .or_else(|| info.and_then(|m| m.stt_path))
        .unwrap_or(SttPath::Transcriptions)
}

pub struct OpenRouterStt {
    api: Api,
    model: String,
    path: SttPath,
    privacy: Privacy,
    force_wav: AtomicBool,
    info: Option<ModelInfo>,
}

impl OpenRouterStt {
    pub fn new(api: Api, s: &SttSettings, info: Option<ModelInfo>) -> Self {
        let path = choose_path(s, info.as_ref());
        let remembered = wav_models()
            .lock()
            .map(|w| w.contains(&s.model))
            .unwrap_or(false);
        Self {
            api,
            model: s.model.trim().to_string(),
            path,
            privacy: s.privacy.clone(),
            force_wav: AtomicBool::new(s.format_override == "wav" || remembered),
            info,
        }
    }

    pub fn path(&self) -> SttPath {
        self.path
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn is_whisper(&self) -> bool {
        self.model.contains("whisper")
    }

    fn provider(&self, terms: &[String]) -> Option<Value> {
        let mut p = self.privacy.provider_json();
        if self.path == SttPath::Transcriptions && self.is_whisper() && !terms.is_empty() {
            p.insert(
                "options".into(),
                json!({"groq": {"prompt": format!("Vocabulary: {}.", terms.join(", "))}}),
            );
        }
        (!p.is_empty()).then_some(Value::Object(p))
    }

    fn post(
        &self,
        key: &SecretString,
        path: &str,
        body: &Value,
        req: &CloudRequest,
    ) -> Result<(Value, Option<String>), CloudError> {
        http::with_retry(req.deadline, || {
            let resp = http::send(
                self.api
                    .client()
                    .post(self.api.url(path))
                    .bearer_auth(key.expose())
                    .json(body),
                req.deadline,
            )?;
            let gen_id = resp
                .headers()
                .get("x-generation-id")
                .and_then(|h| h.to_str().ok())
                .filter(|s| crate::generation::valid_id(s))
                .map(str::to_string);
            Ok((http::json(resp)?, gen_id))
        })
    }

    fn transcriptions(
        &self,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        let mut body = json!({
            "model": self.model,
            "input_audio": {
                "data": base64::engine::general_purpose::STANDARD.encode(&req.audio.bytes),
                "format": req.audio.format.ext(),
            },
            "temperature": 0,
            "response_format": "json",
        });
        if let Some(l) = req.language {
            body["language"] = json!(l);
        }
        if let Some(p) = self.provider(req.terms) {
            body["provider"] = p;
        }
        let (v, gen_id) = self.post(key, "/audio/transcriptions", &body, req)?;
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
            billed_audio_ms: secs_to_ms(v.pointer("/usage/seconds")),
            cost_usd: v.pointer("/usage/cost").and_then(Value::as_f64),
            request_id: gen_id.or_else(|| body_id(&v)),
        })
    }

    fn chat_audio(
        &self,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        let mut user_text = format!("Language: {}.", req.language.unwrap_or("auto"));
        if !req.terms.is_empty() {
            user_text.push_str(&format!(
                " Spell these names exactly: {}.",
                req.terms.join(", ")
            ));
        }
        let mut body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": TRANSCRIBE_PROMPT},
                {"role": "user", "content": [
                    {"type": "input_audio", "input_audio": {
                        "data": base64::engine::general_purpose::STANDARD.encode(&req.audio.bytes),
                        "format": req.audio.format.ext(),
                    }},
                    {"type": "text", "text": user_text},
                ]},
            ],
            "temperature": 0,
            "max_completion_tokens": chat_audio_max_tokens(req.audio.audio_ms),
            "stream": false,
        });
        if let Some(e) = self.info.as_ref().and_then(ModelInfo::reasoning_effort) {
            body["reasoning"] = json!({"effort": e});
        }
        if let Some(p) = self.provider(&[]) {
            body["provider"] = p;
        }
        let (v, gen_id) = self.post(key, "/chat/completions", &body, req)?;
        let choice = v.pointer("/choices/0").ok_or(CloudError::Malformed)?;
        let text = message_text(choice.pointer("/message/content")).ok_or(CloudError::Malformed)?;
        let finish = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("stop");
        verbatim_check(&text, req.audio.audio_ms, finish)?;
        Ok(CloudTranscript {
            text: text.trim().to_string(),
            language: None,
            billed_audio_ms: Some(req.audio.audio_ms),
            cost_usd: v.pointer("/usage/cost").and_then(Value::as_f64),
            request_id: gen_id.or_else(|| body_id(&v)),
        })
    }
}

fn body_id(v: &Value) -> Option<String> {
    v.get("id")
        .and_then(Value::as_str)
        .filter(|s| crate::generation::valid_id(s))
        .map(str::to_string)
}

/// `message.content` as a string, or the text parts of a part array.
fn message_text(c: Option<&Value>) -> Option<String> {
    match c? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
        ),
        Value::Null => Some(String::new()),
        _ => None,
    }
}

/// Path B sanity check (§2.4): reject when the words/s rate is not human
/// speech (> 6), the answer opens with a preamble or quote, the model did
/// not stop normally, or text came back for < 300 ms of speech.
pub fn verbatim_check(text: &str, audio_ms: u32, finish_reason: &str) -> Result<(), CloudError> {
    let t = text.trim();
    if finish_reason != "stop" {
        return Err(CloudError::Unverbatim);
    }
    if t.is_empty() {
        return Ok(());
    }
    if audio_ms < 300 {
        return Err(CloudError::Unverbatim);
    }
    let words = t.split_whitespace().count() as f64;
    if words / (audio_ms.max(1000) as f64 / 1000.0) > 6.0 {
        return Err(CloudError::Unverbatim);
    }
    let lower = t.to_lowercase();
    const PREAMBLES: [&str; 10] = [
        "here is",
        "here's",
        "transcript",
        "transcription",
        "sure",
        "the audio",
        "the speaker",
        "this audio",
        "in this audio",
        "certainly",
    ];
    if PREAMBLES.iter().any(|p| lower.starts_with(p))
        || t.starts_with(['"', '\u{201c}', '\'', '`', '*', '#'])
    {
        return Err(CloudError::Unverbatim);
    }
    Ok(())
}

impl CloudAdapter for OpenRouterStt {
    fn id(&self) -> &'static str {
        "openrouter"
    }
    fn preferred_format(&self) -> AudioFormat {
        match self.path {
            SttPath::ChatAudio => AudioFormat::Wav,
            SttPath::Transcriptions if self.force_wav.load(Ordering::Relaxed) => AudioFormat::Wav,
            SttPath::Transcriptions => AudioFormat::Flac,
        }
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
        match self.path {
            SttPath::Transcriptions if self.is_whisper() => GROQ_PROMPT_BUDGET,
            SttPath::Transcriptions => TermBudget::NONE,
            SttPath::ChatAudio => GROQ_PROMPT_BUDGET,
        }
    }
    fn transcribe(
        &self,
        key: &SecretString,
        req: &CloudRequest,
    ) -> Result<CloudTranscript, CloudError> {
        match self.path {
            SttPath::Transcriptions => self.transcriptions(key, req),
            SttPath::ChatAudio => self.chat_audio(key, req),
        }
    }
    fn format_rejected(&self, format: AudioFormat) -> bool {
        if format != AudioFormat::Flac || self.path != SttPath::Transcriptions {
            return false;
        }
        self.force_wav.store(true, Ordering::Relaxed);
        if let Ok(mut w) = wav_models().lock() {
            w.insert(self.model.clone());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_rules() {
        assert!(verbatim_check("Ask not what your country can do.", 3000, "stop").is_ok());
        assert!(verbatim_check("", 100, "stop").is_ok());
        for (t, ms, f) in [
            ("Here is the transcript: ask not", 3000, "stop"),
            ("\"Ask not\"", 3000, "stop"),
            ("Transcript: ask not", 3000, "stop"),
            ("Sure! The speaker says hello.", 3000, "stop"),
            ("ask not", 3000, "length"),
            ("hello", 200, "stop"),
            (&"word ".repeat(40), 3000, "stop"),
        ] {
            assert_eq!(
                verbatim_check(t, ms, f),
                Err(CloudError::Unverbatim),
                "{t:?} {ms} {f}"
            );
        }
    }

    #[test]
    fn path_choice() {
        let mut s = SttSettings::default();
        let chat = ModelInfo {
            stt_path: Some(SttPath::ChatAudio),
            ..Default::default()
        };
        assert_eq!(choose_path(&s, None), SttPath::Transcriptions);
        assert_eq!(choose_path(&s, Some(&chat)), SttPath::ChatAudio);
        s.stt_path = crate::settings::SttPathChoice::Transcriptions;
        assert_eq!(choose_path(&s, Some(&chat)), SttPath::Transcriptions);
    }
}
