//! The one host this crate talks to (D28 §2.1, §3): `https://openrouter.ai`.
//!
//! The base URL is a compile-time constant. No config field, preset or
//! environment variable can change it, so a stored key can only ever be
//! sent to OpenRouter. The only seam is [`crate::testing::override_base_url`],
//! which exists only with the `mock-server` feature (tests; shells never
//! enable it, `tools/check-build-matrix.sh` checks the shipped flavours).

use std::time::Duration;

use crate::http::{self, Client};

/// OpenRouter API root. Never user-editable.
pub const BASE_URL: &str = "https://openrouter.ai/api/v1";
/// The host a key is bound to and consent is given for.
pub const HOST: &str = "openrouter.ai";

/// A client bound to the OpenRouter API root.
#[derive(Clone)]
pub struct Api {
    base: String,
    http: Client,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").field("base", &self.base).finish()
    }
}

impl Default for Api {
    fn default() -> Self {
        Self::new()
    }
}

impl Api {
    /// The production API (STT connect budget, 3 s).
    pub fn new() -> Self {
        Self::with_connect_timeout(http::CONNECT_TIMEOUT)
    }

    /// The production API with its own connect budget (cleanup: 1.5 s).
    pub fn with_connect_timeout(connect: Duration) -> Self {
        Self {
            // https only, except in `mock-server` builds (tests), whose
            // mock is plain http on loopback.
            http: http::client_with(!cfg!(feature = "mock-server"), connect),
            base: BASE_URL.to_string(),
        }
    }

    /// API root without a trailing slash.
    pub fn base(&self) -> String {
        base_url(&self.base)
    }

    /// `base` + `path` (`path` starts with `/`).
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base(), path)
    }

    pub fn client(&self) -> &Client {
        &self.http
    }

    /// Whether `url` (absolute, or a path) stays on the API's origin; used
    /// before following a pagination link.
    pub fn resolve_same_origin(&self, link: &str) -> Option<String> {
        let origin = origin_of(&self.base())?;
        if link.starts_with('/') && !link.starts_with("//") {
            return Some(format!("{origin}{link}"));
        }
        let o = origin_of(link)?;
        (o == origin).then(|| link.to_string())
    }
}

/// `scheme://host[:port]` of an absolute URL.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() || host.contains('@') {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase()
    ))
}

#[cfg(not(feature = "mock-server"))]
fn base_url(base: &str) -> String {
    base.to_string()
}

/// Tests: the override current at request time, so a mock (or a dead
/// port) can be swapped in under a backend that already exists.
#[cfg(feature = "mock-server")]
fn base_url(base: &str) -> String {
    crate::testing::current_override().unwrap_or_else(|| base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_base_is_openrouter() {
        assert_eq!(BASE_URL, "https://openrouter.ai/api/v1");
        assert!(BASE_URL.contains(HOST));
    }

    #[test]
    fn pagination_stays_on_origin() {
        let a = Api {
            base: BASE_URL.into(),
            http: http::client(),
        };
        assert_eq!(
            a.resolve_same_origin("/api/v1/models?offset=1000")
                .as_deref(),
            Some("https://openrouter.ai/api/v1/models?offset=1000")
        );
        assert!(a
            .resolve_same_origin("https://openrouter.ai/api/v1/models?offset=2")
            .is_some());
        for bad in [
            "https://evil.example/api/v1/models",
            "//evil.example/x",
            "http://openrouter.ai/api/v1/models",
            "https://openrouter.ai.evil.example/x",
            "https://user@openrouter.ai/x",
        ] {
            assert_eq!(a.resolve_same_origin(bad), None, "{bad}");
        }
    }
}
