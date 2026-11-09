//! Model library client (D40 server side, D41 client side): list the
//! models a flow-server serves and download their files, resumable and
//! verified. Shared by the Windows app and (through flow-ffi) Android.
//!
//! Interface: [`LibraryClient::new`] with the same [`ServerConfig`] as
//! dictation (host, port, TLS + pin or Mozilla roots, bearer key), then
//! * [`LibraryClient::list`]: `GET /v1/models`, entries with an unsafe id or
//!   file name dropped (they could not be saved safely anyway);
//! * [`LibraryClient::download`]: one file into `dest`, through `part`.
//!
//! A download is checked against the [`LibraryFile`] the caller passes: its
//! `size` and `sha256` are the expected ones. Callers pass the server's
//! listing for a custom model, or the same file with their built-in pin
//! for a catalogue model (and refuse first when the two differ).
//!
//! Resume: `part` keeps what arrived when a download is cut off (network
//! error, server gone, stall). The next call hashes that prefix and asks
//! for the rest (`Range: bytes=<len>-`, `If-Range: "<sha256>"`); a server
//! that answers with the whole file (`200`) starts it over. A cancel
//! deletes `part` (nothing is kept, as the apps promise); a fingerprint
//! mismatch deletes it too. `dest` appears only once size and hash match.
//!
//! Blocking: each call runs on the client's own small tokio runtime, so
//! call it from a worker thread, never from inside another runtime.
//! Progress and cancel are plain callbacks (JNI- and Tauri-friendly).

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

use flow_protocol::library::{
    file_path, is_safe_name, LibraryFile, LibraryModel, ModelList, PATH_MODELS,
};
use flow_router::{BackendError, Failure, ServerConfig};
use http_body_util::{BodyExt, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::header::{
    AUTHORIZATION, CONTENT_RANGE, ETAG, HOST, IF_RANGE, RANGE, RETRY_AFTER, USER_AGENT,
};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha2::{Digest, Sha256};

/// A body frame must arrive within this (a stalled download fails and
/// keeps its `.part` for the next try).
pub const STALL: Duration = Duration::from_secs(60);
/// The response head must arrive within this after the request.
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Largest `GET /v1/models` body accepted.
const MAX_LIST_BYTES: usize = 8 << 20;
/// How often `progress` is called while bytes arrive (at most).
const TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// No server host set.
    NotConfigured,
    /// DNS, TCP, TLS (incl. a pin mismatch), cleartext refused, timeouts,
    /// a connection cut mid-file (the `.part` is kept: retry resumes).
    Network,
    /// The key was refused (`401`/`403`).
    Auth,
    /// No such model or file on the server (`404`).
    NotFound,
    /// The server is rescanning a changed file (`503`); retry later.
    Unavailable,
    /// Any other HTTP answer, or a malformed one.
    Protocol,
    /// The bytes are not the expected ones (size or SHA-256, or the
    /// server's `ETag` names another hash). Nothing is kept.
    Mismatch,
    /// Reading or writing the local files failed.
    Io,
    /// `cancel` returned true. Nothing is kept.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryError {
    pub kind: ErrorKind,
    pub message: String,
}

impl LibraryError {
    fn new(kind: ErrorKind, m: impl Into<String>) -> Self {
        Self {
            kind,
            message: m.into(),
        }
    }
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LibraryError {}

impl From<BackendError> for LibraryError {
    fn from(e: BackendError) -> Self {
        let kind = match e.kind {
            Failure::ServerNotConfigured => ErrorKind::NotConfigured,
            Failure::AuthFailed => ErrorKind::Auth,
            _ => ErrorKind::Network,
        };
        Self::new(kind, e.message)
    }
}

/// A file name from the listing is a safe relative path: `/`-separated
/// parts that each pass [`is_safe_name`] (no `..`, no absolute paths, no
/// backslashes), at most 5 parts. Apps join it under their own folder.
pub fn is_safe_file_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    parts.len() <= 5 && parts.iter().all(|p| is_safe_name(p))
}

fn safe_model(m: &LibraryModel) -> bool {
    is_safe_name(&m.id)
        && !m.files.is_empty()
        && m.files.iter().all(|f| {
            is_safe_file_name(&f.name)
                && f.sha256.len() == 64
                && f.sha256
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

pub struct LibraryClient {
    cfg: ServerConfig,
    user_agent: String,
    rt: tokio::runtime::Runtime,
}

impl LibraryClient {
    /// `client` names the app in `User-Agent` (`"flow-windows 0.12.0"`).
    pub fn new(cfg: ServerConfig, client: &str) -> Result<Self, LibraryError> {
        crate::check_address(&cfg)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| LibraryError::new(ErrorKind::Io, format!("runtime: {e}")))?;
        Ok(Self {
            cfg,
            user_agent: client.to_string(),
            rt,
        })
    }

    /// The server's model library (`GET /v1/models`).
    pub fn list(&self) -> Result<ModelList, LibraryError> {
        self.rt.block_on(async {
            let resp = self.get(PATH_MODELS, &[]).await?;
            check_status(&resp)?;
            let mut body = resp.into_body();
            let mut buf = Vec::new();
            while let Some(frame) = tokio::time::timeout(STALL, body.frame())
                .await
                .map_err(|_| net("the server stopped sending the model list"))?
            {
                let frame = frame.map_err(|e| net(format!("model list: {e}")))?;
                if let Some(d) = frame.data_ref() {
                    buf.extend_from_slice(d);
                    if buf.len() > MAX_LIST_BYTES {
                        return Err(LibraryError::new(
                            ErrorKind::Protocol,
                            "model list is too large",
                        ));
                    }
                }
            }
            let mut list: ModelList = serde_json::from_slice(&buf).map_err(|e| {
                LibraryError::new(ErrorKind::Protocol, format!("bad model list: {e}"))
            })?;
            list.models.retain(safe_model);
            Ok(list)
        })
    }

    /// Download `file` of model `model_id` into `dest`, through `part`
    /// (see the module docs for resume and verification). `progress` gets
    /// the bytes of this file on disk so far; `cancel` is polled between
    /// chunks. A `dest` that already holds exactly the expected bytes is
    /// kept and nothing is requested.
    pub fn download(
        &self,
        model_id: &str,
        file: &LibraryFile,
        dest: &Path,
        part: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), LibraryError> {
        if !is_safe_name(model_id) || !is_safe_file_name(&file.name) {
            return Err(LibraryError::new(
                ErrorKind::Protocol,
                format!("unsafe model or file name: {model_id}/{}", file.name),
            ));
        }
        if std::fs::metadata(dest).is_ok_and(|m| m.is_file() && m.len() == file.size)
            && hex(&hash_prefix(dest, file.size, cancel)?.1.finalize()) == file.sha256
        {
            progress(file.size);
            return Ok(());
        }
        if let Some(dir) = part.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io_err("create the folder", e))?;
        }
        let r = self.fetch(model_id, file, part, progress, cancel);
        match &r {
            Ok(()) => {}
            // Kept for the next try (resume).
            Err(e) if e.kind == ErrorKind::Network => return r,
            Err(_) => {
                let _ = std::fs::remove_file(part);
                return r;
            }
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io_err("create the folder", e))?;
        }
        std::fs::rename(part, dest).map_err(|e| {
            let _ = std::fs::remove_file(part);
            io_err("save the file", e)
        })
    }

    /// Bring `part` to the full verified file (not renamed yet).
    fn fetch(
        &self,
        model_id: &str,
        file: &LibraryFile,
        part: &Path,
        progress: &mut dyn FnMut(u64),
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), LibraryError> {
        let have = std::fs::metadata(part).map_or(0, |m| m.len());
        let (mut have, mut hasher) = if have > 0 && have <= file.size {
            hash_prefix(part, have, cancel)?
        } else {
            (0, Sha256::new())
        };
        // A complete part from an earlier try: no request needed if it
        // verifies; otherwise start over.
        if have == file.size {
            if hex(&hasher.clone().finalize()) == file.sha256 {
                progress(have);
                return Ok(());
            }
            have = 0;
            hasher = Sha256::new();
        }
        progress(have);
        let path = file_path(model_id, &file.name);
        let etag = format!("\"{}\"", file.sha256);
        self.rt.block_on(async {
            let range;
            let mut headers: Vec<(hyper::header::HeaderName, &str)> = Vec::new();
            if have > 0 {
                range = format!("bytes={have}-");
                headers.push((RANGE, range.as_str()));
                headers.push((IF_RANGE, etag.as_str()));
            }
            let mut resp = self.get(&path, &headers).await?;
            if resp.status() == StatusCode::RANGE_NOT_SATISFIABLE && have > 0 {
                // Our part is not a prefix of the server's file: refetch.
                have = 0;
                hasher = Sha256::new();
                resp = self.get(&path, &[]).await?;
            }
            check_status(&resp)?;
            if let Some(t) = resp.headers().get(ETAG).and_then(|v| v.to_str().ok()) {
                if t != etag {
                    return Err(LibraryError::new(
                        ErrorKind::Mismatch,
                        format!(
                            "{}: the server's copy is not the expected file (its fingerprint differs)",
                            file.name
                        ),
                    ));
                }
            }
            let append = resp.status() == StatusCode::PARTIAL_CONTENT;
            if append {
                let start = resp
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(content_range_start);
                if start != Some(have) {
                    return Err(LibraryError::new(
                        ErrorKind::Protocol,
                        "the server answered another range than asked",
                    ));
                }
            } else {
                have = 0;
                hasher = Sha256::new();
            }
            let mut out = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(part)
                .map_err(|e| io_err("write the file", e))?;
            out.set_len(have).map_err(|e| io_err("write the file", e))?;
            out.seek(SeekFrom::Start(have))
                .map_err(|e| io_err("write the file", e))?;
            progress(have);
            let mut body = resp.into_body();
            let mut last = std::time::Instant::now();
            loop {
                if cancel() {
                    return Err(LibraryError::new(ErrorKind::Cancelled, "cancelled"));
                }
                let frame = match tokio::time::timeout(STALL, body.frame()).await {
                    Err(_) => {
                        let _ = out.sync_all();
                        return Err(net(format!(
                            "{}: no data for {} s",
                            file.name,
                            STALL.as_secs()
                        )));
                    }
                    Ok(None) => break,
                    Ok(Some(Err(e))) => {
                        let _ = out.sync_all();
                        return Err(net(format!("{} stopped: {e}", file.name)));
                    }
                    Ok(Some(Ok(f))) => f,
                };
                let Some(d) = frame.data_ref() else { continue };
                if have + d.len() as u64 > file.size {
                    return Err(LibraryError::new(
                        ErrorKind::Mismatch,
                        format!("{} is larger than expected", file.name),
                    ));
                }
                out.write_all(d).map_err(|e| io_err("write the file", e))?;
                hasher.update(d);
                have += d.len() as u64;
                if last.elapsed() >= TICK {
                    last = std::time::Instant::now();
                    progress(have);
                }
            }
            out.sync_all().map_err(|e| io_err("write the file", e))?;
            if have != file.size {
                return Err(net(format!(
                    "{} ended early ({have} of {} bytes)",
                    file.name, file.size
                )));
            }
            progress(have);
            if hex(&hasher.finalize()) != file.sha256 {
                return Err(LibraryError::new(
                    ErrorKind::Mismatch,
                    format!(
                        "{} did not match its fingerprint, so it was deleted",
                        file.name
                    ),
                ));
            }
            Ok(())
        })
    }

    /// One `GET` on a fresh HTTP/1.1 connection (the downloads are few and
    /// large; no pooling).
    async fn get(
        &self,
        path: &str,
        extra: &[(hyper::header::HeaderName, &str)],
    ) -> Result<Response<Incoming>, LibraryError> {
        let timeout = Duration::from_millis(self.cfg.connect_timeout_ms.max(1000));
        let io = tokio::time::timeout(timeout, crate::conn::dial(&self.cfg))
            .await
            .map_err(|_| {
                net(format!(
                    "{}: no connection within {} ms",
                    self.cfg.addr(),
                    timeout.as_millis()
                ))
            })??;
        let (mut send, conn) = hyper::client::conn::http1::handshake(TokioIo::new(io))
            .await
            .map_err(|e| net(format!("{}: {e}", self.cfg.addr())))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let mut req = Request::get(path)
            .header(HOST, self.cfg.addr())
            .header(USER_AGENT, self.user_agent.as_str());
        if !self.cfg.token.is_empty() {
            req = req.header(AUTHORIZATION, format!("Bearer {}", self.cfg.token));
        }
        for (k, v) in extra {
            req = req.header(k, *v);
        }
        let req = req
            .body(Empty::<Bytes>::new())
            .map_err(|_| LibraryError::new(ErrorKind::Auth, "the key has invalid characters"))?;
        tokio::time::timeout(HEAD_TIMEOUT, send.send_request(req))
            .await
            .map_err(|_| net(format!("{}: no answer", self.cfg.addr())))?
            .map_err(|e| net(format!("{}: {e}", self.cfg.addr())))
    }
}

fn net(m: impl Into<String>) -> LibraryError {
    LibraryError::new(ErrorKind::Network, m)
}

fn io_err(what: &str, e: std::io::Error) -> LibraryError {
    LibraryError::new(ErrorKind::Io, format!("could not {what}: {e}"))
}

fn check_status(r: &Response<Incoming>) -> Result<(), LibraryError> {
    let s = r.status();
    match s.as_u16() {
        200 | 206 => Ok(()),
        401 | 403 => Err(LibraryError::new(
            ErrorKind::Auth,
            format!("the server refused the key (HTTP {})", s.as_u16()),
        )),
        404 => Err(LibraryError::new(
            ErrorKind::NotFound,
            "not on the server (any more): refresh the list",
        )),
        503 => {
            let after = r
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("60");
            Err(LibraryError::new(
                ErrorKind::Unavailable,
                format!("the server is rechecking this file; try again in {after} s"),
            ))
        }
        _ => Err(LibraryError::new(
            ErrorKind::Protocol,
            format!("the server answered HTTP {}", s.as_u16()),
        )),
    }
}

/// `bytes <start>-<end>/<size>` → `start`.
fn content_range_start(v: &str) -> Option<u64> {
    v.trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes read (≤ `len`, less when the file is shorter) and the SHA-256
/// state over the first `len` bytes of `p`.
fn hash_prefix(
    p: &Path,
    len: u64,
    cancel: &dyn Fn() -> bool,
) -> Result<(u64, Sha256), LibraryError> {
    let mut f = std::fs::File::open(p).map_err(|e| io_err("read the file", e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut left = len;
    while left > 0 {
        if cancel() {
            return Err(LibraryError::new(ErrorKind::Cancelled, "cancelled"));
        }
        let want = buf.len().min(left as usize);
        let n = f
            .read(&mut buf[..want])
            .map_err(|e| io_err("read the file", e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        left -= n as u64;
    }
    Ok((len - left, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_relative_and_safe() {
        for ok in ["ggml-base.bin", "sub/tokens.txt", "a/b/c/d/e.onnx"] {
            assert!(is_safe_file_name(ok), "{ok}");
        }
        for bad in [
            "",
            "/abs",
            "../x",
            "a/../b",
            "a//b",
            "a\\b",
            "c:x",
            ".hidden",
            "a/b/c/d/e/f",
        ] {
            assert!(!is_safe_file_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn content_range_parses_the_start() {
        assert_eq!(content_range_start("bytes 10-19/20"), Some(10));
        assert_eq!(content_range_start("bytes */20"), None);
        assert_eq!(content_range_start("items 1-2/3"), None);
    }

    #[test]
    fn unsafe_models_are_dropped() {
        let f = |name: &str, sha: &str| LibraryFile {
            name: name.into(),
            size: 1,
            sha256: sha.into(),
        };
        let m = |id: &str, files: Vec<LibraryFile>| LibraryModel {
            id: id.into(),
            title: "t".into(),
            kind: flow_protocol::library::ModelKind::Stt,
            language: None,
            catalogue_id: None,
            engine: None,
            files,
        };
        let good = "a".repeat(64);
        assert!(safe_model(&m("ok", vec![f("x.bin", &good)])));
        assert!(!safe_model(&m("..", vec![f("x.bin", &good)])));
        assert!(!safe_model(&m("ok", vec![f("../x.bin", &good)])));
        assert!(!safe_model(&m("ok", vec![f("x.bin", "AB")])));
        assert!(!safe_model(&m("ok", vec![])));
    }

    #[test]
    fn plaintext_to_a_public_host_is_refused_before_any_socket() {
        let cfg = ServerConfig {
            host: "example.com".into(),
            port: 8443,
            tls: false,
            ..Default::default()
        };
        let e = LibraryClient::new(cfg, "t").err().unwrap();
        assert_eq!(e.kind, ErrorKind::Network);
        let e = LibraryClient::new(ServerConfig::default(), "t")
            .err()
            .unwrap();
        assert_eq!(e.kind, ErrorKind::NotConfigured);
    }
}
