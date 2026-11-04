//! Opt-in cloud text cleanup (D28 §2.5): [`OpenRouterCleaner`] implements
//! `flow_core::cleanup::Cleaner`, so flow-core's prompt v2, hard timeout,
//! output unwrapping, divergence threshold, guard and deterministic
//! fallback apply unchanged.
//!
//! One streamed `POST /chat/completions` per cleanup: aborting the stream
//! (timeout / cancel) stops processing and billing at supported providers.
//! `provider` = `{zdr: true, data_collection: "deny", sort: "latency",
//! max_price, allow_fallbacks}` by default; no `user`, `session_id` or
//! attribution headers.

use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use flow_core::asr::AbortFlag;
use flow_core::cleanup::{CleanError, CleanPrompt, CleanUsage, Cleaner, PromptScope};
use flow_router::SecretString;

use crate::api::Api;
use crate::budget;
use crate::catalog::{self, ModelInfo};
use crate::generation::{self, CostSink, CostUse};
use crate::http::{self, CloudError};
use crate::keyinfo::{self, KeyInfo};
use crate::settings::{CleanupScope, CleanupSettings};

/// `prefill` warms the connection with `GET /key` at most this often.
pub const PREFILL_INTERVAL: Duration = Duration::from_secs(60);

/// What the last cleanup call did (no text, no key).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CleanupCall {
    pub model: String,
    pub cost_usd: Option<f64>,
    pub cost_estimated: bool,
    pub generation_id: Option<String>,
    pub ttft_ms: Option<u32>,
    pub latency_ms: Option<u32>,
    pub finish_reason: Option<String>,
    /// `Failure` code when it failed (`no_private_route`, `timeout`, …).
    pub error: Option<&'static str>,
}

pub struct OpenRouterCleaner {
    api: Api,
    settings: CleanupSettings,
    key: SecretString,
    info: Option<ModelInfo>,
    last: Mutex<Option<CleanupCall>>,
    last_prefill: Mutex<Option<Instant>>,
    key_info: Arc<Mutex<Option<KeyInfo>>>,
    cost_sink: Option<CostSink>,
    lookup_delay: Duration,
}

impl std::fmt::Debug for OpenRouterCleaner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterCleaner")
            .field("model", &self.settings.model)
            .finish_non_exhaustive()
    }
}

impl OpenRouterCleaner {
    /// The model's catalogue entry comes from the registered store, else
    /// the baked catalogue ([`catalog::resolve`]).
    pub fn new(settings: CleanupSettings, key: SecretString) -> Self {
        let info = catalog::resolve(settings.model.trim());
        let api = Api::with_connect_timeout(Duration::from_millis(
            settings.connect_timeout_ms.clamp(200, 10_000),
        ));
        Self {
            api,
            settings,
            key,
            info,
            last: Mutex::new(None),
            last_prefill: Mutex::new(None),
            key_info: Arc::new(Mutex::new(None)),
            cost_sink: None,
            lookup_delay: generation::LOOKUP_DELAY,
        }
    }

    /// Override the catalogue entry (tests, a fresh `lookup_model`).
    pub fn with_model_info(mut self, info: Option<ModelInfo>) -> Self {
        self.info = info;
        self
    }

    pub fn with_api(mut self, api: Api) -> Self {
        self.api = api;
        self
    }

    /// Delay before a deferred `/generation` lookup (default 30 s).
    pub fn with_cost_lookup_delay(mut self, d: Duration) -> Self {
        self.lookup_delay = d;
        self
    }

    /// Where deferred `/generation` costs go (router usage store).
    pub fn with_cost_sink(mut self, sink: CostSink) -> Self {
        self.cost_sink = Some(sink);
        self
    }

    pub fn settings(&self) -> &CleanupSettings {
        &self.settings
    }

    pub fn model_info(&self) -> Option<&ModelInfo> {
        self.info.as_ref()
    }

    /// The prompt scope the user allowed (default: nothing beyond the
    /// transcript, category, style and dictionary).
    pub fn scope(&self) -> CleanupScope {
        self.settings.scope()
    }

    pub fn last_call(&self) -> Option<CleanupCall> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Last `/key` result fetched by [`Cleaner::prefill`].
    pub fn key_info(&self) -> Option<KeyInfo> {
        self.key_info
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn max_tokens(prompt: &CleanPrompt) -> u32 {
        budget::cleanup_max_tokens(&prompt.transcript)
    }

    /// Pre-flight cost estimate with headroom (§2.6 rule 1); `None` =
    /// unknown (free-text id or unpriced).
    pub fn estimate_cost_usd(&self, prompt: &CleanPrompt) -> Option<f64> {
        let tokens: u32 = prompt
            .messages()
            .iter()
            .map(|(_, c)| budget::est_tokens(c) + 4)
            .sum();
        budget::estimate_cleanup(self.info.as_ref()?, tokens, Self::max_tokens(prompt))
    }

    /// The request body (exposed for contract tests).
    pub fn request_body(&self, prompt: &CleanPrompt) -> Value {
        let messages: Vec<Value> = prompt
            .messages()
            .into_iter()
            .map(|(role, content)| json!({"role": role, "content": content}))
            .collect();
        let mut body = json!({
            "model": self.settings.model.trim(),
            "messages": messages,
            "temperature": 0,
            "seed": 0,
            "max_completion_tokens": Self::max_tokens(prompt),
            "stream": true,
        });
        if let Some(m) = &self.info {
            if let Some(e) = m.reasoning_effort() {
                body["reasoning"] = json!({"effort": e});
            }
            if m.supports.include_reasoning {
                body["include_reasoning"] = json!(false);
            }
        }
        let mut p = self.settings.privacy.provider_json();
        let sort = self.settings.sort.trim();
        if matches!(sort, "latency" | "throughput" | "price") {
            p.insert("sort".into(), sort.into());
        }
        if let Some(mp) = self.info.as_ref().and_then(budget::max_price) {
            p.insert("max_price".into(), mp.json());
        }
        p.insert(
            "allow_fallbacks".into(),
            self.settings.allow_fallbacks.into(),
        );
        body["provider"] = Value::Object(p);
        body
    }

    /// One streamed cleanup call. The call record comes back whether or
    /// not it worked.
    pub fn run(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
    ) -> (Result<String, CloudError>, CleanupCall) {
        let t0 = Instant::now();
        let mut call = CleanupCall {
            model: self.settings.model.trim().to_string(),
            ..Default::default()
        };
        let res = self.stream(prompt, deadline, cancel, t0, &mut call);
        call.latency_ms = Some(t0.elapsed().as_millis() as u32);
        if let Err(e) = &res {
            call.error = Some(e.code());
        }
        if call.cost_usd.is_none() && call.generation_id.is_some() {
            // Started but no usage chunk (cancel / timeout / error).
            call.cost_estimated = true;
            call.cost_usd = self.estimate_cost_usd(prompt);
            if let (Some(sink), Some(id)) = (&self.cost_sink, &call.generation_id) {
                generation::schedule(
                    self.api.clone(),
                    self.key.clone(),
                    CostUse::Cleanup,
                    id.clone(),
                    call.cost_usd,
                    self.lookup_delay,
                    sink.clone(),
                );
            }
        }
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
        if self.key.is_empty() {
            return Err(CloudError::Auth);
        }
        if cancel.is_set() {
            return Err(CloudError::Timeout);
        }
        let body = self.request_body(prompt);
        let resp = http::send(
            self.api
                .client()
                .post(self.api.url("/chat/completions"))
                .bearer_auth(self.key.expose())
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&body),
            deadline,
        )?;
        if let Some(id) = resp
            .headers()
            .get("x-generation-id")
            .and_then(|h| h.to_str().ok())
            .filter(|s| generation::valid_id(s))
        {
            call.generation_id = Some(id.to_string());
        }
        read_stream(resp, deadline, cancel, t0, call)
    }
}

/// Read a 2xx `/chat/completions` answer: SSE (streamed) or plain JSON
/// (a server that ignored `stream: true`). Shared by the OpenRouter and the
/// custom-server cleaner (D52).
pub(crate) fn read_stream(
    resp: http::Response,
    deadline: Instant,
    cancel: &AbortFlag,
    t0: Instant,
    call: &mut CleanupCall,
) -> Result<String, CloudError> {
    let is_sse = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|c| c.contains("event-stream"));
    if !is_sse {
        // A provider that ignored `stream: true`: plain JSON.
        let v = http::json(resp)?;
        return finish_json(&v, call);
    }
    // Lines are read on a helper thread so cancel / deadline are seen
    // within a few ms; dropping the receiver makes that thread drop the
    // response (closes the connection = provider stops billing).
    let (tx, rx) = mpsc::sync_channel::<Option<String>>(64);
    std::thread::spawn(move || {
        let mut r = BufReader::new(resp);
        loop {
            let mut line = String::new();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
                Ok(_) => {
                    if tx.send(Some(line)).is_err() {
                        return;
                    }
                }
            }
        }
    });
    let mut sse = SseState::default();
    loop {
        if cancel.is_set() || Instant::now() >= deadline {
            sse.apply(call);
            return Err(CloudError::Timeout);
        }
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(Some(line)) => {
                let had = !sse.text.is_empty();
                match sse.line(&line) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(e) => {
                        sse.apply(call);
                        return Err(e);
                    }
                }
                if !had && !sse.text.is_empty() && call.ttft_ms.is_none() {
                    call.ttft_ms = Some(t0.elapsed().as_millis() as u32);
                }
            }
            // EOF without [DONE]: take what arrived.
            Ok(None) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    sse.apply(call);
    if sse.finish.is_none() && sse.text.is_empty() {
        return Err(CloudError::Malformed);
    }
    Ok(sse.text)
}

fn finish_json(v: &Value, call: &mut CleanupCall) -> Result<String, CloudError> {
    if v.get("error").is_some() {
        return Err(CloudError::Server(502));
    }
    let c = v.pointer("/choices/0").ok_or(CloudError::Malformed)?;
    let finish = c.get("finish_reason").and_then(Value::as_str);
    call.finish_reason = finish.map(str::to_string);
    call.cost_usd = v.pointer("/usage/cost").and_then(Value::as_f64);
    if call.generation_id.is_none() {
        call.generation_id = v
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| generation::valid_id(s))
            .map(str::to_string);
    }
    if finish == Some("error") {
        return Err(CloudError::Server(502));
    }
    Ok(c.pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string())
}

/// Incremental SSE parser (OpenRouter streaming, §1.3).
#[derive(Debug, Default)]
pub struct SseState {
    pub text: String,
    pub finish: Option<String>,
    pub cost: Option<f64>,
    pub id: Option<String>,
}

impl SseState {
    /// Feed one line. `Ok(true)` = `[DONE]`. A mid-stream `error` event or
    /// `finish_reason: "error"` is a provider error. Comment lines
    /// (`: OPENROUTER PROCESSING`) and non-data fields are ignored.
    pub fn line(&mut self, line: &str) -> Result<bool, CloudError> {
        let l = line.trim_end_matches(['\r', '\n']);
        let Some(d) = l.strip_prefix("data:") else {
            return Ok(false);
        };
        let d = d.trim();
        if d == "[DONE]" {
            return Ok(true);
        }
        let Ok(v) = serde_json::from_str::<Value>(d) else {
            return Ok(false);
        };
        if self.id.is_none() {
            self.id = v
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| generation::valid_id(s))
                .map(str::to_string);
        }
        if let Some(c) = v.pointer("/usage/cost").and_then(Value::as_f64) {
            self.cost = Some(c);
        }
        if v.get("error").is_some() {
            self.finish = Some("error".into());
            return Err(CloudError::Server(502));
        }
        if let Some(c) = v.pointer("/choices/0") {
            if let Some(t) = c.pointer("/delta/content").and_then(Value::as_str) {
                self.text.push_str(t);
            }
            if let Some(f) = c.get("finish_reason").and_then(Value::as_str) {
                self.finish = Some(f.to_string());
                if f == "error" {
                    return Err(CloudError::Server(502));
                }
            }
        }
        Ok(false)
    }

    fn apply(&self, call: &mut CleanupCall) {
        if self.cost.is_some() {
            call.cost_usd = self.cost;
        }
        if call.generation_id.is_none() {
            call.generation_id = self.id.clone();
        }
        call.finish_reason = self.finish.clone();
    }
}

/// `CleanError` for a failed call: the `Failure` code only. A fast failure
/// before any output (connect, auth, quota, no private route, busy) is
/// `Unavailable`, so a `RoutedCleaner` may try the local model.
pub fn clean_error(e: &CloudError) -> CleanError {
    if e.is_fast_unavailable() {
        CleanError::Unavailable(e.code().to_string())
    } else {
        CleanError::Backend(e.code().to_string())
    }
}

impl CleanupCall {
    pub fn usage(&self) -> CleanUsage {
        CleanUsage {
            engine: Some("cloud".into()),
            model: Some(self.model.clone()),
            cost_usd: self.cost_usd.map(flow_core::Usd),
            cost_estimated: self.cost_estimated,
            generation_id: self.generation_id.clone(),
            ttft_ms: self.ttft_ms,
            latency_ms: self.latency_ms,
        }
    }
}

impl Cleaner for OpenRouterCleaner {
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

    /// Sends nothing with content: warms the pooled TLS connection with
    /// `GET /key` (≤ once per 60 s), which also refreshes the credits row.
    fn prefill(&self, _prompt: &CleanPrompt) {
        {
            let mut lp = self.last_prefill.lock().unwrap_or_else(|e| e.into_inner());
            if lp.is_some_and(|t| t.elapsed() < PREFILL_INTERVAL) || self.key.is_empty() {
                return;
            }
            *lp = Some(Instant::now());
        }
        let (api, key, slot) = (self.api.clone(), self.key.clone(), self.key_info.clone());
        std::thread::spawn(move || {
            if let Ok(k) = keyinfo::fetch_with(&api, &key, Instant::now() + Duration::from_secs(5))
            {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(k);
            }
        });
    }

    fn prompt_scope(&self) -> PromptScope {
        let s = self.scope();
        PromptScope {
            surrounding_text: s.surrounding_text,
            screen_words: s.screen_words,
            app_id: s.app_id,
        }
    }

    fn engine(&self) -> &'static str {
        "cloud"
    }

    fn model(&self) -> Option<String> {
        Some(self.settings.model.trim().to_string())
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(self.settings.timeout_ms)
    }

    fn estimate_cost_usd(&self, prompt: &CleanPrompt) -> Option<f64> {
        OpenRouterCleaner::estimate_cost_usd(self, prompt)
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
                "model": self.settings.model.trim(),
                "last_cost_usd": c.as_ref().and_then(|c| c.cost_usd),
                "last_cost_estimated": c.as_ref().map(|c| c.cost_estimated),
                "last_generation_id": c.as_ref().and_then(|c| c.generation_id.clone()),
                "last_latency_ms": c.as_ref().and_then(|c| c.latency_ms),
                "ttft_ms": c.as_ref().and_then(|c| c.ttft_ms),
                "last_error": c.as_ref().and_then(|c| c.error),
            })
            .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser() {
        let mut s = SseState::default();
        for l in [
            ": OPENROUTER PROCESSING\n",
            "\n",
            "data: {\"id\":\"gen-1\",\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n",
            "event: ping\n",
            "data: {\"id\":\"gen-1\",\"choices\":[{\"delta\":{\"content\":\"lo.\"},\"finish_reason\":\"stop\"}]}\n",
            "data: {\"id\":\"gen-1\",\"choices\":[],\"usage\":{\"cost\":0.0001}}\n",
        ] {
            assert_eq!(s.line(l), Ok(false), "{l}");
        }
        assert_eq!(s.line("data: [DONE]\n"), Ok(true));
        assert_eq!(s.text, "Hello.");
        assert_eq!(s.cost, Some(0.0001));
        assert_eq!(s.id.as_deref(), Some("gen-1"));
        let mut e = SseState::default();
        assert!(e
            .line("data: {\"error\":{\"code\":502,\"message\":\"x\"},\"choices\":[{\"finish_reason\":\"error\"}]}")
            .is_err());
    }

    #[test]
    fn errors_carry_only_codes() {
        assert_eq!(
            clean_error(&CloudError::NoPrivateRoute).to_string(),
            "no_private_route"
        );
        assert_eq!(clean_error(&CloudError::Auth).to_string(), "auth_failed");
    }
}
