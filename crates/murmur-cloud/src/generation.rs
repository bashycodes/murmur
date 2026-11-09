//! Deferred cost lookup (D28 §2.6 rule 4): when a response carried no
//! `usage.cost` (timeout, abandoned stream, mid-stream error), ask
//! `GET /generation?id=<X-Generation-Id>` once, later and off the
//! dictation path, and replace the estimate with `total_cost`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use murmur_router::SecretString;

use crate::api::Api;
use crate::http::{self, CloudError};

/// Default delay before the lookup (OpenRouter needs time to settle).
pub const LOOKUP_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerationInfo {
    pub id: String,
    pub total_cost: Option<f64>,
    pub cancelled: bool,
    pub model: Option<String>,
    pub native_tokens_prompt: Option<u64>,
    pub native_tokens_completion: Option<u64>,
}

/// Which use a deferred cost belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostUse {
    Stt,
    Cleanup,
}

/// A deferred cost result, handed to the shell/router's sink.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeferredCost {
    pub use_: CostUse,
    pub generation_id: String,
    /// What was recorded at the time (`cost_estimated: true`).
    pub estimated_usd: Option<f64>,
    /// `total_cost`, or `None` when the lookup failed.
    pub actual_usd: Option<f64>,
}

/// Receives deferred costs (the router replaces its estimate).
pub type CostSink = Arc<dyn Fn(DeferredCost) + Send + Sync>;

/// Generation ids look like `gen-…`; anything else is not sent.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

pub fn fetch(
    api: &Api,
    key: &SecretString,
    id: &str,
    deadline: Instant,
) -> Result<GenerationInfo, CloudError> {
    if !valid_id(id) {
        return Err(CloudError::BadRequest {
            status: 400,
            code: Some("generation_id".into()),
        });
    }
    let v = http::json(http::send(
        api.client()
            .get(api.url("/generation"))
            .query(&[("id", id)])
            .bearer_auth(key.expose()),
        deadline,
    )?)?;
    let d = v.get("data").ok_or(CloudError::Malformed)?;
    Ok(GenerationInfo {
        id: id.to_string(),
        total_cost: d.get("total_cost").and_then(Value::as_f64),
        cancelled: d.get("cancelled").and_then(Value::as_bool).unwrap_or(false),
        model: d.get("model").and_then(Value::as_str).map(str::to_string),
        native_tokens_prompt: d.get("native_tokens_prompt").and_then(Value::as_u64),
        native_tokens_completion: d.get("native_tokens_completion").and_then(Value::as_u64),
    })
}

/// Look `id` up once after `delay` on a background thread and hand the
/// result to `sink`. Returns at once.
pub fn schedule(
    api: Api,
    key: SecretString,
    use_: CostUse,
    id: String,
    estimated_usd: Option<f64>,
    delay: Duration,
    sink: CostSink,
) {
    if !valid_id(&id) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let actual = fetch(&api, &key, &id, Instant::now() + Duration::from_secs(10))
            .ok()
            .and_then(|g| g.total_cost);
        sink(DeferredCost {
            use_,
            generation_id: id,
            estimated_usd,
            actual_usd: actual,
        });
    });
}
