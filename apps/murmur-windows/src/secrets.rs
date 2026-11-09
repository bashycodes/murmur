//! Secrets at rest (hybrid-architecture §2.5): the server token — and any
//! later secret such as a cloud API key — is stored in `prefs.json` only as
//! a DPAPI blob (`CryptProtectData`, current-user scope), base64-encoded.
//! A copied `prefs.json` is useless on another account or PC.
//!
//! Non-Windows hosts (the Linux test/CI build) have no DPAPI: there the
//! blob is the plain text behind a `plain:` marker so the round trip is
//! testable. The shipped app is Windows-only.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// Extra entropy bound into every blob: another app calling
/// `CryptUnprotectData` on our blob without it fails.
#[cfg_attr(not(windows), allow(dead_code))]
const ENTROPY: &[u8] = b"Murmur/secret/v1";

/// Encrypt `plain` for the current user. `""` stays `""` (= no secret).
pub fn protect(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    imp::protect(plain.as_bytes()).map(|b| B64.encode(b))
}

/// Inverse of [`protect`]. A blob from another user/PC fails: the caller
/// treats that as "no secret" and asks the user to enter it again.
pub fn unprotect(blob: &str) -> Result<String, String> {
    if blob.is_empty() {
        return Ok(String::new());
    }
    let raw = B64.decode(blob.trim()).map_err(|e| e.to_string())?;
    let plain = imp::unprotect(&raw)?;
    String::from_utf8(plain).map_err(|e| e.to_string())
}

#[cfg(windows)]
mod imp {
    use super::ENTROPY;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(b: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: b.len() as u32,
            pbData: b.as_ptr() as *mut u8,
        }
    }

    /// Copy DPAPI's LocalAlloc'd output and free it.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as _));
        v
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                windows::core::w!("Murmur"),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|e| format!("DPAPI protect: {e}"))?;
            Ok(take(out))
        }
    }

    pub fn unprotect(raw: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(raw);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
            .map_err(|e| format!("DPAPI unprotect: {e}"))?;
            Ok(take(out))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    const MARK: &[u8] = b"plain:";

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        Ok([MARK, plain].concat())
    }

    pub fn unprotect(raw: &[u8]) -> Result<Vec<u8>, String> {
        raw.strip_prefix(MARK)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "not a secret blob from this host".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_empty() {
        let b = protect("0123456789abcdef-token").unwrap();
        assert!(!b.contains("0123456789abcdef-token"), "stored encoded");
        assert_eq!(unprotect(&b).unwrap(), "0123456789abcdef-token");
        assert_eq!(protect("").unwrap(), "");
        assert_eq!(unprotect("").unwrap(), "");
        assert!(unprotect("not base64!").is_err());
        assert!(unprotect(&B64.encode(b"garbage")).is_err());
    }
}
