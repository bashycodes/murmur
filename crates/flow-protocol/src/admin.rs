//! Admin API v1 (D39): manage the server's named keys (D38) over plain
//! HTTP JSON on the same port and TLS as the stream.
//!
//! Every request carries the same `Authorization: Bearer <key>` as the
//! stream. Any active key may call [`PATH_WHOAMI`]; the `/v1/admin/...`
//! routes need a key whose `admin` flag is set (else 403 [`code::FORBIDDEN`]).
//! Errors are an [`AdminError`] body with a stable `code`.
//!
//! | Method + path | Request | 2xx response |
//! |---|---|---|
//! | `GET` [`PATH_WHOAMI`] | - | 200 [`WhoAmI`] |
//! | `GET` [`PATH_ADMIN_KEYS`] | - | 200 [`KeyList`] |
//! | `POST` [`PATH_ADMIN_KEYS`] | [`CreateKey`] | 201 [`CreatedKey`] (the only time the secret is sent) |
//! | `DELETE` [`PATH_ADMIN_KEYS`]`/{id}` | - | 200 [`RevokedKey`] |
//!
//! Unknown fields are ignored everywhere, so fields can be added without a
//! new version; a breaking change gets new `/v2/...` paths.

use serde::{Deserialize, Serialize};

/// Admin API version (the `/v1` in the paths).
pub const ADMIN_API_VERSION: u32 = 1;
/// Who the bearer key is ([`WhoAmI`]); any active key.
pub const PATH_WHOAMI: &str = "/v1/whoami";
/// List (`GET`) and create (`POST`) keys; revoke is `DELETE {this}/{id}`.
pub const PATH_ADMIN_KEYS: &str = "/v1/admin/keys";
/// Largest request body the admin routes accept.
pub const MAX_ADMIN_BODY_BYTES: usize = 4 * 1024;

/// `DELETE` path for one key.
pub fn key_path(id: &str) -> String {
    format!("{PATH_ADMIN_KEYS}/{id}")
}

/// Stable [`AdminError::code`] values (HTTP status in brackets).
pub mod code {
    /// (401) missing, unknown or revoked key.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// (403) the key is not an admin key.
    pub const FORBIDDEN: &str = "forbidden";
    /// (404) no active key with that id.
    pub const NOT_FOUND: &str = "not_found";
    /// (404) the server runs with `auth = "none"`: there are no keys.
    pub const AUTH_OFF: &str = "auth_off";
    /// (409) revoking it would leave no active admin key.
    pub const LAST_ADMIN: &str = "last_admin";
    /// (409) an active key already has that name.
    pub const NAME_TAKEN: &str = "name_taken";
    /// (400) bad JSON or a bad name.
    pub const BAD_REQUEST: &str = "bad_request";
    /// (500) the server could not write its key file.
    pub const SERVER_ERROR: &str = "server_error";
    /// Client side only: the server could not be reached.
    pub const CONNECT_FAILED: &str = "connect_failed";
    /// Client side only: the answer was not the expected JSON.
    pub const BAD_RESPONSE: &str = "bad_response";
    /// Client side only: the server predates the admin API (404 without
    /// an [`super::AdminError`] body).
    pub const UNSUPPORTED: &str = "unsupported";
}

/// Error body of every non-2xx admin answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdminError {
    pub code: String,
    pub message: String,
}

impl AdminError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AdminError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            f.write_str(&self.code)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for AdminError {}

/// `GET /v1/whoami`: the key this request authenticated with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WhoAmI {
    pub id: String,
    pub name: String,
    pub admin: bool,
}

/// One key as the admin API shows it. Never the secret or its hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyView {
    pub id: String,
    pub name: String,
    pub admin: bool,
    /// Unix seconds.
    pub created: u64,
    /// Unix seconds; `None` = active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked: Option<u64>,
    /// Unix seconds of the last real use: a finished dictation or a model
    /// download. Older servers omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
    /// Unix seconds of the last authenticated request of any kind
    /// (connection checks and admin page visits too). Older servers omit
    /// it and send only `last_seen`; read it through [`KeyView::connected`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_connected: Option<u64>,
    /// Same as `last_connected`, still sent for clients that predate the
    /// split (they show it as "last seen").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
    /// `hello.client` of its last connection (e.g. `flow-android 1.0.0`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    pub dictations: u64,
    pub audio_ms: u64,
    /// Model library downloads (D40): requests that sent bytes, and the
    /// bytes sent. Older servers omit them (0).
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub download_bytes: u64,
    /// The key this request authenticated with.
    pub current: bool,
}

impl KeyView {
    pub fn active(&self) -> bool {
        self.revoked.is_none()
    }

    /// Last connected, falling back to an older server's `last_seen`.
    pub fn connected(&self) -> Option<u64> {
        self.last_connected.or(self.last_seen)
    }
}

/// `GET /v1/admin/keys`: every key, revoked ones too, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyList {
    pub keys: Vec<KeyView>,
}

/// `POST /v1/admin/keys` body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateKey {
    /// 1-64 characters of `A-Z a-z 0-9 . _ -`, unique among active keys.
    pub name: String,
    pub admin: bool,
}

/// `POST /v1/admin/keys` answer (201). `secret` is shown once and never
/// stored by the server (only its hash) or by the client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CreatedKey {
    pub key: KeyView,
    pub secret: String,
}

/// `DELETE /v1/admin/keys/{id}` answer: the key, now revoked. Open
/// connections with it finish; the next one gets 401.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RevokedKey {
    pub key: KeyView,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shapes_are_stable() {
        let v = KeyView {
            id: "k-0123456789abcdef".into(),
            name: "phone".into(),
            created: 1,
            ..Default::default()
        };
        let j = serde_json::to_value(&v).unwrap();
        assert_eq!(
            j,
            serde_json::json!({"id":"k-0123456789abcdef","name":"phone","admin":false,
                "created":1,"dictations":0,"audio_ms":0,"downloads":0,"download_bytes":0,"current":false})
        );
        // Unknown fields are ignored; missing ones default.
        let c: CreateKey = serde_json::from_str(r#"{"name":"x","future":1}"#).unwrap();
        assert_eq!(
            c,
            CreateKey {
                name: "x".into(),
                admin: false
            }
        );
        let e: AdminError = serde_json::from_str(r#"{"code":"forbidden"}"#).unwrap();
        assert_eq!(e.to_string(), "forbidden");
        assert_eq!(key_path("k-1"), "/v1/admin/keys/k-1");
    }

    #[test]
    fn connected_falls_back_to_an_older_servers_last_seen() {
        let old: KeyView = serde_json::from_str(r#"{"id":"k-1","last_seen":5}"#).unwrap();
        assert_eq!((old.connected(), old.last_used), (Some(5), None));
        let new: KeyView =
            serde_json::from_str(r#"{"id":"k-1","last_used":3,"last_connected":7,"last_seen":7}"#)
                .unwrap();
        assert_eq!((new.connected(), new.last_used), (Some(7), Some(3)));
    }
}
