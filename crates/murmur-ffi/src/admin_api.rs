//! D39 admin API for the Android "Users & keys" page (feature `admin`, only
//! in the `hybrid` / `remote` flavours). JSON in, JSON out over
//! [`murmur_remote::admin::AdminClient`], so the JNI layer (`jni_admin`) stays
//! a thin string shim and this is testable without a JVM.
//!
//! Every function takes the saved server config (`ServerConfig` JSON, the
//! same one `nativeProbeServer` gets), blocks for at most
//! `murmur_remote::admin::REQUEST_TIMEOUT`, and never fails: it answers
//! `{"ok":true,"value":<answer>}` or `{"ok":false,"code":"…","message":"…"}`
//! (`code` = the `murmur_protocol::admin::code` values; with `throttled`,
//! D112, also `"retry_after_s":N` when the server sent a wait).
//!
//! Privacy: nothing here logs. A created key's secret exists only in the
//! returned string; the decoded copy is wiped before returning.

use murmur_protocol::admin::{code, AdminError};
use murmur_protocol::dictionary::{check_entry, TeamDictionary, TeamEntry};
use murmur_remote::admin::AdminClient;
use murmur_remote::team_dict::TeamDictCache;
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
    let mut j = serde_json::json!({"ok": false, "code": e.code, "message": e.message});
    if let Some(n) = e.retry_after_s {
        j["retry_after_s"] = n.into();
    }
    j.to_string()
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

/// The team dictionary cache for `server_json`'s server at `cache_path`.
fn team(server_json: &str, cache_path: &str) -> Result<(AdminClient, TeamDictCache), AdminError> {
    if cache_path.trim().is_empty() {
        return Err(AdminError::new(code::BAD_REQUEST, "no cache path"));
    }
    Ok((client(server_json)?, TeamDictCache::new(cache_path)))
}

/// The team dictionary cached at `cache_path` for `server_json`'s server,
/// without any network: `{entries:[{word, replacement?}]}` (empty for no
/// server, another server's cache or none). For the app's start, before
/// the first refresh.
pub fn team_dictionary_cached(server_json: &str, cache_path: &str) -> String {
    let server = serde_json::from_str::<ServerConfig>(server_json)
        .ok()
        .filter(|c| !c.host.trim().is_empty())
        .map(|c| c.addr());
    let entries = match server {
        Some(s) if !cache_path.trim().is_empty() => TeamDictCache::new(cache_path).load(&s),
        _ => Vec::new(),
    };
    ok(&TeamDictionary { entries })
}

/// `GET /v1/dictionary` with the cached ETag; caches a new list. Always
/// `ok`: `{entries, changed, error?:{code,message}}`, where `entries` is the
/// list to use now (the cached copy when the server can't be reached; empty
/// when it has no team dictionary). A bad config is an error answer.
pub fn team_dictionary_refresh(server_json: &str, cache_path: &str) -> String {
    match team(server_json, cache_path) {
        Ok((c, cache)) => ok(&cache.refresh(&c)),
        Err(e) => err(&e),
    }
}

/// `POST /v1/admin/dictionary` (admin key): add `word` (with `replacement`
/// when not blank) or replace the entry with that word, and cache the
/// answer. `{entries}`, or `forbidden` / `bad_request` (the message says
/// what is wrong with the entry) / `too_many`.
pub fn team_dictionary_add(
    server_json: &str,
    cache_path: &str,
    word: &str,
    replacement: &str,
) -> String {
    let e = TeamEntry {
        word: word.into(),
        replacement: Some(replacement.to_string()).filter(|r| !r.trim().is_empty()),
    };
    if let Err(m) = check_entry(&e) {
        return err(&AdminError::new(code::BAD_REQUEST, m));
    }
    answer(team(server_json, cache_path).and_then(|(c, cache)| {
        let a = c.add_team_entry(&e)?;
        Ok(TeamDictionary {
            entries: cache.edited(&c, &a),
        })
    }))
}

/// `DELETE /v1/admin/dictionary` (admin key): remove `word` and cache the
/// answer. `{entries}`, or `forbidden` / `not_found`.
pub fn team_dictionary_remove(server_json: &str, cache_path: &str, word: &str) -> String {
    answer(team(server_json, cache_path).and_then(|(c, cache)| {
        let a = c.remove_team_entry(word)?;
        Ok(TeamDictionary {
            entries: cache.edited(&c, &a),
        })
    }))
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
    fn plaintext_to_a_public_host_is_dialled() {
        // D111: no cleartext rule; TEST-NET-3 goes nowhere, so only the
        // network fails, and an old `allow_plaintext` key is ignored.
        let cfg = r#"{"host":"203.0.113.5","tls":false,"allow_plaintext":true,"connect_timeout_ms":300,"token":"secret-token-0123456789"}"#;
        let r = v(&whoami(cfg));
        assert_eq!(r["code"], code::CONNECT_FAILED, "{r}");
        assert!(!r["message"].as_str().unwrap().contains("plaintext"), "{r}");
    }

    #[test]
    fn team_dictionary_without_a_server_or_with_a_bad_entry() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("team.json");
        let p = path.to_str().unwrap();
        // No server: nothing cached, and a refresh is a config error.
        let r = v(&team_dictionary_cached(r#"{"host":""}"#, p));
        assert_eq!(r["value"]["entries"], serde_json::json!([]));
        assert_eq!(v(&team_dictionary_cached("not json", p))["ok"], true);
        let r = v(&team_dictionary_refresh(r#"{"host":"","token":""}"#, p));
        assert_eq!(r["code"], code::BAD_REQUEST);
        // A bad entry is refused before any socket.
        let cfg = r#"{"host":"203.0.113.5","port":1,"token":"secret-token-0123456789"}"#;
        let r = v(&team_dictionary_add(cfg, p, "<b>x</b>", ""));
        assert_eq!(r["code"], code::BAD_REQUEST);
        assert!(r["message"].as_str().unwrap().contains("markup"), "{r}");
        assert_eq!(
            v(&team_dictionary_add(cfg, "", "ok", ""))["code"],
            code::BAD_REQUEST
        );
    }

    #[test]
    fn team_dictionary_offline_answers_the_cache() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("team.json");
        let p = path.to_str().unwrap();
        let cfg = r#"{"host":"127.0.0.1","port":1,"connect_timeout_ms":300,"token":"secret-token-0123456789"}"#;
        TeamDictCache::new(&path)
            .store(
                "127.0.0.1:1",
                &TeamDictionary {
                    entries: vec![TeamEntry {
                        word: "Quillon".into(),
                        replacement: None,
                    }],
                },
                Some("\"t\"".into()),
            )
            .unwrap();
        let r = v(&team_dictionary_cached(cfg, p));
        assert_eq!(r["value"]["entries"][0]["word"], "Quillon");
        let r = v(&team_dictionary_refresh(cfg, p));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["value"]["entries"][0]["word"], "Quillon");
        assert_eq!(r["value"]["changed"], false);
        assert_eq!(r["value"]["error"]["code"], code::CONNECT_FAILED);
    }

    #[test]
    fn wipe_zeroes_and_clears() {
        let mut s = String::from("abc");
        wipe(&mut s);
        assert!(s.is_empty());
    }
}
