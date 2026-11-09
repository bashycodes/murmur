//! Model library (D40 server, D41 client) for the Android shell: JSON in,
//! JSON out over `murmur_remote::library` (feature `library`, only in the
//! `hybrid` / `remote` flavours; the `local` library has no network code).
//!
//! * [`list`]: `ServerConfig` JSON → `{"ok":true,"models":[…]}`, the
//!   server's `ModelList` with unsafe entries already dropped.
//! * [`download`]: one file of one model into `dest` through `part`,
//!   verified against the `LibraryFile` JSON the shell passes (its built-in
//!   pin for a catalogue copy, the server's listing for a custom model).
//!   Resumable: an interrupted download keeps `part` and the next call asks
//!   for the rest; a cancel or a fingerprint mismatch deletes it.
//!
//! Failures: `{"ok":false,"error":"<kind>","message":"…"}` with kind one of
//! `bad_config`, `not_configured`, `network`, `auth`, `not_found`,
//! `unavailable`, `protocol`, `mismatch`, `io`, `cancelled`. Both calls
//! block (call them off the main thread). The key never appears in a
//! result.

use std::path::Path;

use murmur_protocol::library::LibraryFile;
use murmur_remote::library::{ErrorKind, LibraryClient, LibraryError};
use murmur_router::ServerConfig;
use serde_json::json;

/// `User-Agent` of the shell's library requests.
fn client_name() -> String {
    format!("murmur-ffi {}", murmur_build_info::FULL)
}

pub fn kind_str(k: ErrorKind) -> &'static str {
    match k {
        ErrorKind::NotConfigured => "not_configured",
        ErrorKind::Network => "network",
        ErrorKind::Auth => "auth",
        ErrorKind::NotFound => "not_found",
        ErrorKind::Unavailable => "unavailable",
        ErrorKind::Protocol => "protocol",
        ErrorKind::Mismatch => "mismatch",
        ErrorKind::Io => "io",
        ErrorKind::Cancelled => "cancelled",
    }
}

pub fn error_json(kind: &str, message: &str) -> String {
    json!({"ok": false, "error": kind, "message": message}).to_string()
}

fn lib_error(e: &LibraryError) -> String {
    error_json(kind_str(e.kind), &e.message)
}

fn client(server_json: &str) -> Result<LibraryClient, String> {
    let cfg: ServerConfig = serde_json::from_str(server_json)
        .map_err(|e| error_json("bad_config", &format!("server config: {e}")))?;
    LibraryClient::new(cfg, &client_name()).map_err(|e| lib_error(&e))
}

/// `GET /v1/models` on the server `server_json` names.
pub fn list(server_json: &str) -> String {
    let c = match client(server_json) {
        Ok(c) => c,
        Err(e) => return e,
    };
    match c.list() {
        Ok(l) => json!({"ok": true, "models": l.models}).to_string(),
        Err(e) => lib_error(&e),
    }
}

/// Download `file_json` (a `LibraryFile`: name, size, sha256) of model
/// `model_id` into `dest` through `part`. `progress` gets the bytes on disk
/// so far; `cancel` is polled between chunks.
pub fn download(
    server_json: &str,
    model_id: &str,
    file_json: &str,
    dest: &str,
    part: &str,
    progress: &mut dyn FnMut(u64),
    cancel: &dyn Fn() -> bool,
) -> String {
    let file: LibraryFile = match serde_json::from_str(file_json) {
        Ok(f) => f,
        Err(e) => return error_json("bad_config", &format!("file: {e}")),
    };
    if dest.is_empty() || part.is_empty() || dest == part {
        return error_json("bad_config", "dest and part must be two paths");
    }
    let c = match client(server_json) {
        Ok(c) => c,
        Err(e) => return e,
    };
    match c.download(
        model_id,
        &file,
        Path::new(dest),
        Path::new(part),
        progress,
        cancel,
    ) {
        Ok(()) => json!({"ok": true}).to_string(),
        Err(e) => lib_error(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn bad_server_config_is_bad_config() {
        let r = parse(&list("not json"));
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"], "bad_config");
    }

    #[test]
    fn no_host_is_not_configured() {
        let r = parse(&list("{}"));
        assert_eq!(r["error"], "not_configured", "{r}");
    }

    #[test]
    fn cleartext_to_a_public_host_is_refused_before_any_socket() {
        let r = parse(&list(r#"{"host":"example.com","port":7313,"tls":false}"#));
        assert_eq!(r["error"], "network", "{r}");
    }

    #[test]
    fn download_checks_its_arguments_first() {
        let never = || false;
        let r = parse(&download(
            "{}",
            "m",
            "{",
            "/a",
            "/a.part",
            &mut |_| {},
            &never,
        ));
        assert_eq!(r["error"], "bad_config");
        let f = r#"{"name":"x.bin","size":1,"sha256":"aa"}"#;
        let r = parse(&download("{}", "m", f, "/a", "/a", &mut |_| {}, &never));
        assert_eq!(r["error"], "bad_config");
        let r = parse(&download(
            "{}",
            "m",
            f,
            "/a",
            "/a.part",
            &mut |_| {},
            &never,
        ));
        assert_eq!(r["error"], "not_configured");
    }

    #[test]
    fn the_key_never_appears_in_an_error() {
        let token = "secret-token-0123456789";
        let j = format!(r#"{{"host":"example.com","port":7313,"tls":false,"token":"{token}"}}"#);
        assert!(!list(&j).contains(token));
    }
}
