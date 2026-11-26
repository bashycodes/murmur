//! Admin API client (D39, `murmur_protocol::admin`): who am I, and list /
//! create / revoke the server's keys. Plain HTTP/1.1 JSON on the stream's
//! host, port, TLS and bearer key, with the same address checks as the
//! stream (a host must be set; TLS pin honoured when TLS is on).
//!
//! Blocking: every call runs on its own short-lived single-thread runtime,
//! bounded by `connect_timeout_ms` for the connection and [`REQUEST_TIMEOUT`]
//! overall. Shared by murmur-windows and, through murmur-ffi, Android. Nothing
//! here logs; a created key's secret is only in the returned value.

use std::time::Duration;

use murmur_protocol::admin::{
    code, key_path, AdminError, CreateKey, CreatedKey, KeyList, RevokedKey, WhoAmI,
    PATH_ADMIN_KEYS, PATH_WHOAMI,
};
use murmur_protocol::dictionary::{
    RemoveTeamEntry, TeamDictionary, TeamEntry, PATH_ADMIN_DICTIONARY, PATH_DICTIONARY,
};
use murmur_router::ServerConfig;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Upper bound on one admin request, connection included.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest answer read (a key list is a few hundred bytes per key).
const MAX_RESPONSE_BYTES: usize = 1 << 20;

/// An admin client for one server config. Cheap; holds no connection.
#[derive(Clone)]
pub struct AdminClient {
    cfg: ServerConfig,
    client: String,
}

impl AdminClient {
    /// `client` names the app in `User-Agent` (e.g. `"murmur-windows 1.0.0"`).
    pub fn new(cfg: ServerConfig, client: &str) -> Self {
        Self {
            cfg,
            client: client.into(),
        }
    }

    /// The server this client talks to (`host:port`), which names a cached
    /// team dictionary ([`crate::team_dict::TeamDictCache`]).
    pub fn server(&self) -> String {
        self.cfg.addr()
    }

    /// The key this config's token is (any active key).
    pub fn whoami(&self) -> Result<WhoAmI, AdminError> {
        self.call("GET", PATH_WHOAMI, None, 200)
    }

    /// Every key with its usage (admin key only).
    pub fn list_keys(&self) -> Result<KeyList, AdminError> {
        self.call("GET", PATH_ADMIN_KEYS, None, 200)
    }

    /// Create a key; the answer holds its secret, the only copy anywhere.
    pub fn create_key(&self, name: &str, admin: bool) -> Result<CreatedKey, AdminError> {
        let body = serde_json::to_vec(&CreateKey {
            name: name.into(),
            admin,
        })
        .expect("CreateKey serialises");
        self.call("POST", PATH_ADMIN_KEYS, Some(body), 201)
    }

    /// Revoke key `id`. The server refuses the last admin key (`last_admin`).
    pub fn revoke_key(&self, id: &str) -> Result<RevokedKey, AdminError> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(AdminError::new(code::BAD_REQUEST, "bad key id"));
        }
        self.call("DELETE", &key_path(id), None, 200)
    }

    /// The server's team dictionary (any active key). With the `etag` of
    /// the copy the caller holds, `Ok(None)` means that copy is current
    /// (304). Otherwise the list and its new ETag.
    pub fn team_dictionary(
        &self,
        etag: Option<&str>,
    ) -> Result<Option<(TeamDictionary, Option<String>)>, AdminError> {
        let extra: Vec<(&str, &str)> = etag.map(|t| ("If-None-Match", t)).into_iter().collect();
        let a = self.round("GET", PATH_DICTIONARY, None, &extra)?;
        if a.status == 304 {
            return Ok(None);
        }
        let list = decode(&a, 200)?;
        Ok(Some((list, a.etag)))
    }

    /// Add `e` to the team dictionary, or replace the entry with the same
    /// word (admin key only). The whole list and its ETag.
    pub fn add_team_entry(
        &self,
        e: &TeamEntry,
    ) -> Result<(TeamDictionary, Option<String>), AdminError> {
        let body = serde_json::to_vec(e).expect("TeamEntry serialises");
        let a = self.round("POST", PATH_ADMIN_DICTIONARY, Some(body), &[])?;
        Ok((decode(&a, 200)?, a.etag))
    }

    /// Remove `word` from the team dictionary (admin key only).
    pub fn remove_team_entry(
        &self,
        word: &str,
    ) -> Result<(TeamDictionary, Option<String>), AdminError> {
        let body = serde_json::to_vec(&RemoveTeamEntry { word: word.into() })
            .expect("RemoveTeamEntry serialises");
        let a = self.round("DELETE", PATH_ADMIN_DICTIONARY, Some(body), &[])?;
        Ok((decode(&a, 200)?, a.etag))
    }

    fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        want: u16,
    ) -> Result<T, AdminError> {
        decode(&self.round(method, path, body, &[])?, want)
    }

    /// One request and its raw answer.
    fn round(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        extra: &[(&str, &str)],
    ) -> Result<Answer, AdminError> {
        crate::check_address(&self.cfg).map_err(connect_err)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| AdminError::new(code::CONNECT_FAILED, e.to_string()))?;
        rt.block_on(async {
            tokio::time::timeout(REQUEST_TIMEOUT, self.exchange(method, path, body, extra))
                .await
                .map_err(|_| {
                    AdminError::new(
                        code::CONNECT_FAILED,
                        format!("{}: timed out", self.cfg.addr()),
                    )
                })?
        })
    }

    async fn exchange(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        extra: &[(&str, &str)],
    ) -> Result<Answer, AdminError> {
        let connect = Duration::from_millis(self.cfg.connect_timeout_ms.max(1));
        let io = tokio::time::timeout(connect, crate::conn::dial(&self.cfg))
            .await
            .map_err(|_| {
                AdminError::new(
                    code::CONNECT_FAILED,
                    format!("{}: connect timed out", self.cfg.addr()),
                )
            })?
            .map_err(connect_err)?;
        let req = request(
            &self.cfg,
            &self.client,
            method,
            path,
            body.as_deref(),
            extra,
        );
        round_trip(io, &req).await
    }
}

fn connect_err(e: murmur_router::BackendError) -> AdminError {
    AdminError::new(code::CONNECT_FAILED, e.to_string())
}

fn request(
    cfg: &ServerConfig,
    client: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    extra: &[(&str, &str)],
) -> Vec<u8> {
    let mut h = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {client}\r\nAccept: application/json\r\nConnection: close\r\n",
        cfg.addr()
    );
    if !cfg.token.is_empty() {
        h.push_str(&format!("Authorization: Bearer {}\r\n", cfg.token));
    }
    for (k, v) in extra {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    let body = body.unwrap_or_default();
    if !body.is_empty() || method == "POST" {
        h.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    h.push_str("\r\n");
    let mut out = h.into_bytes();
    out.extend_from_slice(body);
    out
}

/// One HTTP answer.
#[derive(Debug, PartialEq, Eq)]
struct Answer {
    status: u16,
    /// `Retry-After` in seconds (a 429 lockout, D112).
    retry_after_s: Option<u64>,
    /// `ETag` (the team dictionary).
    etag: Option<String>,
    body: Vec<u8>,
}

/// Send `req`, read one response.
async fn round_trip<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    req: &[u8],
) -> Result<Answer, AdminError> {
    let io = |e: std::io::Error| AdminError::new(code::CONNECT_FAILED, e.to_string());
    s.write_all(req).await.map_err(io)?;
    s.flush().await.map_err(io)?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        // Stop as soon as a Content-Length body is complete: a TLS peer
        // may close without close_notify, which reads as an error.
        if let Some(r) = parse(&buf, false) {
            return r;
        }
        let n = match s.read(&mut chunk).await {
            Ok(n) => n,
            Err(e) if !buf.is_empty() => {
                return parse(&buf, true).unwrap_or_else(|| Err(io(e)));
            }
            Err(e) => return Err(io(e)),
        };
        if n == 0 {
            return parse(&buf, true).unwrap_or_else(|| Err(bad("connection closed early")));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_RESPONSE_BYTES {
            return Err(bad("answer too large"));
        }
    }
}

fn bad(m: &str) -> AdminError {
    AdminError::new(code::BAD_RESPONSE, m)
}

/// Parse an HTTP/1.1 response from `buf`: `None` = need more bytes
/// (`eof` = there are no more, so a body without a length ends here).
fn parse(buf: &[u8], eof: bool) -> Option<Result<Answer, AdminError>> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = match std::str::from_utf8(&buf[..end]) {
        Ok(h) => h,
        Err(_) => return Some(Err(bad("bad response head"))),
    };
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse::<u16>().ok());
    let Some(status) = status else {
        return Some(Err(bad("bad status line")));
    };
    let mut len = None;
    let mut chunked = false;
    let mut retry_after_s = None;
    let mut etag = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if k.eq_ignore_ascii_case("content-length") {
            len = v.parse::<usize>().ok();
        } else if k.eq_ignore_ascii_case("transfer-encoding") {
            chunked = v.to_ascii_lowercase().contains("chunked");
        } else if k.eq_ignore_ascii_case("retry-after") {
            retry_after_s = crate::conn::retry_after_s(Some(v));
        } else if k.eq_ignore_ascii_case("etag") {
            etag = Some(v.to_string());
        }
    }
    let body = &buf[end + 4..];
    // A 304 (or 204) has no body, whatever the headers say.
    let len = if status == 304 || status == 204 {
        Some(0)
    } else {
        len
    };
    let answer = |body: Vec<u8>| {
        Ok(Answer {
            status,
            retry_after_s,
            etag: etag.clone(),
            body,
        })
    };
    if chunked {
        return dechunk(body)
            .map(answer)
            .or_else(|| eof.then(|| Err(bad("truncated chunked answer"))));
    }
    match len {
        Some(n) if body.len() >= n => Some(answer(body[..n].to_vec())),
        Some(_) if eof => Some(Err(bad("truncated answer"))),
        Some(_) => None,
        None if eof => Some(answer(body.to_vec())),
        None => None,
    }
}

/// A complete chunked body, or `None` while the last chunk is missing.
fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let size = std::str::from_utf8(&b[..eol]).ok()?;
        let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        if b.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}

fn decode<T: DeserializeOwned>(a: &Answer, want: u16) -> Result<T, AdminError> {
    let (status, body) = (a.status, &a.body[..]);
    if status == want {
        return serde_json::from_slice(body).map_err(|e| bad(&format!("bad answer: {e}")));
    }
    // D112: locked out after wrong keys; the wait is in the header.
    if status == 429 {
        let message = serde_json::from_slice::<AdminError>(body)
            .ok()
            .filter(|e| !e.message.is_empty())
            .map_or_else(|| murmur_router::throttled_text(None), |e| e.message);
        return Err(AdminError {
            retry_after_s: a.retry_after_s,
            ..AdminError::new(code::THROTTLED, message)
        });
    }
    match serde_json::from_slice::<AdminError>(body) {
        Ok(e) if !e.code.is_empty() => Err(e),
        _ if status == 404 => Err(AdminError::new(
            code::UNSUPPORTED,
            "this server has no admin API: update murmur-server",
        )),
        _ if status == 401 => Err(AdminError::new(
            code::UNAUTHORIZED,
            "unknown or revoked key",
        )),
        _ => Err(bad(&format!("HTTP {status}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_length_chunked_and_eof_bodies() {
        let ok = |status, body: &[u8]| Answer {
            status,
            retry_after_s: None,
            etag: None,
            body: body.to_vec(),
        };
        let r = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}";
        assert_eq!(parse(r, false).unwrap().unwrap(), ok(200, b"{}"));
        assert!(parse(&r[..r.len() - 1], false).is_none());
        assert!(parse(&r[..r.len() - 1], true).unwrap().is_err());
        let c = b"HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse(c, false).unwrap().unwrap(), ok(201, b"abcde"));
        assert!(parse(&c[..c.len() - 7], false).is_none());
        let e = b"HTTP/1.1 404 Not Found\r\n\r\nnope";
        assert!(parse(e, false).is_none());
        assert_eq!(parse(e, true).unwrap().unwrap(), ok(404, b"nope"));
        let t = b"HTTP/1.1 429 Too Many Requests\r\nretry-after: 29\r\ncontent-length: 0\r\n\r\n";
        assert_eq!(parse(t, false).unwrap().unwrap().retry_after_s, Some(29));
        assert!(parse(b"garbage\r\n\r\n", true).unwrap().is_err());
        // The team dictionary's ETag; a 304 is complete without a body.
        let n = b"HTTP/1.1 304 Not Modified\r\netag: \"ab12\"\r\n\r\n";
        let a = parse(n, false).unwrap().unwrap();
        assert_eq!((a.status, a.etag.as_deref()), (304, Some("\"ab12\"")));
    }

    #[test]
    fn maps_errors() {
        let a = |status, retry_after_s, body: &[u8]| Answer {
            status,
            retry_after_s,
            etag: None,
            body: body.to_vec(),
        };
        let e = decode::<WhoAmI>(
            &a(403, None, br#"{"code":"forbidden","message":"no"}"#),
            200,
        )
        .unwrap_err();
        assert_eq!((e.code.as_str(), e.message.as_str()), ("forbidden", "no"));
        let e = decode::<WhoAmI>(&a(404, None, b""), 200).unwrap_err();
        assert_eq!(e.code, code::UNSUPPORTED);
        let e = decode::<WhoAmI>(&a(401, None, b"unauthorized"), 200).unwrap_err();
        assert_eq!(e.code, code::UNAUTHORIZED);
        // D112: a lockout carries the server's wait.
        let body = br#"{"code":"throttled","message":"too many failed attempts"}"#;
        let e = decode::<WhoAmI>(&a(429, Some(42), body), 200).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.retry_after_s),
            (code::THROTTLED, Some(42))
        );
        let e = decode::<WhoAmI>(&a(429, None, b""), 200).unwrap_err();
        assert_eq!((e.code.as_str(), e.retry_after_s), (code::THROTTLED, None));
        let w = decode::<WhoAmI>(
            &a(200, None, br#"{"id":"k-1","name":"a","admin":true}"#),
            200,
        )
        .unwrap();
        assert!(w.admin);
    }

    #[test]
    fn request_carries_the_key_and_a_json_body() {
        let cfg = ServerConfig {
            host: "127.0.0.1".into(),
            port: 7313,
            token: "secret-token-0123456789".into(),
            ..Default::default()
        };
        let r = String::from_utf8(request(
            &cfg,
            "t 1",
            "POST",
            "/v1/admin/keys",
            Some(b"{}"),
            &[],
        ))
        .unwrap();
        assert!(r.starts_with("POST /v1/admin/keys HTTP/1.1\r\n"), "{r}");
        assert!(r.contains("Authorization: Bearer secret-token-0123456789\r\n"));
        assert!(r.contains("Content-Length: 2\r\n") && r.ends_with("\r\n\r\n{}"));
        let r = String::from_utf8(request(&cfg, "t", "GET", "/v1/whoami", None, &[])).unwrap();
        assert!(!r.contains("Content-Length"));
        let extra = [("If-None-Match", "\"ab12\"")];
        let r =
            String::from_utf8(request(&cfg, "t", "GET", "/v1/dictionary", None, &extra)).unwrap();
        assert!(r.contains("If-None-Match: \"ab12\"\r\n"), "{r}");
    }

    #[test]
    fn whoami_without_a_host_fails_before_any_socket() {
        let c = AdminClient::new(
            ServerConfig {
                token: "secret-token-0123456789".into(),
                ..Default::default()
            },
            "t",
        );
        assert_eq!(c.whoami().unwrap_err().code, code::CONNECT_FAILED);
    }

    #[test]
    fn revoke_rejects_a_bad_id_before_any_socket() {
        // A public address with TLS off is allowed (D111); the id check
        // still refuses first.
        let c = AdminClient::new(
            ServerConfig {
                host: "203.0.113.5".into(),
                token: "secret-token-0123456789".into(),
                ..Default::default()
            },
            "t",
        );
        assert_eq!(c.revoke_key("../x").unwrap_err().code, code::BAD_REQUEST);
    }
}
