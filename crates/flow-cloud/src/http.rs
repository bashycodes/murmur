//! HTTP plumbing shared by the adapters: one pooled blocking client,
//! deadline-bounded requests, status → [`CloudError`] mapping, the retry
//! rules of docs/cloud-stt.md §3.2.
//!
//! Nothing here ever formats a request, a header, a URL or a response body
//! into an error: errors carry the HTTP status and a sanitised provider
//! error code only (invariant C3; URLs can hold dictionary terms).

use std::time::{Duration, Instant};

use flow_router::{BackendError, Failure};

pub use reqwest::blocking::{Client, RequestBuilder, Response};

/// Cap on any JSON body we read (catalogue pages are ~1 MB).
pub const MAX_BODY: usize = 8 * 1024 * 1024;

/// TCP + TLS connect budget (§3.3).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Retry a 429 only when the provider asks for at most this long.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(2);

/// Provider call failure (§6.2, D28 §2.7). `Display` is safe to show and
/// log: it never holds a key, URL, header, body or provider free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudError {
    /// DNS / TCP / TLS failure (the kind, never the URL).
    Connect(String),
    /// HTTP 401 / 403 (not a limit or moderation): key rejected.
    Auth,
    /// HTTP 402 or a 403 key/workspace limit: out of credits or budget.
    Quota,
    /// HTTP 429, or 402 `in_flight_budget_exhausted` (transient).
    RateLimited {
        retry_after_ms: Option<u64>,
    },
    /// HTTP 413.
    TooLarge,
    /// HTTP 503 "no provider meets your routing requirements": with
    /// `zdr: true` this model has no zero-retention endpoint.
    NoPrivateRoute,
    /// HTTP 403 from a moderation / guardrail check (not sticky).
    Moderation,
    /// A 400 that names the audio format (path A retries once with WAV).
    UnsupportedFormat,
    /// Path B: the chat model's answer did not look like a verbatim
    /// transcript (§2.4 sanity check).
    Unverbatim,
    /// Other 4xx: status + sanitised provider error code.
    BadRequest {
        status: u16,
        code: Option<String>,
    },
    /// 5xx, or an SSE stream that ended in `finish_reason: "error"`.
    Server(u16),
    Timeout,
    /// 2xx with a body we cannot read.
    Malformed,
}

impl CloudError {
    pub fn failure(&self) -> Failure {
        match self {
            CloudError::Connect(_) => Failure::ConnectFailed,
            CloudError::Auth => Failure::AuthFailed,
            CloudError::Quota => Failure::QuotaExceeded,
            CloudError::RateLimited { .. } => Failure::Busy,
            CloudError::Timeout => Failure::Timeout,
            // TODO(or-route): Failure::NoPrivateRoute once flow-router has it.
            CloudError::NoPrivateRoute => Failure::ProviderError,
            CloudError::TooLarge
            | CloudError::Moderation
            | CloudError::UnsupportedFormat
            | CloudError::Unverbatim
            | CloudError::BadRequest { .. }
            | CloudError::Server(_)
            | CloudError::Malformed => Failure::ProviderError,
        }
    }

    /// The `Failure` code string, with the D28 code for a missing private
    /// route (`no_private_route`) even before the router enum has it.
    pub fn code(&self) -> &'static str {
        match self {
            CloudError::NoPrivateRoute => "no_private_route",
            e => e.failure().as_str(),
        }
    }

    /// Sub-code for `provider_error`s (`moderation`, `chat_audio_unverbatim`).
    pub fn detail(&self) -> Option<&'static str> {
        match self {
            CloudError::Moderation => Some("moderation"),
            CloudError::Unverbatim => Some("chat_audio_unverbatim"),
            CloudError::UnsupportedFormat => Some("unsupported_format"),
            _ => None,
        }
    }

    /// Failed before any output and is likely to fail again right away:
    /// another backend may be tried (cleanup's `CleanError::Unavailable`).
    pub fn is_fast_unavailable(&self) -> bool {
        matches!(
            self,
            CloudError::Connect(_)
                | CloudError::Auth
                | CloudError::Quota
                | CloudError::NoPrivateRoute
                | CloudError::RateLimited { .. }
        )
    }

    pub fn to_backend(&self) -> BackendError {
        BackendError::new(self.failure(), self.to_string())
    }
}

impl std::fmt::Display for CloudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloudError::Connect(k) => write!(f, "cannot reach OpenRouter ({k})"),
            CloudError::Auth => f.write_str("OpenRouter rejected the API key"),
            CloudError::Quota => {
                f.write_str("OpenRouter account or key is out of credits or over its limit")
            }
            CloudError::RateLimited { .. } => f.write_str("OpenRouter rate limit or busy"),
            CloudError::TooLarge => f.write_str("audio too large for OpenRouter (HTTP 413)"),
            CloudError::NoPrivateRoute => f.write_str(
                "model has no zero-retention provider (pick a model marked ZDR, \
                 or allow non-ZDR providers)",
            ),
            CloudError::Moderation => f.write_str("blocked by a moderation or guardrail check"),
            CloudError::UnsupportedFormat => {
                f.write_str("the provider does not accept this audio format")
            }
            CloudError::Unverbatim => {
                f.write_str("chat model did not return a verbatim transcript")
            }
            CloudError::BadRequest { status, code } => match code {
                Some(c) => write!(f, "OpenRouter rejected the request (HTTP {status}, {c})"),
                None => write!(f, "OpenRouter rejected the request (HTTP {status})"),
            },
            CloudError::Server(s) => write!(f, "provider error (HTTP {s})"),
            CloudError::Timeout => f.write_str("provider did not answer in time"),
            CloudError::Malformed => f.write_str("provider answer could not be read"),
        }
    }
}

impl std::error::Error for CloudError {}

/// One client per backend, kept across sessions (pooled TLS connection).
/// Never follows a redirect (review S3): reqwest would re-post the key and
/// audio to a host the user never consented to; a 3xx is a
/// `provider_error` instead. No `Referer` either.
pub fn client() -> Client {
    client_with(true, CONNECT_TIMEOUT)
}

/// [`client`] with a connect budget, refusing any non-https request when
/// `https_only` (always, except against the test mock server).
pub fn client_with(https_only: bool, connect: Duration) -> Client {
    Client::builder()
        .connect_timeout(connect)
        .pool_idle_timeout(Duration::from_secs(90))
        .user_agent(concat!("flow-clone/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .https_only(https_only)
        .build()
        // Never fall back to a default client: it would follow redirects.
        .expect("reqwest client without redirects")
}

/// Time left until `deadline`, or `Timeout`.
pub fn remaining(deadline: Instant) -> Result<Duration, CloudError> {
    let now = Instant::now();
    if deadline <= now + Duration::from_millis(50) {
        return Err(CloudError::Timeout);
    }
    Ok(deadline - now)
}

fn transport(e: reqwest::Error) -> CloudError {
    if e.is_timeout() {
        CloudError::Timeout
    } else if e.is_connect() {
        CloudError::Connect("connect".into())
    } else if e.is_request() || e.is_body() {
        CloudError::Connect("transport".into())
    } else {
        CloudError::Connect("http".into())
    }
}

/// Send with a timeout bounded by `deadline`; non-2xx → `CloudError`.
pub fn send(rb: RequestBuilder, deadline: Instant) -> Result<Response, CloudError> {
    let t = remaining(deadline)?;
    let resp = rb.timeout(t).send().map_err(transport)?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(resp);
    }
    let retry_after_ms = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|s| (s * 1000.0) as u64);
    let body = resp.bytes().map(|b| b.to_vec()).unwrap_or_default();
    Err(map_status(status, retry_after_ms, &body))
}

/// OpenRouter's error table (D28 §1.5, §2.7) → [`CloudError`]. The body is
/// inspected here and dropped: only a sanitised code survives.
pub fn map_status(status: u16, retry_after_ms: Option<u64>, body: &[u8]) -> CloudError {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let code = error_code(body);
    let lc = |p: &str| {
        v.pointer(p)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    let message = lc("/error/message");
    let code_lc = code.as_deref().unwrap_or("").to_ascii_lowercase();
    let has_meta = |k: &str| v.pointer(&format!("/error/metadata/{k}")).is_some();
    let quota_word = |s: &str| {
        [
            "quota",
            "credit",
            "insufficient",
            "payment",
            "billing",
            "budget",
        ]
        .iter()
        .any(|w| s.contains(w))
    };
    match status {
        401 => CloudError::Auth,
        402 if code_lc.contains("in_flight") || message.contains("in_flight") => {
            CloudError::RateLimited { retry_after_ms }
        }
        402 => CloudError::Quota,
        403 if has_meta("limit_source") || quota_word(&code_lc) || message.contains("limit") => {
            CloudError::Quota
        }
        403 if has_meta("reasons")
            || has_meta("flagged_input")
            || message.contains("moderat")
            || message.contains("guardrail")
            || message.contains("flagged") =>
        {
            CloudError::Moderation
        }
        403 => CloudError::Auth,
        408 => CloudError::Timeout,
        413 => CloudError::TooLarge,
        429 if quota_word(&code_lc) => CloudError::Quota,
        429 => CloudError::RateLimited { retry_after_ms },
        400 | 415 | 422
            if ["format", "codec", "unsupported audio", "decode"]
                .iter()
                .any(|w| message.contains(w) || code_lc.contains(w)) =>
        {
            CloudError::UnsupportedFormat
        }
        // Redirects are never followed (S3).
        300..=399 => CloudError::BadRequest {
            status,
            code: Some("redirect".into()),
        },
        503 if [
            "routing requirement",
            "no endpoints",
            "no allowed providers",
            "data policy",
            "zero data retention",
            "zdr",
        ]
        .iter()
        .any(|w| message.contains(w)) =>
        {
            CloudError::NoPrivateRoute
        }
        500..=599 => CloudError::Server(status),
        _ if quota_word(&code_lc) => CloudError::Quota,
        _ => CloudError::BadRequest { status, code },
    }
}

/// Read a 2xx JSON body (at most [`MAX_BODY`] bytes).
pub fn json(resp: Response) -> Result<serde_json::Value, CloudError> {
    json_capped(resp, MAX_BODY)
}

/// Read a 2xx JSON body of at most `cap` bytes; larger is `Malformed`.
pub fn json_capped(resp: Response, cap: usize) -> Result<serde_json::Value, CloudError> {
    use std::io::Read;
    if resp.content_length().is_some_and(|n| n as usize > cap) {
        return Err(CloudError::Malformed);
    }
    let mut b = Vec::new();
    resp.take(cap as u64 + 1).read_to_end(&mut b).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut
            || e.to_string().to_ascii_lowercase().contains("timed out")
        {
            CloudError::Timeout
        } else {
            CloudError::Malformed
        }
    })?;
    if b.len() > cap {
        return Err(CloudError::Malformed);
    }
    serde_json::from_slice(&b).map_err(|_| CloudError::Malformed)
}

/// Run `f` with the §3.2 retry rules: one retry for a 429 whose
/// `Retry-After` ≤ 2 s, or for a 502/503/504, if the deadline allows.
pub fn with_retry<T>(
    deadline: Instant,
    f: impl FnMut() -> Result<T, CloudError>,
) -> Result<T, CloudError> {
    retry(deadline, true, f)
}

/// [`with_retry`] for a request that is not safe to repeat blindly (it
/// stores audio or starts a billed job): a 504 may mean the first attempt
/// went through, so only a 429 and a 502/503 are retried (review S19).
pub fn with_retry_non_idempotent<T>(
    deadline: Instant,
    f: impl FnMut() -> Result<T, CloudError>,
) -> Result<T, CloudError> {
    retry(deadline, false, f)
}

fn retry<T>(
    deadline: Instant,
    retry_504: bool,
    mut f: impl FnMut() -> Result<T, CloudError>,
) -> Result<T, CloudError> {
    match f() {
        Err(CloudError::RateLimited {
            retry_after_ms: Some(ms),
        }) if Duration::from_millis(ms) <= MAX_RETRY_AFTER
            && Instant::now() + Duration::from_millis(ms + 500) < deadline =>
        {
            std::thread::sleep(Duration::from_millis(ms));
            f()
        }
        Err(CloudError::Server(s @ 502..=504))
            if (s != 504 || retry_504)
                && Instant::now() + Duration::from_millis(1000) < deadline =>
        {
            std::thread::sleep(Duration::from_millis(200));
            f()
        }
        r => r,
    }
}

/// A short machine-readable error code from a provider error body, never
/// free text: `[A-Za-z0-9_.-]`, ≤ 64 chars.
pub fn error_code(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let candidates = [
        v.pointer("/error/code"),
        v.pointer("/error/type"),
        v.pointer("/detail/status"),
        v.pointer("/detail/code"),
        v.pointer("/err_code"),
        v.pointer("/code"),
        v.pointer("/error"),
    ];
    let code = candidates.into_iter().flatten().find_map(|c| {
        let s = match c {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => return None,
        };
        let ok = !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
        ok.then_some(s)
    });
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_is_sanitised() {
        assert_eq!(
            error_code(br#"{"error":{"code":"insufficient_quota","message":"You exceeded"}}"#),
            Some("insufficient_quota".into())
        );
        assert_eq!(
            error_code(br#"{"detail":{"status":"invalid_api_key","message":"x"}}"#),
            Some("invalid_api_key".into())
        );
        assert_eq!(
            error_code(br#"{"err_code":"INVALID_AUTH","err_msg":"Invalid credentials."}"#),
            Some("INVALID_AUTH".into())
        );
        // Free text is never taken as a code.
        assert_eq!(error_code(br#"{"error":"Your key sk-abc is bad"}"#), None);
        assert_eq!(error_code(b"<html>"), None);
    }

    #[test]
    fn failures_map_to_router_kinds() {
        assert_eq!(CloudError::Auth.failure(), Failure::AuthFailed);
        assert_eq!(CloudError::Quota.failure(), Failure::QuotaExceeded);
        assert_eq!(
            CloudError::RateLimited {
                retry_after_ms: None
            }
            .failure(),
            Failure::Busy
        );
        assert_eq!(CloudError::Server(500).failure(), Failure::ProviderError);
        assert_eq!(CloudError::Malformed.failure(), Failure::ProviderError);
        assert_eq!(CloudError::Timeout.failure(), Failure::Timeout);
        assert_eq!(
            CloudError::Connect("connect".into()).failure(),
            Failure::ConnectFailed
        );
    }
}
