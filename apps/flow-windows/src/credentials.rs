//! Cloud STT API keys at rest (D18 §8.1, invariant C3).
//!
//! Keys live only in the OS secure store and in Rust memory: never in
//! `prefs.json`, Tauri events, logs or crash text. On Windows the store is
//! Credential Manager, one generic credential per provider named
//! `flow-clone/cloud/<provider id>` (`CRED_PERSIST_LOCAL_MACHINE`: this
//! user on this PC, not roamed). Other hosts (the Linux test/CI build) use
//! [`MemoryStore`], the keyring stand-in the tests run against.
//!
//! The webview only ever sees [`KeyState`] (set / not set + `••••` and the
//! last 4 characters), never the key after it was entered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Credential Manager target name of a provider's key.
pub fn target_name(provider_id: &str) -> String {
    format!("flow-clone/cloud/{provider_id}")
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
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(&target_name(provider_id))
            .cloned())
    }
    fn set(&self, provider_id: &str, key: &str) -> Result<(), String> {
        self.map
            .lock()
            .unwrap()
            .insert(target_name(provider_id), key.to_string());
        Ok(())
    }
    fn delete(&self, provider_id: &str) -> Result<(), String> {
        self.map.lock().unwrap().remove(&target_name(provider_id));
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
        Ok(Some(k)) if !k.is_empty() => KeyState {
            set: true,
            masked: mask(&k),
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

#[cfg(windows)]
mod win {
    use super::{target_name, CredentialStore};
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

    impl CredentialStore for WindowsCredentialStore {
        fn get(&self, provider_id: &str) -> Result<Option<String>, String> {
            let target = HSTRING::from(target_name(provider_id));
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
                let blob =
                    std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize)
                        .to_vec();
                CredFree(p as *const _);
                String::from_utf8(blob)
                    .map(Some)
                    .map_err(|_| "stored key is not UTF-8".to_string())
            }
        }

        fn set(&self, provider_id: &str, key: &str) -> Result<(), String> {
            let mut target: Vec<u16> = target_name(provider_id)
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut user: Vec<u16> = "flow-clone".encode_utf16().chain(Some(0)).collect();
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

        fn delete(&self, provider_id: &str) -> Result<(), String> {
            let target = HSTRING::from(target_name(provider_id));
            match unsafe { CredDeleteW(&target, CRED_TYPE_GENERIC, 0) } {
                Ok(()) => Ok(()),
                Err(e) if not_found(&e) => Ok(()),
                Err(e) => Err(format!("Credential Manager delete failed ({})", e.code())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_is_a_keyring_stand_in() {
        let s = MemoryStore::new();
        assert_eq!(s.get("groq-1").unwrap(), None);
        s.set("groq-1", "gsk_0123456789abcdef").unwrap();
        assert_eq!(
            s.get("groq-1").unwrap().as_deref(),
            Some("gsk_0123456789abcdef")
        );
        assert_eq!(s.targets(), vec!["flow-clone/cloud/groq-1".to_string()]);
        s.set("groq-1", "gsk_other-key-000000").unwrap();
        assert_eq!(
            s.get("groq-1").unwrap().as_deref(),
            Some("gsk_other-key-000000")
        );
        s.delete("groq-1").unwrap();
        s.delete("groq-1").unwrap(); // idempotent
        assert_eq!(s.get("groq-1").unwrap(), None);
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
}
