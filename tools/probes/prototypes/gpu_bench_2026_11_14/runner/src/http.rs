//! A `Cleaner` that sends the core's prompt to a llama.cpp `llama-server`
//! and decodes the way the murmur-llm plugin does (crates/murmur-llm
//! engine.rs): the same messages (system, few-shot turns, user), greedy,
//! at most 2 × transcript tokens + 32 new tokens, the plugin's stop
//! strings. The server keeps the shared prompt prefix in its KV cache
//! between requests (`cache_prompt`), as the plugin does. Drafting is the
//! server's flag (`--spec-type`), not set per request.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use murmur_core::asr::AbortFlag;
use murmur_core::cleanup::{CleanError, CleanPrompt, Cleaner};
use serde_json::{json, Value};

/// crates/murmur-llm/src/engine.rs STOP_STRINGS.
const STOP: &[&str] = &[
    "<|im_end|>",
    "<|im_start|>",
    "<|eot_id|>",
    "<|end_of_text|>",
    "<|endoftext|>",
    "</s>",
    "\nTRANSCRIPT:",
    "\nEXAMPLE",
    "\nAPP:",
];

pub struct HttpCleaner {
    base: String,
    last: Mutex<Value>,
}

impl HttpCleaner {
    pub fn new(base: &str) -> Self {
        HttpCleaner {
            base: base.trim_end_matches('/').to_string(),
            last: Mutex::new(Value::Null),
        }
    }

    fn tokens(&self, text: &str) -> Result<usize, CleanError> {
        let v: Value = ureq::post(&format!("{}/tokenize", self.base))
            .timeout(Duration::from_secs(30))
            .send_json(json!({"content": text}))
            .map_err(|e| CleanError::Backend(format!("tokenize: {e}")))?
            .into_json()
            .map_err(|e| CleanError::Backend(format!("tokenize json: {e}")))?;
        Ok(v["tokens"].as_array().map_or(0, |a| a.len()))
    }
}

impl Cleaner for HttpCleaner {
    fn clean(&self, p: &CleanPrompt, deadline: Instant) -> Result<String, CleanError> {
        let mut messages = vec![json!({"role": "system", "content": p.system})];
        for (u, a) in &p.examples {
            messages.push(json!({"role": "user", "content": u}));
            messages.push(json!({"role": "assistant", "content": a}));
        }
        messages.push(json!({"role": "user", "content": p.user}));
        let max_tokens = 2 * self.tokens(&p.transcript)? + 32;
        let left = deadline.saturating_duration_since(Instant::now());
        let t = Instant::now();
        let v: Value = ureq::post(&format!("{}/v1/chat/completions", self.base))
            .timeout(left.max(Duration::from_millis(1)))
            .send_json(json!({
                "messages": messages,
                "max_tokens": max_tokens,
                "temperature": 0.0,
                "top_k": 1,
                "stop": STOP,
                "cache_prompt": true,
            }))
            .map_err(|e| CleanError::Backend(format!("http: {e}")))?
            .into_json()
            .map_err(|e| CleanError::Backend(format!("json: {e}")))?;
        let http_ms = t.elapsed().as_millis() as u64;
        let tm = &v["timings"];
        let cache = tm["cache_n"].as_u64().unwrap_or(0);
        let prompt_n = tm["prompt_n"].as_u64().unwrap_or(0);
        *self.last.lock().unwrap() = json!({
            "prompt_tokens": cache + prompt_n,
            "prefill_tokens": prompt_n,
            "prefill_ms": tm["prompt_ms"],
            "gen_tokens": tm["predicted_n"],
            "gen_ms": tm["predicted_ms"],
            "draft_n": tm["draft_n"],
            "draft_accepted": tm["draft_n_accepted"],
            "http_ms": http_ms,
            "finish": v["choices"][0]["finish_reason"],
        });
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.trim().to_string())
            .ok_or_else(|| CleanError::Backend("no content".into()))
    }

    fn clean_cancellable(
        &self,
        p: &CleanPrompt,
        d: Instant,
        _c: &AbortFlag,
    ) -> Result<String, CleanError> {
        self.clean(p, d)
    }

    fn info_json(&self) -> Option<String> {
        Some(json!({"last": *self.last.lock().unwrap()}).to_string())
    }

    fn engine(&self) -> &'static str {
        "server"
    }
}
