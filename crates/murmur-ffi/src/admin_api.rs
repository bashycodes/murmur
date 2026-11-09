//! D39 admin API for the Android "Users & keys" page (feature `admin`, only
//! in the `hybrid` / `remote` flavours). JSON in, JSON out over
//! [`murmur_remote::admin::AdminClient`], so the JNI layer (`jni_admin`) stays
//! a thin string shim and this is testable without a JVM.
//!
//! Every function takes the saved server config (`ServerConfig` JSON, the
//! same one `nativeProbeServer` gets), blocks for at most
//! `murmur_remote::admin::REQUEST_TIMEOUT`, and never fails: it answers
//! `{"ok":true,"value":<answer>}` or `{"ok":false,"code":"…","message":"…"}`
//! (`code` = the `murmur_protocol::admin::code` values).
//!
//! Privacy: nothing here logs. A created key's secret exists only in the
//! returned string; the decoded copy is wiped before returning.

use murmur_protocol::admin::{code, AdminError};
use murmur_remote::admin::AdminClient;
use murmur_router::ServerConfig;
use serde::Serialize;

fn client(server_json: &str) -> Result<AdminClient, AdminError> {
    let cfg: ServerConfig = serde_json::from_str(server_json)
        .map_err(|e| AdminError::new(code::BAD_REQUEST, format!("bad server config: {e}")))?;
    if cfg.host.is_empty() || cfg.token.is_empty() {
        return Err(AdminError::new(
            code::BAD_REQUEST,
            "save a server address and token first",
        ));
    }
    Ok(AdminClient::new(
        cfg,
        &format!("murmur-android {}", murmur_build_info::FULL),
    ))
}

fn ok<T: Serialize>(v: &T) -> String {
    #[derive(Serialize)]
    struct Ok<'a, T> {
        ok: bool,
        value: &'a T,
    }
    serde_json::to_string(&Ok { ok: true, value: v }).unwrap_or_else(|e| {
        err(&AdminError::new(
            code::SERVER_ERROR,
            format!("encode answer: {e}"),
        ))
    })
}

fn err(e: &AdminError) -> String {
    serde_json::json!({"ok": false, "code": e.code, "message": e.message}).to_string()
}

fn answer<T: Serialize>(r: Result<T, AdminError>) -> String {
    match r {
        Ok(v) => ok(&v),
        Err(e) => err(&e),
    }
}

/// `GET /v1/whoami`: `{id, name, admin}` of the config's key.
pub fn whoami(server_json: &str) -> String {
    answer(client(server_json).and_then(|c| c.whoami()))
}

/// `GET /v1/admin/keys`: `{keys:[…]}` (admin key only, else `forbidden`).
pub fn list_keys(server_json: &str) -> String {
    answer(client(server_json).and_then(|c| c.list_keys()))
}

/// `POST /v1/admin/keys`: `{key, secret}`. The only copy of the secret is
/// in the returned string (the caller wipes it after handing it on).
pub fn create_key(server_json: &str, name: &str, admin: bool) -> String {
    let name = name.trim();
    if name.is_empty() {
        return err(&AdminError::new(
            code::BAD_REQUEST,
            "give the key a name, e.g. phone or laptop",
        ));
    }
    match client(server_json).and_then(|c| c.create_key(name, admin)) {
        Ok(mut created) => {
            let s = ok(&created);
            wipe(&mut created.secret);
            s
        }
        Err(e) => err(&e),
    }
}

/// `DELETE /v1/admin/keys/{id}`: `{key}` (refuses the last admin key).
pub fn revoke_key(server_json: &str, id: &str) -> String {
    answer(client(server_json).and_then(|c| c.revoke_key(id)))
}

/// Overwrite a string's bytes in place (best effort: copies made by
/// serde or the allocator are out of reach).
pub fn wipe(s: &mut String) {
    // SAFETY: zero bytes are valid UTF-8.
    unsafe { s.as_bytes_mut().fill(0) };
    s.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn bad_or_missing_config_is_an_answer_not_a_panic() {
        let r = v(&whoami("not json"));
        assert_eq!(r["ok"], false);
        assert_eq!(r["code"], code::BAD_REQUEST);
        let r = v(&list_keys(r#"{"host":"","token":""}"#));
        assert_eq!(r["code"], code::BAD_REQUEST);
        assert!(r["message"].as_str().unwrap().contains("save a server"));
    }

    #[test]
    fn create_needs_a_name_and_revoke_a_sane_id() {
        let cfg = r#"{"host":"127.0.0.1","port":1,"token":"secret-token-0123456789"}"#;
        assert_eq!(v(&create_key(cfg, "  ", false))["code"], code::BAD_REQUEST);
        assert_eq!(v(&revoke_key(cfg, "../x"))["code"], code::BAD_REQUEST);
    }

    #[test]
    fn plaintext_to_a_public_host_is_refused_before_any_socket() {
        let cfg = r#"{"host":"8.8.8.8","token":"secret-token-0123456789"}"#;
        assert_eq!(v(&whoami(cfg))["code"], code::CONNECT_FAILED);
    }

    #[test]
    fn wipe_zeroes_and_clears() {
        let mut s = String::from("abc");
        wipe(&mut s);
        assert!(s.is_empty());
    }
}
