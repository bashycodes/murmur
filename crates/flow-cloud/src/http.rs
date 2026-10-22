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

/// TCP + TLS connect budget (§3.3).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Retry a 429 only when the provider asks for at most this long.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(2);

/// Provider call failure (§6.2). `Display` is safe to show and log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudError {
    /// DNS / TCP / TLS failure (the kind, never the URL).
    Connect(String),
    /// HTTP 401/403: key rejected.
    Auth,
    /// HTTP 402 or a quota/credit error code: out of credits.
    Quota,
    /// HTTP 429.
    RateLimited {
        retry_after_ms: Option<u64>,
    },
    /// HTTP 413.
    TooLarge,
    /// Other 4xx: status + sanitised provider error code.
    BadRequest {
        status: u16,
        code: Option<String>,
    },
    /// 5xx.
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
            CloudError::TooLarge
            | CloudError::BadRequest { .. }
            | CloudError::Server(_)
            | CloudError::Malformed => Failure::ProviderError,
        }
    }

    pub fn to_backend(&self) -> BackendError {
        BackendError::new(self.failure(), self.to_string())
    }
}

impl std::fmt::Display for CloudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloudError::Connect(k) => write!(f, "cannot reach the provider ({k})"),
            CloudError::Auth => f.write_str("provider rejected the API key"),
            CloudError::Quota => f.write_str("provider account is out of credits or quota"),
            CloudError::RateLimited { .. } => f.write_str("provider rate limit (HTTP 429)"),
            CloudError::TooLarge => f.write_str("audio too large for the provider (HTTP 413)"),
            CloudError::BadRequest { status, code } => match code {
                Some(c) => write!(f, "provider rejected the request (HTTP {status}, {c})"),
                None => write!(f, "provider rejected the request (HTTP {status})"),
            },
            CloudError::Server(s) => write!(f, "provider error (HTTP {s})"),
            CloudError::Timeout => f.write_str("provider did not answer in time"),
            CloudError::Malformed => f.write_str("provider answer could not be read"),
        }
    }
}

impl std::error::Error for CloudError {}

/// One client per backend, kept across sessions (pooled TLS connection).
/// Never follows a redirect (review S3): reqwest would forward custom key
/// headers (`xi-api-key`, `token` auth) and re-post the audio to a host
/// the user never consented to; a 3xx is a `provider_error` instead. No
/// `Referer` either (query strings can hold dictionary terms).
pub fn client() -> Client {
    client_for(false)
}

/// [`client`], refusing any non-https request when `https_only` (a base
/// URL that is https never downgrades).
pub fn client_for(https_only: bool) -> Client {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(90))
        .user_agent(concat!("flow-clone/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .https_only(https_only)
        .build()
        // Never fall back to a default client: it would follow redirects.
        .expect("reqwest client without redirects")
}

/// A plain-http client that connects only to `addrs` for `host` (the
/// addresses already checked as private): no second DNS lookup.
pub fn pinned_client(host: &str, addrs: &[std::net::SocketAddr]) -> Client {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(concat!("flow-clone/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .resolve_to_addrs(host, addrs)
        .build()
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
    let code = error_code(&body);
    let quota_code = code.as_deref().is_some_and(|c| {
        let c = c.to_ascii_lowercase();
        ["quota", "credit", "insufficient", "payment", "billing"]
            .iter()
            .any(|w| c.contains(w))
    });
    Err(match status {
        401 | 403 if !quota_code => CloudError::Auth,
        402 => CloudError::Quota,
        _ if quota_code && status < 500 => CloudError::Quota,
        401 | 403 => CloudError::Auth,
        413 => CloudError::TooLarge,
        429 => CloudError::RateLimited { retry_after_ms },
        // Redirects are never followed (S3): the provider moved, or
        // something tries to send the key/audio elsewhere.
        300..=399 => CloudError::BadRequest {
            status,
            code: Some("redirect".into()),
        },
        500..=599 => CloudError::Server(status),
        _ => CloudError::BadRequest { status, code },
    })
}

/// Read a 2xx JSON body.
pub fn json(resp: Response) -> Result<serde_json::Value, CloudError> {
    let b = resp.bytes().map_err(|e| {
        if e.is_timeout() {
            CloudError::Timeout
        } else {
            CloudError::Malformed
        }
    })?;
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

/// `base_url` without a trailing slash, plus `path`.
pub fn url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim().trim_end_matches('/'), path)
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
