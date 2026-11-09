//! Runtime environment variables after the Murmur rename (D83).
//!
//! Every variable is read as `MURMUR_<NAME>`. For one transition period the
//! name the product had before the rename (`FLOW_<NAME>`) is still read when
//! the new one is unset, and the first such read of each name prints a
//! notice to stderr (the server's journal, the app's console), so a deploy
//! that still sets the old name keeps working and says what to change.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::sync::Mutex;

/// Prefix of every runtime variable.
pub const PREFIX: &str = "MURMUR_";
/// The prefix before D83, still read for one transition period.
pub const OLD_PREFIX: &str = "FLOW_";

/// Old names already announced (one notice per name per process).
static NOTICED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// `std::env::var_os` for `MURMUR_<NAME>`, falling back to `FLOW_<NAME>`
/// with a one-time notice. `name` is the full new name (`"MURMUR_LLM_LIB"`).
pub fn var_os(name: &str) -> Option<OsString> {
    let (value, old) = resolve(name, |k| std::env::var_os(k))?;
    if let Some(old) = old {
        notice_once(&old, name);
    }
    Some(value)
}

/// [`var_os`] with `std::env::var`'s result type, so it drops in for it.
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    var_os(name)
        .ok_or(std::env::VarError::NotPresent)?
        .into_string()
        .map_err(std::env::VarError::NotUnicode)
}

/// The value of `name`, else of its old spelling (also returned, for the
/// notice). Pure apart from `get`, so the precedence is testable.
pub fn resolve(
    name: &str,
    get: impl Fn(&str) -> Option<OsString>,
) -> Option<(OsString, Option<String>)> {
    if let Some(v) = get(name) {
        return Some((v, None));
    }
    let old = old_name(name)?;
    get(&old).map(|v| (v, Some(old)))
}

/// `MURMUR_X` → `FLOW_X`; `None` for a name without the prefix.
pub fn old_name(name: &str) -> Option<String> {
    name.strip_prefix(PREFIX)
        .map(|rest| format!("{OLD_PREFIX}{rest}"))
}

fn notice_once(old: &str, new: &str) {
    let first = NOTICED
        .lock()
        .map(|mut s| s.insert(old.to_string()))
        .unwrap_or(false);
    if first {
        eprintln!(
            "notice: {old} is the old name of {new}; it is still read for now, rename it (D83)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn the_new_name_wins_over_the_old_one() {
        let got = resolve(
            "MURMUR_LLM_LIB",
            env(&[("MURMUR_LLM_LIB", "new"), ("FLOW_LLM_LIB", "old")]),
        );
        assert_eq!(got, Some((OsString::from("new"), None)));
    }

    #[test]
    fn the_old_name_is_read_when_the_new_one_is_unset() {
        let got = resolve("MURMUR_LLM_LIB", env(&[("FLOW_LLM_LIB", "old")]));
        assert_eq!(
            got,
            Some((OsString::from("old"), Some("FLOW_LLM_LIB".to_string())))
        );
    }

    #[test]
    fn unset_or_unprefixed_names_have_no_fallback() {
        assert_eq!(resolve("MURMUR_LLM_LIB", env(&[])), None);
        assert_eq!(resolve("HOME", env(&[("FLOW_HOME", "x")])), None);
        assert_eq!(
            old_name("MURMUR_WHISPER_DEVICE").as_deref(),
            Some("FLOW_WHISPER_DEVICE")
        );
    }
}
