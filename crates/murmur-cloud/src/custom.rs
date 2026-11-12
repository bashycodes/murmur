//! D52: text cleanup through the user's own OpenAI-compatible server (a
//! LiteLLM proxy, vLLM, Ollama, …) instead of OpenRouter. D85: the same
//! server can do speech to text too ([`crate::adapters::litellm`]); its
//! speech models are found here ([`speech_models`]).
//!
//! The base URL is whatever the user typed (normalised by
//! `murmur_router::cloud::normalize_custom_base_url`); plain http is allowed
//! to any host by the owner's choice (D52 notes the risk). Requests are
//! plain OpenAI chat completions, streamed, with none of OpenRouter's
//! extensions (`provider`, `reasoning`, `include_reasoning`, `/key`,
//! `/generation`). The key is optional: without one no `Authorization`
//! header is sent. Redirects are never followed (same client rules as
//! OpenRouter). Cost: LiteLLM's `x-litellm-response-cost` header when the
//! server sends it, else unknown.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use murmur_core::asr::AbortFlag;
use murmur_core::cleanup::{CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope};
use murmur_router::cloud::{valid_custom_model_id, CustomCleanupConfig};
use murmur_router::SecretString;

use crate::budget;
use crate::cleaner::{clean_error, read_stream, CleanupCall, PREFILL_INTERVAL};
use crate::http::{self, CloudError};

/// LiteLLM's per-response cost header (USD).
pub const COST_HEADER: &str = "x-litellm-response-cost";
/// The same before LiteLLM's own adjustments; streamed answers send only
/// this one.
pub const COST_HEADER_ORIGINAL: &str = "x-litellm-response-cost-original";
/// At most this many ids are kept from `/models`.
pub const MAX_MODELS: usize = 1000;
/// Cap on a `/models` body.
const MODELS_BODY_CAP: usize = 2 * 1024 * 1024;

/// A plain-HTTP-capable client: http or https, no redirects, no referer.
fn client(connect: Duration) -> http::Client {
    http::client_with(false, connect)
}

/// `RequestBuilder` with the bearer key, when there is one.
fn auth(rb: http::RequestBuilder, key: Option<&SecretString>) -> http::RequestBuilder {
    match key.filter(|k| !k.is_empty()) {
        Some(k) => rb.bearer_auth(k.expose()),
        None => rb,
    }
}

/// Cleaner for the custom server: `murmur_core::cleanup::Cleaner`, wrapped by
/// the router's gate like the OpenRouter one.
pub struct CustomCleaner {
    /// Normalised API root, or why the configured URL is unusable.
    base: Result<String, String>,
    cfg: CustomCleanupConfig,
    scope: PromptScope,
    key: Option<SecretString>,
    http: http::Client,
    last: Mutex<Option<CleanupCall>>,
    last_prefill: Arc<Mutex<Option<Instant>>>,
}

impl std::fmt::Debug for CustomCleaner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomCleaner")
            .field("model", &self.cfg.model)
            .finish_non_exhaustive()
    }
}

impl CustomCleaner {
    pub fn new(cfg: &CustomCleanupConfig, scope: PromptScope, key: Option<SecretString>) -> Self {
        Self {
            base: cfg.api_base(),
            http: client(Duration::from_millis(
                cfg.connect_timeout_ms.clamp(200, 10_000),
            )),
            cfg: cfg.clone(),
            scope,
            key: key.filter(|k| !k.is_empty()),
            last: Mutex::new(None),
            last_prefill: Arc::new(Mutex::new(None)),
        }
    }

    fn model_id(&self) -> &str {
        self.cfg.model.trim()
    }

    pub fn last_call(&self) -> Option<CleanupCall> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The request body (exposed for contract tests): plain OpenAI chat
    /// completions. `max_tokens` rather than `max_completion_tokens` and no
    /// `seed`: older OpenAI-compatible servers know only the former, and
    /// LiteLLM refuses parameters a backend lacks unless `drop_params` is set.
    pub fn request_body(&self, prompt: &CleanPrompt) -> Value {
        let messages: Vec<Value> = prompt
            .messages()
            .into_iter()
            .map(|(role, content)| json!({"role": role, "content": content}))
            .collect();
        json!({
            "model": self.model_id(),
            "messages": messages,
            "temperature": 0,
            "max_tokens": budget::cleanup_max_tokens(&prompt.transcript),
            "stream": true,
        })
    }

    /// One streamed cleanup call; the call record comes back either way.
    pub fn run(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
    ) -> (Result<String, CloudError>, CleanupCall) {
        let t0 = Instant::now();
        let mut call = CleanupCall {
            model: self.model_id().to_string(),
            ..Default::default()
        };
        let res = self.stream(prompt, deadline, cancel, t0, &mut call);
        call.latency_ms = Some(t0.elapsed().as_millis() as u32);
        if let Err(e) = &res {
            call.error = Some(e.code());
        }
        // An OpenAI completion id is not an OpenRouter generation: nothing
        // is looked up later, and the gate must not price it.
        call.generation_id = None;
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(call.clone());
        (res, call)
    }

    fn stream(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
        t0: Instant,
        call: &mut CleanupCall,
    ) -> Result<String, CloudError> {
        let base = self
            .base
            .as_ref()
            .map_err(|_| CloudError::Connect("bad_base_url".into()))?;
        if !valid_custom_model_id(self.model_id()) {
            return Err(CloudError::BadRequest {
                status: 0,
                code: Some("bad_model_id".into()),
            });
        }
        if cancel.is_set() {
            return Err(CloudError::Timeout);
        }
        let rb = self
            .http
            .post(format!("{base}/chat/completions"))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&self.request_body(prompt));
        let resp = http::send(auth(rb, self.key.as_ref()), deadline)?;
        let header_cost = cost_header(resp.headers());
        let r = read_stream(resp, deadline, cancel, t0, call);
        if call.cost_usd.is_none() {
            call.cost_usd = header_cost;
        }
        r
    }
}

/// `x-litellm-response-cost` as a finite, non-negative USD amount, else
/// (header absent, as on LiteLLM's streamed answers) its
/// `x-litellm-response-cost-original`.
pub fn cost_header(h: &reqwest::header::HeaderMap) -> Option<f64> {
    let name = if h.contains_key(COST_HEADER) {
        COST_HEADER
    } else {
        COST_HEADER_ORIGINAL
    };
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|c| c.is_finite() && *c >= 0.0)
}

impl Cleaner for CustomCleaner {
    fn clean(&self, prompt: &CleanPrompt, deadline: Instant) -> Result<String, CleanError> {
        self.clean_cancellable(prompt, deadline, &AbortFlag::default())
    }

    fn clean_cancellable(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
    ) -> Result<String, CleanError> {
        self.run(prompt, deadline, cancel)
            .0
            .map_err(|e| clean_error(&e))
    }

    /// Sends nothing with content: warms the connection with `GET /models`
    /// at most once per [`PREFILL_INTERVAL`].
    fn prefill(&self, _prompt: &CleanPrompt) {
        let Ok(base) = self.base.clone() else {
            return;
        };
        {
            let mut lp = self.last_prefill.lock().unwrap_or_else(|e| e.into_inner());
            if lp.is_some_and(|t| t.elapsed() < PREFILL_INTERVAL) {
                return;
            }
            *lp = Some(Instant::now());
        }
        let (http, key) = (self.http.clone(), self.key.clone());
        std::thread::spawn(move || {
            let rb = http.get(format!("{base}/models"));
            let _ = http::send(
                auth(rb, key.as_ref()),
                Instant::now() + Duration::from_secs(5),
            );
        });
    }

    fn prompt_scope(&self) -> PromptScope {
        self.scope
    }

    fn engine(&self) -> &'static str {
        "cloud"
    }

    fn model(&self) -> Option<String> {
        Some(self.model_id().to_string())
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(self.cfg.timeout_ms)
    }

    /// No price table for a self-hosted server.
    fn estimate_cost_usd(&self, _prompt: &CleanPrompt) -> Option<f64> {
        None
    }

    fn clean_metered(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
    ) -> (Result<String, CleanError>, CleanUsage) {
        let (r, call) = self.run(prompt, deadline, cancel);
        (r.map_err(|e| clean_error(&e)), call.usage())
    }

    fn info_json(&self) -> Option<String> {
        let c = self.last_call();
        Some(
            json!({
                "engine": "cloud",
                "provider": "custom",
                "model": self.model_id(),
                "last_cost_usd": c.as_ref().and_then(|c| c.cost_usd),
                "last_latency_ms": c.as_ref().and_then(|c| c.latency_ms),
                "ttft_ms": c.as_ref().and_then(|c| c.ttft_ms),
                "last_error": c.as_ref().and_then(|c| c.error),
            })
            .to_string(),
        )
    }
}

/// `GET {base}/models` on the custom server: the model ids it lists
/// (`data[].id`), sorted, without duplicates or unusable ids. Also the
/// settings page's "Test connection".
pub fn list_models(
    base_url: &str,
    key: Option<&SecretString>,
    timeout: Duration,
) -> Result<Vec<String>, CloudError> {
    let base = murmur_router::cloud::normalize_custom_base_url(base_url)
        .map_err(|_| CloudError::Connect("bad_base_url".into()))?;
    let deadline = Instant::now() + timeout;
    let rb = client(timeout.min(Duration::from_secs(5))).get(format!("{base}/models"));
    let resp = http::send(auth(rb, key), deadline)?;
    let v = http::json_capped(resp, MODELS_BODY_CAP)?;
    parse_models(&v)
}

/// The settings pages' model list / "Test connection" result, the same
/// JSON on every shell: `{"models": [ids]}` or `{"error": code,
/// "message": line}` (never the URL or the key).
pub fn models_json(r: Result<Vec<String>, CloudError>) -> Value {
    match r {
        Ok(models) => json!({ "models": models }),
        Err(e) => json!({ "error": e.code(), "message": describe(&e) }),
    }
}

/// D85: [`models_json`] plus `"speech_models"`, the ids among them that
/// can transcribe ([`speech_models`]): one `/models` call, then one
/// `/model/info` call when the first worked.
pub fn models_with_speech_json(
    base_url: &str,
    key: Option<&SecretString>,
    timeout: Duration,
) -> Value {
    let t0 = Instant::now();
    match list_models(base_url, key, timeout) {
        Ok(models) => {
            let left = timeout
                .saturating_sub(t0.elapsed())
                .max(Duration::from_secs(1));
            let speech = speech_models(base_url, key, left, &models);
            json!({ "models": models, "speech_models": speech })
        }
        Err(e) => models_json(Err(e)),
    }
}

/// D85: the speech-to-text models among `listed` (the server's `/models`
/// ids). LiteLLM says which ones: `GET {base}/model/info` gives each
/// model's `model_info.mode`, `audio_transcription` for speech. A server
/// without that endpoint (vLLM, Ollama, an older proxy) or an answer that
/// does not parse falls back to the names: ids containing `whisper` or
/// `transcribe`. Sorted, without duplicates; never an error.
pub fn speech_models(
    base_url: &str,
    key: Option<&SecretString>,
    timeout: Duration,
    listed: &[String],
) -> Vec<String> {
    let info = murmur_router::cloud::normalize_custom_base_url(base_url)
        .map_err(|_| CloudError::Connect("bad_base_url".into()))
        .and_then(|base| {
            let deadline = Instant::now() + timeout;
            let rb = client(timeout.min(Duration::from_secs(5))).get(format!("{base}/model/info"));
            let resp = http::send(auth(rb, key), deadline)?;
            http::json_capped(resp, MODELS_BODY_CAP)
        });
    match info.ok().and_then(|v| parse_speech_models(&v)) {
        Some(ids) => ids,
        None => speech_like(listed),
    }
}

/// LiteLLM's `/model/info` (`{"data": [{"model_name", "model_info":
/// {"mode"}}]}`) → the names whose mode is `audio_transcription`; `None`
/// when the body is not that shape.
pub fn parse_speech_models(v: &Value) -> Option<Vec<String>> {
    let arr = v.get("data").and_then(Value::as_array)?;
    let mut ids: Vec<String> = arr
        .iter()
        .filter(|m| {
            m.pointer("/model_info/mode").and_then(Value::as_str) == Some("audio_transcription")
        })
        .filter_map(|m| m.get("model_name").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|id| valid_custom_model_id(id))
        .collect();
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_MODELS);
    Some(ids)
}

/// Ids that look like speech-to-text models by name (`whisper`,
/// `transcribe`), for servers that do not say.
pub fn speech_like(ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = ids
        .iter()
        .filter(|id| {
            let l = id.to_ascii_lowercase();
            l.contains("whisper") || l.contains("transcribe")
        })
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The fixed sentence "Test cleanup" sends (never user text).
pub const TEST_SAMPLE: &str =
    "um so i think we should uh meet on tuesday no wait wednesday at three";

/// "Test cleanup" result on the custom server.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct CleanupTest {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub model: String,
}

/// "Test cleanup" end to end: one streamed chat completion of
/// [`TEST_SAMPLE`] on `cfg` with `key` (blocking, ≤ 2× the timeout).
pub fn test_cleanup(cfg: &CustomCleanupConfig, key: Option<SecretString>) -> CleanupTest {
    let model = cfg.model.trim().to_string();
    let c = CustomCleaner::new(cfg, PromptScope::default(), key);
    let prompt = murmur_core::cleanup::build_prompt(
        TEST_SAMPLE,
        murmur_core::config::CleanupLevel::Light,
        murmur_core::config::Style::default(),
        &murmur_core::config::FieldContext::default(),
        &murmur_core::dictionary::Dictionary::default(),
    );
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_millis(cfg.timeout_ms.max(800) * 2);
    let (r, call) = c.run(&prompt, deadline, &AbortFlag::default());
    let latency_ms = Some(t0.elapsed().as_millis() as u32);
    match r {
        Ok(text) => CleanupTest {
            ok: true,
            text: Some(text),
            latency_ms,
            cost_usd: call.cost_usd,
            model,
            ..Default::default()
        },
        Err(e) => CleanupTest {
            error: Some(e.code().to_string()),
            message: Some(describe(&e)),
            latency_ms,
            model,
            ..Default::default()
        },
    }
}

/// `{"data": [{"id": "…"}, …]}` → ids (also a bare array of such objects).
pub fn parse_models(v: &Value) -> Result<Vec<String>, CloudError> {
    let arr = v
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| v.as_array())
        .ok_or(CloudError::Malformed)?;
    let mut ids: Vec<String> = arr
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|id| valid_custom_model_id(id))
        .collect();
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_MODELS);
    Ok(ids)
}

/// A provider-neutral line for the settings page (the [`CloudError`]
/// `Display` names OpenRouter). Never a URL, key or body.
pub fn describe(e: &CloudError) -> String {
    match e {
        CloudError::Connect(k) if k == "bad_base_url" => {
            "the server address is not a valid http(s) URL".into()
        }
        CloudError::Connect(_) => "cannot reach the server".into(),
        CloudError::Auth => "the server rejected the key, or needs one".into(),
        CloudError::BadRequest { code: Some(c), .. } if c == http::UNKNOWN_MODEL => {
            "the server has no model by that name: pick one from its list".into()
        }
        CloudError::Quota => "the server says the key is over its budget".into(),
        CloudError::RateLimited { .. } => "the server is busy or rate limited".into(),
        CloudError::Timeout => "the server did not answer in time".into(),
        CloudError::Malformed => "the server's answer is not OpenAI-compatible".into(),
        CloudError::Server(s) => format!("server error (HTTP {s})"),
        CloudError::BadRequest {
            status,
            code: Some(c),
        } if *status > 0 => format!("the server refused the request (HTTP {status}, {c})"),
        CloudError::BadRequest { status, .. } if *status > 0 => {
            format!("the server refused the request (HTTP {status})")
        }
        other => other.code().replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_parse() {
        let v = json!({"object": "list", "data": [
            {"id": "gpt-4o", "object": "model"},
            {"id": "Qwen/Qwen2.5-7B-Instruct"},
            {"id": "gpt-4o"},
            {"id": "has space"},
            {"name": "no id"},
        ]});
        assert_eq!(
            parse_models(&v).unwrap(),
            ["Qwen/Qwen2.5-7B-Instruct", "gpt-4o"]
        );
        assert_eq!(parse_models(&json!([{"id": "a"}])).unwrap(), ["a"]);
        assert_eq!(parse_models(&json!({"x": 1})), Err(CloudError::Malformed));
    }

    #[test]
    fn speech_models_from_litellm_model_info() {
        let v = json!({"data": [
            {"model_name": "local-whisper", "model_info": {"mode": "audio_transcription"}},
            {"model_name": "local-cleanup", "model_info": {"mode": "chat"}},
            {"model_name": "no-info"},
            {"model_name": "local-whisper", "model_info": {"mode": "audio_transcription"}},
        ]});
        assert_eq!(parse_speech_models(&v).unwrap(), ["local-whisper"]);
        assert_eq!(
            parse_speech_models(&json!({"data": []})).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(parse_speech_models(&json!({"detail": "Not Found"})), None);
    }

    #[test]
    fn speech_models_by_name_when_the_server_does_not_say() {
        let ids: Vec<String> = [
            "gpt-4o-mini",
            "whisper-1",
            "gpt-4o-mini-transcribe",
            "Systran/faster-Whisper-small",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            speech_like(&ids),
            [
                "Systran/faster-Whisper-small",
                "gpt-4o-mini-transcribe",
                "whisper-1"
            ]
        );
    }

    #[test]
    fn cost_header_is_checked() {
        let mut h = reqwest::header::HeaderMap::new();
        assert_eq!(cost_header(&h), None);
        h.insert(COST_HEADER, "0.00042".parse().unwrap());
        assert_eq!(cost_header(&h), Some(0.00042));
        h.insert(COST_HEADER, "-1".parse().unwrap());
        assert_eq!(cost_header(&h), None);
        h.insert(COST_HEADER, "NaN".parse().unwrap());
        assert_eq!(cost_header(&h), None);
        // Streamed answers carry only the `-original` header.
        let mut s = reqwest::header::HeaderMap::new();
        s.insert(COST_HEADER_ORIGINAL, "0.0003".parse().unwrap());
        assert_eq!(cost_header(&s), Some(0.0003));
        s.insert(COST_HEADER, "0.0001".parse().unwrap());
        assert_eq!(cost_header(&s), Some(0.0001), "the main header wins");
    }

    #[test]
    fn descriptions_never_name_openrouter() {
        for e in [
            CloudError::Connect("connect".into()),
            CloudError::Connect("bad_base_url".into()),
            CloudError::Auth,
            CloudError::Quota,
            CloudError::Timeout,
            CloudError::Malformed,
            CloudError::Server(502),
            CloudError::BadRequest {
                status: 404,
                code: None,
            },
            CloudError::NoPrivateRoute,
        ] {
            assert!(!describe(&e).contains("OpenRouter"), "{e:?}");
        }
    }
}
