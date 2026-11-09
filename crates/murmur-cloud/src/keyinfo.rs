//! `GET /key` (D28 §1.5, §2.6): the key's credit limit and usage, for the
//! "Check key" button and the credits row. Label and limits are not
//! secrets; the key itself never appears in [`KeyInfo`].

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use murmur_router::SecretString;

use crate::api::Api;
use crate::http::{self, CloudError};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyInfo {
    pub label: Option<String>,
    /// Credit limit in USD; `None` = no limit (the UI warns).
    pub limit: Option<f64>,
    pub limit_remaining: Option<f64>,
    pub limit_reset: Option<String>,
    pub usage: Option<f64>,
    pub usage_daily: Option<f64>,
    pub usage_weekly: Option<f64>,
    pub usage_monthly: Option<f64>,
    pub is_free_tier: bool,
    /// `free_model_daily_requests.remaining`.
    pub free_requests_remaining: Option<u64>,
    /// When it was fetched (unix ms).
    pub checked_at_ms: u64,
}

impl KeyInfo {
    /// "This key has no credit limit; OpenRouter recommends setting one."
    pub fn warn_no_limit(&self) -> bool {
        self.limit.is_none()
    }

    pub fn from_json(v: &Value) -> Result<KeyInfo, CloudError> {
        let d = v.get("data").ok_or(CloudError::Malformed)?;
        let f = |k: &str| d.get(k).and_then(Value::as_f64);
        Ok(KeyInfo {
            // A label is user text, shown as is; bound its size.
            label: d
                .get("label")
                .and_then(Value::as_str)
                .map(|s| s.chars().take(80).collect()),
            limit: f("limit"),
            limit_remaining: f("limit_remaining"),
            limit_reset: d
                .get("limit_reset")
                .and_then(Value::as_str)
                .map(|s| s.chars().take(32).collect()),
            usage: f("usage"),
            usage_daily: f("usage_daily"),
            usage_weekly: f("usage_weekly"),
            usage_monthly: f("usage_monthly"),
            is_free_tier: d
                .get("is_free_tier")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            free_requests_remaining: d
                .pointer("/free_model_daily_requests/remaining")
                .and_then(Value::as_u64),
            checked_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
        })
    }
}

/// `GET /key` on the production API (5 s).
pub fn fetch(key: &SecretString) -> Result<KeyInfo, CloudError> {
    fetch_with(&Api::new(), key, Instant::now() + Duration::from_secs(5))
}

pub fn fetch_with(api: &Api, key: &SecretString, deadline: Instant) -> Result<KeyInfo, CloudError> {
    if key.is_empty() {
        return Err(CloudError::Auth);
    }
    let v = http::json(http::send(
        api.client().get(api.url("/key")).bearer_auth(key.expose()),
        deadline,
    )?)?;
    KeyInfo::from_json(&v)
}
