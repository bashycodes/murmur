//! The OpenRouter API key at rest (D18 §8.1 / D28, invariant C3).
//!
//! Keys live only in the OS secure store and in Rust memory: never in
//! `prefs.json`, Tauri events, logs or crash text. On Windows the store is
//! Credential Manager, one generic credential named
//! `murmur/cloud/openrouter` (D28: OpenRouter is the only cloud
//! provider; [`apply_migration`] moves / deletes the D18 per-provider
//! entries `murmur/cloud/<id>` once) (`CRED_PERSIST_LOCAL_MACHINE`: this
//! user on this PC, not roamed). Other hosts (the Linux test/CI build) use
//! [`MemoryStore`], the keyring stand-in the tests run against.
//!
//! The webview only ever sees [`KeyState`] (set / not set + `••••` and the
//! last 4 characters), never the key after it was entered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Credential Manager target name of a provider's key.
pub fn target_name(provider_id: &str) -> String {
    format!("murmur/cloud/{provider_id}")
}

/// D83: the target name before the Murmur rename. Read (and copied to
/// [`target_name`]) when the new one is missing; never written.
pub fn old_target_name(provider_id: &str) -> String {
    format!("flow-clone/cloud/{provider_id}")
}

/// [`CredentialStore::get`] over raw target reads/writes: the key under
/// [`target_name`], else one under [`old_target_name`], which is copied to
/// the new target on the way (the old entry stays for the old app).
fn get_or_pick_up(
    read: impl Fn(&str) -> Result<Option<String>, String>,
    write: impl Fn(&str, &str) -> Result<(), String>,
    provider_id: &str,
) -> Result<Option<String>, String> {
    let new = target_name(provider_id);
    if let Some(v) = read(&new)? {
        return Ok(Some(v));
    }
    match read(&old_target_name(provider_id)) {
        Ok(Some(v)) => {
            let _ = write(&new, &v);
            Ok(Some(v))
        }
        _ => Ok(None),
    }
}

/// Secure storage for provider keys, keyed by provider id.
pub trait CredentialStore: Send + Sync {
    /// The stored key, `None` when there is none.
    fn get(&self, provider_id: &str) -> Result<Option<String>, String>;
    fn set(&self, provider_id: &str, key: &str) -> Result<(), String>;
    /// Removing a key that does not exist is not an error.
    fn delete(&self, provider_id: &str) -> Result<(), String>;
}

/// In-memory store: the Linux stand-in for Credential Manager (tests, CI).
/// Keyed by [`target_name`] so it mirrors what Windows stores.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Target names currently stored (tests).
    pub fn targets(&self) -> Vec<String> {
        let mut v: Vec<String> = self.map.lock().unwrap().keys().cloned().collect();
        v.sort();
        v
    }
}

impl CredentialStore for MemoryStore {
    fn get(&self, provider_id: &str) -> Result<Option<String>, String> {
        get_or_pick_up(
            |t| Ok(self.map.lock().unwrap().get(t).cloned()),
            |t, k| {
                self.map.lock().unwrap().insert(t.into(), k.into());
                Ok(())
            },
            provider_id,
        )
    }
    fn set(&self, provider_id: &str, key: &str) -> Result<(), String> {
        self.map
            .lock()
            .unwrap()
            .insert(target_name(provider_id), key.to_string());
        Ok(())
    }
    fn delete(&self, provider_id: &str) -> Result<(), String> {
        let mut m = self.map.lock().unwrap();
        m.remove(&target_name(provider_id));
        // Or the next get would pick the old one up again.
        m.remove(&old_target_name(provider_id));
        Ok(())
    }
}

/// The store the app uses: Credential Manager on Windows, memory elsewhere.
pub fn system_store() -> Arc<dyn CredentialStore> {
    #[cfg(windows)]
    {
        Arc::new(win::WindowsCredentialStore)
    }
    #[cfg(not(windows))]
    {
        MemoryStore::new()
    }
}

/// What the Developer page may know about a key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KeyState {
    pub set: bool,
    /// `••••` + last 4 characters (`""` when not set). Keys of 8 characters
    /// or fewer show only the dots.
    pub masked: String,
}

pub fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n == 0 {
        return String::new();
    }
    if n <= 8 {
        return "••••".into();
    }
    let tail: String = key.chars().skip(n - 4).collect();
    format!("••••{tail}")
}

pub fn key_state(store: &dyn CredentialStore, provider_id: &str) -> KeyState {
    match store.get(provider_id) {
        Ok(Some(raw)) if !decode(&raw).key.is_empty() => KeyState {
            set: true,
            masked: mask(&decode(&raw).key),
        },
        _ => KeyState {
            set: false,
            masked: String::new(),
        },
    }
}

/// A key as typed/pasted: trimmed; must be one line of printable ASCII
/// (every provider issues such keys; this also keeps CR/LF out of headers).
pub fn normalize_key(raw: &str) -> Result<String, String> {
    let k = raw.trim();
    if k.is_empty() {
        return Err("Enter an API key".into());
    }
    if !k.bytes().all(|b| (0x21..0x7f).contains(&b)) {
        return Err("API key must be one line without spaces".into());
    }
    if k.len() > 512 {
        return Err("API key is too long".into());
    }
    Ok(k.to_string())
}

// ------------------------------------------------ host + adapter binding

/// The id of the one OpenRouter key (D28): Credential Manager
/// `murmur/cloud/openrouter`.
pub const OPENROUTER: &str = murmur_router::cloud::OPENROUTER_ID;

/// A stored key and the endpoint it was saved for (review S4): the host
/// and adapter. D28: the only binding is `(openrouter.ai, openrouter)`;
/// a key bound to anything else is never handed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredKey {
    pub key: String,
    /// `(host, adapter)`; `None` = a key stored before bindings existed.
    pub bound: Option<(String, String)>,
}

const BOUND_V1: &str = "murmur-bound-v1\n";
/// D83: the blob header before the Murmur rename; [`decode`] reads it.
const OLD_BOUND_V1: &str = "flow-bound-v1\n";

/// The binding of the OpenRouter key.
pub fn openrouter_binding() -> (String, String) {
    (
        murmur_router::cloud::OPENROUTER_HOST.to_string(),
        "openrouter".to_string(),
    )
}

/// Credential blob: `murmur-bound-v1\n<host>\n<adapter>\n<key>`.
pub fn encode(key: &str, bound: &(String, String)) -> String {
    format!("{BOUND_V1}{}\n{}\n{key}", bound.0, bound.1)
}

pub fn decode(raw: &str) -> StoredKey {
    if let Some(rest) = raw
        .strip_prefix(BOUND_V1)
        .or_else(|| raw.strip_prefix(OLD_BOUND_V1))
    {
        let mut it = rest.splitn(3, '\n');
        if let (Some(h), Some(a), Some(k)) = (it.next(), it.next(), it.next()) {
            return StoredKey {
                key: k.to_string(),
                bound: Some((h.to_string(), a.to_string())),
            };
        }
    }
    StoredKey {
        key: raw.to_string(),
        bound: None,
    }
}

/// Save the OpenRouter key (bound to openrouter.ai + the openrouter adapter).
pub fn store_openrouter_key(store: &dyn CredentialStore, key: &str) -> Result<(), String> {
    store.set(OPENROUTER, &encode(key, &openrouter_binding()))
}

/// The OpenRouter key to hand the router, or `None`. A legacy (unbound)
/// key under `openrouter` is bound on the way; a key bound to any other
/// host or adapter is deleted (never sent to openrouter.ai).
pub fn openrouter_key(store: &dyn CredentialStore) -> Option<String> {
    let raw = store.get(OPENROUTER).ok().flatten()?;
    let sk = decode(&raw);
    if sk.key.is_empty() {
        return None;
    }
    match &sk.bound {
        Some(b) if *b == openrouter_binding() => Some(sk.key),
        Some(_) => {
            let _ = store.delete(OPENROUTER);
            None
        }
        None => {
            let _ = store_openrouter_key(store, &sk.key);
            Some(sk.key)
        }
    }
}

/// D52: the id of the optional key for the user's own OpenAI-compatible
/// cleanup server: Credential Manager `murmur/cloud/custom`. Not bound
/// to a host (the base URL is the user's to change); the router only calls
/// a server whose host the user consented to, so a changed URL sends
/// nothing, key included, until that consent is given again.
pub const CUSTOM: &str = murmur_router::cloud::CUSTOM_CLEANUP_ID;

/// Save the custom cleanup server's key.
pub fn store_custom_key(store: &dyn CredentialStore, key: &str) -> Result<(), String> {
    store.set(CUSTOM, key)
}

/// The custom cleanup server's key to hand the router, or `None` (the key
/// is optional: LiteLLM without a master key, Ollama, vLLM).
pub fn custom_key(store: &dyn CredentialStore) -> Option<String> {
    let raw = store.get(CUSTOM).ok().flatten()?;
    let k = decode(&raw).key;
    (!k.is_empty()).then_some(k)
}

/// D52: the key the settings page's custom-server calls ("Load models",
/// "Test connection", "Test cleanup") send to `base_url`: a key typed into
/// the field (it goes where the user points it), else the saved key only
/// when `base_url`'s host is the consented one
/// (`CustomCleanupConfig::saved_key_allowed_for`), else none — so a URL
/// typed into the page never receives the saved key.
pub fn custom_key_for(
    typed: Option<&str>,
    saved: impl FnOnce() -> Option<String>,
    cfg: &murmur_router::cloud::CustomCleanupConfig,
    base_url: &str,
) -> Result<Option<String>, String> {
    match typed.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => normalize_key(k).map(Some),
        None if cfg.saved_key_allowed_for(base_url) => Ok(saved()),
        None => Ok(None),
    }
}

/// A key is saved but [`custom_key_for`] would not send it to `base_url`
/// (no key was typed and that host has no consent): the page must ask
/// for consent or say so, never call the server without the key it expects.
pub fn saved_custom_key_withheld(
    typed: bool,
    has_saved: bool,
    cfg: &murmur_router::cloud::CustomCleanupConfig,
    base_url: &str,
) -> bool {
    !typed && has_saved && !cfg.saved_key_allowed_for(base_url)
}

/// What a D28 migration did to the key store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMigration {
    /// The kept OpenRouter key moved from this id to `openrouter`.
    pub moved_from: Option<String>,
    /// Ids whose key was deleted (other providers; a kept entry's key that
    /// was bound to another host).
    pub deleted: Vec<String>,
    /// Store errors (ids only, never a key).
    pub errors: Vec<String>,
}

/// Apply `RouterConfig::migrate_d28`'s report to Credential Manager: move
/// `murmur/cloud/<key_from>` to `murmur/cloud/openrouter` (only if
/// that key was saved for openrouter.ai: bound to it, or a legacy unbound
/// key of an entry whose base URL was openrouter.ai, `legacy_host`), and
/// delete every `delete_keys` entry. Idempotent.
pub fn apply_migration(
    store: &dyn CredentialStore,
    report: &murmur_router::MigrationReport,
    legacy_host: &dyn Fn(&str) -> Option<String>,
) -> KeyMigration {
    let mut out = KeyMigration::default();
    let del = |id: &str, out: &mut KeyMigration| match store.delete(id) {
        Ok(()) => out.deleted.push(id.to_string()),
        Err(_) => out.errors.push(id.to_string()),
    };
    if let Some(from) = report.key_from.as_deref().filter(|f| *f != OPENROUTER) {
        match store.get(from) {
            Ok(Some(raw)) => {
                let sk = decode(&raw);
                let for_openrouter = match &sk.bound {
                    Some(b) => *b == openrouter_binding(),
                    None => {
                        legacy_host(from).as_deref() == Some(murmur_router::cloud::OPENROUTER_HOST)
                    }
                };
                if for_openrouter && !sk.key.is_empty() {
                    if store_openrouter_key(store, &sk.key).is_ok() {
                        out.moved_from = Some(from.to_string());
                        del(from, &mut out);
                        out.deleted.retain(|d| d != from);
                    } else {
                        out.errors.push(from.to_string());
                    }
                } else {
                    del(from, &mut out);
                }
            }
            Ok(None) => {}
            Err(_) => out.errors.push(from.to_string()),
        }
    }
    for id in &report.delete_keys {
        if id != OPENROUTER {
            del(id, &mut out);
        }
    }
    out
}

#[cfg(windows)]
mod win {
    use super::{get_or_pick_up, old_target_name, target_name, CredentialStore};
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_FLAGS,
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    pub struct WindowsCredentialStore;

    fn not_found(e: &windows::core::Error) -> bool {
        e.code() == ERROR_NOT_FOUND.to_hresult()
    }

    fn read(target: &str) -> Result<Option<String>, String> {
        let target = HSTRING::from(target);
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        unsafe {
            if let Err(e) = CredReadW(&target, CRED_TYPE_GENERIC, 0, &mut p) {
                return if not_found(&e) {
                    Ok(None)
                } else {
                    Err(format!("Credential Manager read failed ({})", e.code()))
                };
            }
            let c = &*p;
            let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)
                .to_vec();
            CredFree(p as *const _);
            String::from_utf8(blob)
                .map(Some)
                .map_err(|_| "stored key is not UTF-8".to_string())
        }
    }

    fn write(target: &str, key: &str) -> Result<(), String> {
        let mut target: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
        let mut user: Vec<u16> = "murmur".encode_utf16().chain(Some(0)).collect();
        let mut blob = key.as_bytes().to_vec();
        let cred = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(target.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: PWSTR(user.as_mut_ptr()),
            ..Default::default()
        };
        let r = unsafe { CredWriteW(&cred, 0) };
        // Wipe our copy of the key bytes.
        for b in blob.iter_mut() {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
        r.map_err(|e| format!("Credential Manager write failed ({})", e.code()))
    }

    fn remove(target: &str) -> Result<(), String> {
        let target = HSTRING::from(target);
        match unsafe { CredDeleteW(&target, CRED_TYPE_GENERIC, 0) } {
            Ok(()) => Ok(()),
            Err(e) if not_found(&e) => Ok(()),
            Err(e) => Err(format!("Credential Manager delete failed ({})", e.code())),
        }
    }

    impl CredentialStore for WindowsCredentialStore {
        fn get(&self, provider_id: &str) -> Result<Option<String>, String> {
            get_or_pick_up(read, write, provider_id)
        }

        fn set(&self, provider_id: &str, key: &str) -> Result<(), String> {
            write(&target_name(provider_id), key)
        }

        fn delete(&self, provider_id: &str) -> Result<(), String> {
            remove(&target_name(provider_id))?;
            // Or the next get would pick the old one up again (D83).
            remove(&old_target_name(provider_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_is_a_keyring_stand_in() {
        let s = MemoryStore::new();
        assert_eq!(s.get("legacy-1").unwrap(), None);
        s.set("legacy-1", "sk-0123456789abcdef").unwrap();
        assert_eq!(
            s.get("legacy-1").unwrap().as_deref(),
            Some("sk-0123456789abcdef")
        );
        assert_eq!(s.targets(), vec!["murmur/cloud/legacy-1".to_string()]);
        s.set("legacy-1", "sk-other-key-000000").unwrap();
        assert_eq!(
            s.get("legacy-1").unwrap().as_deref(),
            Some("sk-other-key-000000")
        );
        s.delete("legacy-1").unwrap();
        s.delete("legacy-1").unwrap(); // idempotent
        assert_eq!(s.get("legacy-1").unwrap(), None);
        assert!(s.targets().is_empty());
    }

    #[test]
    fn masking_shows_at_most_the_last_four() {
        assert_eq!(mask(""), "");
        assert_eq!(mask("short"), "••••");
        assert_eq!(mask("sk-or-v1-0123456789abcdef"), "••••cdef");
        let s = MemoryStore::new();
        assert_eq!(key_state(&*s, "x").masked, "");
        s.set("x", "sk-LEAKCHECK-0123456789").unwrap();
        let ks = key_state(&*s, "x");
        assert!(ks.set);
        assert!(!ks.masked.contains("LEAKCHECK"));
    }

    #[test]
    fn keys_are_one_printable_line() {
        assert_eq!(normalize_key("  sk-abc \n").unwrap(), "sk-abc");
        assert!(normalize_key("").is_err());
        assert!(normalize_key("sk abc").is_err());
        assert!(normalize_key("sk-a\r\nX-Evil: 1").is_err());
        assert!(normalize_key("ключ").is_err());
    }

    #[test]
    fn the_custom_key_is_its_own_optional_entry() {
        let s = MemoryStore::new();
        assert_eq!(custom_key(&*s), None);
        store_openrouter_key(&*s, "sk-or-v1-abcdefgh1234").unwrap();
        assert_eq!(custom_key(&*s), None, "the OpenRouter key is not it");
        store_custom_key(&*s, "sk-litellm-LEAKCHECK-0001").unwrap();
        assert_eq!(
            custom_key(&*s).as_deref(),
            Some("sk-litellm-LEAKCHECK-0001")
        );
        assert!(s.targets().contains(&"murmur/cloud/custom".to_string()));
        assert_eq!(key_state(&*s, CUSTOM).masked, "••••0001");
        assert_eq!(
            openrouter_key(&*s).as_deref(),
            Some("sk-or-v1-abcdefgh1234")
        );
        s.delete(CUSTOM).unwrap();
        assert_eq!(custom_key(&*s), None);
        assert!(!key_state(&*s, CUSTOM).set);
    }

    #[test]
    fn the_openrouter_key_is_bound_to_openrouter_ai() {
        let s = MemoryStore::new();
        store_openrouter_key(&*s, "sk-or-v1-abcdefgh1234").unwrap();
        let raw = s.get(OPENROUTER).unwrap().unwrap();
        assert_eq!(
            decode(&raw),
            StoredKey {
                key: "sk-or-v1-abcdefgh1234".into(),
                bound: Some(("openrouter.ai".into(), "openrouter".into()))
            }
        );
        assert_eq!(s.targets(), vec!["murmur/cloud/openrouter".to_string()]);
        assert_eq!(key_state(&*s, OPENROUTER).masked, "••••1234");
        assert_eq!(
            openrouter_key(&*s).as_deref(),
            Some("sk-or-v1-abcdefgh1234")
        );
    }

    #[test]
    fn a_key_bound_elsewhere_is_never_handed_out() {
        let s = MemoryStore::new();
        s.set(
            OPENROUTER,
            &encode(
                "sk-abcdefgh1234",
                &("stt.example.net".into(), "openai_compat".into()),
            ),
        )
        .unwrap();
        assert_eq!(openrouter_key(&*s), None);
        assert_eq!(s.get(OPENROUTER).unwrap(), None, "and deleted");
    }

    #[test]
    fn a_legacy_unbound_key_is_bound_on_first_use() {
        let s = MemoryStore::new();
        s.set(OPENROUTER, "sk-or-legacy12345").unwrap();
        assert_eq!(openrouter_key(&*s).as_deref(), Some("sk-or-legacy12345"));
        assert!(decode(&s.get(OPENROUTER).unwrap().unwrap()).bound.is_some());
    }

    fn report(from: Option<&str>, delete: &[&str]) -> murmur_router::MigrationReport {
        murmur_router::MigrationReport {
            migrated: true,
            key_from: from.map(str::to_string),
            key_to: from.map(|_| OPENROUTER.to_string()),
            delete_keys: delete.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn migration_moves_the_openrouter_key_and_deletes_the_rest() {
        let s = MemoryStore::new();
        s.set(
            "openrouter-1",
            &encode(
                "sk-or-v1-LEAKCHECK-0001",
                &("openrouter.ai".into(), "openrouter".into()),
            ),
        )
        .unwrap();
        s.set(
            "legacy-1",
            &encode(
                "sk-LEAKCHECK-0002",
                &("stt.example.net".into(), "openai_compat".into()),
            ),
        )
        .unwrap();
        let m = apply_migration(&*s, &report(Some("openrouter-1"), &["legacy-1"]), &|_| None);
        assert_eq!(m.moved_from.as_deref(), Some("openrouter-1"));
        assert_eq!(m.deleted, vec!["legacy-1".to_string()]);
        assert_eq!(s.targets(), vec!["murmur/cloud/openrouter".to_string()]);
        assert_eq!(
            openrouter_key(&*s).as_deref(),
            Some("sk-or-v1-LEAKCHECK-0001")
        );
        // Idempotent: a second run changes nothing.
        let again = apply_migration(&*s, &report(Some("openrouter-1"), &["legacy-1"]), &|_| None);
        assert_eq!(again.moved_from, None);
        assert_eq!(
            openrouter_key(&*s).as_deref(),
            Some("sk-or-v1-LEAKCHECK-0001")
        );
    }

    #[test]
    fn migration_never_moves_a_key_saved_for_another_host() {
        // An openrouter-adapter entry whose base URL was edited to another
        // host: its key belongs to that host, not to openrouter.ai.
        let s = MemoryStore::new();
        s.set(
            "openrouter-1",
            &encode(
                "sk-LEAKCHECK-other",
                &("or.example.net".into(), "openrouter".into()),
            ),
        )
        .unwrap();
        let m = apply_migration(&*s, &report(Some("openrouter-1"), &[]), &|_| None);
        assert_eq!(m.moved_from, None);
        assert!(s.targets().is_empty(), "{:?}", s.targets());
        // A legacy unbound key moves only if its entry pointed at openrouter.ai.
        s.set("openrouter-2", "sk-or-legacy-0003").unwrap();
        let host = |id: &str| (id == "openrouter-2").then(|| "openrouter.ai".to_string());
        let m = apply_migration(&*s, &report(Some("openrouter-2"), &[]), &host);
        assert_eq!(m.moved_from.as_deref(), Some("openrouter-2"));
        assert_eq!(openrouter_key(&*s).as_deref(), Some("sk-or-legacy-0003"));
        s.set("openrouter-3", "sk-legacy-elsewhere").unwrap();
        let m = apply_migration(&*s, &report(Some("openrouter-3"), &[]), &|_| None);
        assert_eq!(m.moved_from, None);
        assert_eq!(s.get("openrouter-3").unwrap(), None);
    }

    #[test]
    fn custom_key_goes_to_the_consented_host_only() {
        let cfg = murmur_router::cloud::CustomCleanupConfig {
            base_url: "http://10.0.0.5:4000".into(),
            consent_host: "10.0.0.5".into(),
            consent_at_ms: 1,
            ..Default::default()
        };
        let saved = || Some("sk-saved-LEAKCHECK".to_string());
        let pick = |typed: Option<&str>, url: &str| custom_key_for(typed, saved, &cfg, url);
        // A typed key wins and goes wherever the user points it.
        assert_eq!(
            pick(Some(" sk-typed "), "http://elsewhere.example").unwrap(),
            Some("sk-typed".into())
        );
        // The saved key: consented host only.
        assert_eq!(
            pick(None, "http://10.0.0.5:4000/v1").unwrap(),
            Some("sk-saved-LEAKCHECK".into())
        );
        assert_eq!(
            pick(Some("  "), "http://10.0.0.5:9000").unwrap().as_deref(),
            Some("sk-saved-LEAKCHECK")
        );
        assert_eq!(pick(None, "http://attacker.example:4000").unwrap(), None);
        assert_eq!(pick(None, "http://10.0.0.6:4000").unwrap(), None);
        assert_eq!(pick(None, "not a url").unwrap(), None);
        let unconsented = murmur_router::cloud::CustomCleanupConfig {
            consent_at_ms: 0,
            ..cfg.clone()
        };
        assert_eq!(
            custom_key_for(None, saved, &unconsented, "http://10.0.0.5:4000").unwrap(),
            None
        );
        assert!(pick(Some("two words"), "http://10.0.0.5:4000").is_err());
    }

    #[test]
    fn a_key_stored_before_the_rename_is_picked_up_once_and_kept_for_the_old_app() {
        let s = MemoryStore::new();
        let old_blob = format!(
            "flow-bound-v1\n{}\nopenrouter\nsk-synthetic-old",
            murmur_router::cloud::OPENROUTER_HOST
        );
        s.map
            .lock()
            .unwrap()
            .insert(old_target_name(OPENROUTER), old_blob.clone());
        assert_eq!(
            openrouter_key(s.as_ref()).as_deref(),
            Some("sk-synthetic-old")
        );
        // Copied to the new target, left under the old one.
        assert_eq!(
            s.map.lock().unwrap().get(&target_name(OPENROUTER)),
            Some(&old_blob)
        );
        assert_eq!(
            s.map.lock().unwrap().get(&old_target_name(OPENROUTER)),
            Some(&old_blob)
        );
        // The new one wins from now on.
        store_openrouter_key(s.as_ref(), "sk-synthetic-new").unwrap();
        assert_eq!(
            openrouter_key(s.as_ref()).as_deref(),
            Some("sk-synthetic-new")
        );
        // Deleting removes both, so it does not come back.
        s.delete(OPENROUTER).unwrap();
        assert_eq!(s.get(OPENROUTER).unwrap(), None);
    }
}
