//! Team dictionary API v1: one shared word list per murmur-server. Every
//! client on the server downloads it and merges it under the user's own
//! dictionary (`murmur_core::dictionary::with_team`: a personal entry wins
//! on the same word), so team words bias the speech model, reach the AI
//! cleanup and rewrite like personal ones. No snippets.
//!
//! Same transport, bearer key, errors ([`crate::admin::AdminError`]) and
//! D112 throttle as the admin API.
//!
//! | Method + path | Request | 2xx response |
//! |---|---|---|
//! | `GET` [`PATH_DICTIONARY`] (any key) | `If-None-Match` optional | 200 [`TeamDictionary`] + `ETag`, or 304 |
//! | `POST` [`PATH_ADMIN_DICTIONARY`] (admin key) | [`TeamEntry`] | 200 [`TeamDictionary`] + `ETag` (added, or the same word replaced) |
//! | `DELETE` [`PATH_ADMIN_DICTIONARY`] (admin key) | [`RemoveTeamEntry`] | 200 [`TeamDictionary`] + `ETag` |
//!
//! A bad entry is a 400 `bad_request` whose message says what is wrong
//! ([`check_entry`]); a full list is a 400 `too_many`; removing a word
//! that is not there is a 404 `not_found`. A server with `auth = "none"`
//! answers 404 `auth_off`: it has no team dictionary. A server whose list
//! file does not parse refuses every edit with 500 [`CODE_STORE_CORRUPT`].

use serde::{Deserialize, Serialize};

/// The team list, for any active key (`GET`, conditional with `ETag`).
pub const PATH_DICTIONARY: &str = "/v1/dictionary";
/// Add or replace (`POST`) and remove (`DELETE`) one entry; admin key only.
pub const PATH_ADMIN_DICTIONARY: &str = "/v1/admin/dictionary";
/// Most entries a team dictionary holds.
pub const MAX_ENTRIES: usize = 500;
/// Longest word or replacement, in characters.
pub const MAX_CHARS: usize = 64;
/// Most whitespace-separated words in a word or replacement.
pub const MAX_TOKENS: usize = 6;
/// Characters refused as markup (HTML, templates, Markdown code).
pub const MARKUP_CHARS: &[char] = &['<', '>', '{', '}', '[', ']', '`', '\\'];
/// Largest request body the edit routes accept (one entry).
pub const MAX_EDIT_BODY_BYTES: usize = 4 * 1024;

/// [`crate::admin::AdminError::code`] for a full list (400).
pub const CODE_TOO_MANY: &str = "too_many";
/// [`crate::admin::AdminError::code`] for an edit while the server's list
/// file can't be read (500): the server refuses to overwrite it.
pub const CODE_STORE_CORRUPT: &str = "store_corrupt";
/// Longest `ETag` a client accepts, sends or caches.
pub const MAX_ETAG_LEN: usize = 128;

/// An `ETag` safe to cache and send back in `If-None-Match`: 1 to
/// [`MAX_ETAG_LEN`] bytes of visible ASCII or spaces (no CR, LF, NUL or
/// other control characters, so it can never split a header).
pub fn valid_etag(t: &str) -> bool {
    !t.is_empty() && t.len() <= MAX_ETAG_LEN && t.bytes().all(|b| (0x20..0x7f).contains(&b))
}

/// One team entry: a word to recognise, or with `replacement` a word to
/// rewrite. Same JSON as a personal dictionary entry without `isSnippet`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamEntry {
    pub word: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
}

impl TeamEntry {
    /// The entry as the core's dictionary entry (never a snippet).
    pub fn to_dict_entry(&self) -> murmur_core::dictionary::DictEntry {
        murmur_core::dictionary::DictEntry {
            word: self.word.clone(),
            replacement: self.replacement.clone(),
            is_snippet: false,
        }
    }
}

/// `GET` [`PATH_DICTIONARY`] answer, and every edit's answer: the whole list,
/// in the order the admin added it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamDictionary {
    pub entries: Vec<TeamEntry>,
}

impl TeamDictionary {
    /// The entries as core dictionary entries, for
    /// `murmur_core::dictionary::with_team`.
    pub fn dict_entries(&self) -> Vec<murmur_core::dictionary::DictEntry> {
        self.entries.iter().map(TeamEntry::to_dict_entry).collect()
    }
}

/// `DELETE` [`PATH_ADMIN_DICTIONARY`] body: the word to remove (case and
/// spacing ignored).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoveTeamEntry {
    pub word: String,
}

/// Check one text (a word or a replacement) is term-like: 1 to
/// [`MAX_CHARS`] characters after trimming, at most [`MAX_TOKENS`] words,
/// no control characters (line breaks and tabs included) and no
/// [`MARKUP_CHARS`]. Returns the trimmed text.
fn check_text(what: &str, s: &str) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(format!("{what} is empty"));
    }
    let n = t.chars().count();
    if n > MAX_CHARS {
        return Err(format!(
            "{what} \"{}…\" is {n} characters; at most {MAX_CHARS}",
            t.chars().take(20).collect::<String>()
        ));
    }
    if t.chars().any(char::is_control) {
        return Err(format!(
            "{what} \"{t}\" has a line break, tab or control character"
        ));
    }
    if let Some(c) = t.chars().find(|c| MARKUP_CHARS.contains(c)) {
        return Err(format!(
            "{what} \"{t}\" has the markup character '{c}'; a team word is plain text"
        ));
    }
    let words = t.split_whitespace().count();
    if words > MAX_TOKENS {
        return Err(format!(
            "{what} \"{t}\" is {words} words; at most {MAX_TOKENS}"
        ));
    }
    Ok(t.to_string())
}

/// The entry an app's two boxes make: `replacement` blank means none.
/// Checked and trimmed like the server does ([`check_entry`]), so this is
/// exactly what an add sends.
pub fn entry_from(word: &str, replacement: &str) -> Result<TeamEntry, String> {
    check_entry(&TeamEntry {
        word: word.into(),
        replacement: Some(replacement.to_string()),
    })
}

/// The entry, trimmed, or why the server refuses it (the message the
/// apps show). An empty replacement is no replacement.
pub fn check_entry(e: &TeamEntry) -> Result<TeamEntry, String> {
    let word = check_text("the word", &e.word)?;
    let replacement = match e.replacement.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(r) => Some(check_text("the replacement", r)?),
    };
    Ok(TeamEntry { word, replacement })
}

/// Keep the entries that pass [`check_entry`] (trimmed), the first of any
/// repeated word (case and spacing ignored), and at most [`MAX_ENTRIES`].
/// Clients run it on what they download or cache, so a bad file never
/// reaches a dictation.
pub fn sanitize(entries: &[TeamEntry]) -> Vec<TeamEntry> {
    let mut seen = std::collections::HashSet::new();
    entries
        .iter()
        .filter_map(|e| check_entry(e).ok())
        .filter(|e| seen.insert(murmur_core::postprocess::word_key(&e.word)))
        .take(MAX_ENTRIES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(word: &str, replacement: Option<&str>) -> TeamEntry {
        TeamEntry {
            word: word.into(),
            replacement: replacement.map(Into::into),
        }
    }

    #[test]
    fn etags_with_control_characters_or_too_long_are_refused() {
        assert!(valid_etag("\"0123456789abcdef\""));
        assert!(valid_etag("W/\"x\""));
        for bad in [
            "",
            "\"a\"\r\nX-Evil: 1",
            "\"a\nb\"",
            "\"a\0\"",
            "\"a\tb\"",
            "\"é\"",
        ] {
            assert!(!valid_etag(bad), "{bad:?}");
        }
        assert!(valid_etag(&"a".repeat(MAX_ETAG_LEN)));
        assert!(!valid_etag(&"a".repeat(MAX_ETAG_LEN + 1)));
    }

    #[test]
    fn entry_from_trims_and_drops_a_blank_replacement() {
        assert_eq!(entry_from(" Quillon ", "  "), Ok(e("Quillon", None)));
        assert_eq!(
            entry_from(" acme corp ", " AcmeCorp "),
            Ok(e("acme corp", Some("AcmeCorp")))
        );
        assert!(entry_from("<b>", "").is_err());
    }

    /// The length vectors shared with Kotlin's `TeamDictionary.wordError`:
    /// characters are code points, astral ones included.
    #[test]
    fn word_length_vectors() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../murmur-core/fixtures/team-dictionary/words.json"
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for c in v["cases"].as_array().unwrap() {
            let ok = check_entry(&e(c["word"].as_str().unwrap(), None)).is_ok();
            assert_eq!(ok, c["ok"].as_bool().unwrap(), "{}", c["name"]);
        }
    }

    #[test]
    fn term_like_entries_pass_trimmed() {
        assert_eq!(
            check_entry(&e("  Quillon ", Some(" Quillon Labs "))),
            Ok(e("Quillon", Some("Quillon Labs")))
        );
        assert_eq!(check_entry(&e("C#", Some(""))), Ok(e("C#", None)));
        assert!(check_entry(&e("one two three four five six", None)).is_ok());
        assert!(check_entry(&e(&"x".repeat(MAX_CHARS), None)).is_ok());
        // Non-Latin scripts count characters, not bytes.
        assert!(check_entry(&e(&"ש".repeat(MAX_CHARS), None)).is_ok());
    }

    #[test]
    fn non_terms_are_refused_with_a_reason() {
        let bad = |w: &str, r: Option<&str>, says: &str| {
            let m = check_entry(&e(w, r)).unwrap_err();
            assert!(m.contains(says), "{w:?} {r:?}: {m}");
        };
        bad("", None, "empty");
        bad("   ", None, "empty");
        bad(&"x".repeat(MAX_CHARS + 1), None, "at most 64");
        bad("one two three four five six seven", None, "7 words");
        bad("a\nb", None, "line break");
        bad("tab\there", None, "line break");
        bad("<b>bold</b>", None, "markup");
        bad("{{name}}", None, "markup");
        bad("`code`", None, "markup");
        bad("ok", Some("[link](x)"), "the replacement");
    }

    #[test]
    fn sanitize_drops_bad_and_repeated_and_caps() {
        let got = sanitize(&[
            e("Zephyr", None),
            e("<i>x</i>", None),
            e(" zephyr ", Some("ZephyrCo")),
            e("Acme", Some("AcmeCorp")),
        ]);
        assert_eq!(got, [e("Zephyr", None), e("Acme", Some("AcmeCorp"))]);
        let many: Vec<TeamEntry> = (0..MAX_ENTRIES + 5)
            .map(|i| e(&format!("w{i}"), None))
            .collect();
        assert_eq!(sanitize(&many).len(), MAX_ENTRIES);
    }

    #[test]
    fn wire_shapes_are_stable() {
        let d = TeamDictionary {
            entries: vec![e("Zephyr", None), e("acme", Some("AcmeCorp"))],
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({"entries":[{"word":"Zephyr"},{"word":"acme","replacement":"AcmeCorp"}]})
        );
        // Unknown fields (a personal entry's isSnippet) are ignored.
        let t: TeamEntry =
            serde_json::from_str(r#"{"word":"x","isSnippet":true,"future":1}"#).unwrap();
        assert_eq!(t, e("x", None));
        assert!(!d.dict_entries()[1].is_snippet);
    }
}
