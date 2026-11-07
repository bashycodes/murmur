//! Shared dictionary / snippet export format: `[{word, replacement, isSnippet}]`.
//!
//! Matches Android Room `dictionary_words`
//! (spec §9). An entry with no replacement is a pure vocabulary word (ASR
//! bias only); with a replacement it is a deterministic rewrite; with
//! `isSnippet` the replacement is an expansion body.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// D64: public product and tech names the AI cleanup knows how to spell
/// without the user adding them (`SessionConfig::builtin_terms`, on by
/// default). Prompt-only: never an ASR hotword and never a deterministic
/// rewrite. Kept small and free of names that are also ordinary words
/// ("Slack", "Zoom", "Notion"), and of near-twins ("TikTok" next to
/// "TickTick").
pub const BUILTIN_TERMS: &[&str] = &[
    "TickTick",
    "Wispr Flow",
    "WhatsApp",
    "ChatGPT",
    "OpenAI",
    "GitHub",
    "YouTube",
    "LinkedIn",
    "iPhone",
    "PowerPoint",
    "JavaScript",
    "TypeScript",
    "Wi-Fi",
    "Gmail",
    "PayPal",
    "WordPress",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictEntry {
    pub word: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
    #[serde(default)]
    pub is_snippet: bool,
}

impl DictEntry {
    pub fn word(w: &str) -> Self {
        Self {
            word: w.into(),
            replacement: None,
            is_snippet: false,
        }
    }
    pub fn replacement(w: &str, r: &str) -> Self {
        Self {
            word: w.into(),
            replacement: Some(r.into()),
            is_snippet: false,
        }
    }
    pub fn snippet(w: &str, body: &str) -> Self {
        Self {
            word: w.into(),
            replacement: Some(body.into()),
            is_snippet: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dictionary {
    pub entries: Vec<DictEntry>,
}

impl Dictionary {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("dictionary serializes")
    }

    /// Words that should bias the ASR: plain words and the *target* of
    /// replacements (the spelling the user wants). Snippet bodies are not
    /// vocabulary.
    pub fn vocabulary(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| !e.is_snippet)
            .map(|e| e.replacement.as_deref().unwrap_or(&e.word))
            .collect()
    }

    /// D64: the dictionary the cleanup prompt picks its terms from: this
    /// one, followed by [`BUILTIN_TERMS`] as plain words when `builtin`
    /// (a built-in name the vocabulary already has, ignoring case, is not
    /// repeated). Unchanged (borrowed) when `builtin` is off.
    pub fn with_builtin_terms(&self, builtin: bool) -> Cow<'_, Dictionary> {
        if !builtin {
            return Cow::Borrowed(self);
        }
        let have: Vec<String> = self.vocabulary().iter().map(|w| w.to_lowercase()).collect();
        let mut d = self.clone();
        d.entries.extend(
            BUILTIN_TERMS
                .iter()
                .filter(|t| !have.contains(&t.to_lowercase()))
                .map(|t| DictEntry::word(t)),
        );
        Cow::Owned(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shared_json_format() {
        let json = r#"[
            {"word": "wispr", "replacement": "Wispr", "isSnippet": false},
            {"word": "my address", "replacement": "1 Main St", "isSnippet": true},
            {"word": "Kubernetes"}
        ]"#;
        let d = Dictionary::from_json(json).unwrap();
        assert_eq!(d.entries.len(), 3);
        assert_eq!(d.entries[0].replacement.as_deref(), Some("Wispr"));
        assert!(d.entries[1].is_snippet);
        assert!(!d.entries[2].is_snippet);
        assert_eq!(d.entries[2].replacement, None);
    }

    #[test]
    fn serializes_camel_case_is_snippet() {
        let d = Dictionary {
            entries: vec![DictEntry::snippet("sig", "Best, Me")],
        };
        let s = d.to_json();
        assert!(s.contains("\"isSnippet\":true"), "{s}");
    }

    #[test]
    fn vocabulary_lists_words_and_replacements_without_snippet_bodies() {
        let d = Dictionary {
            entries: vec![
                DictEntry::word("Kubernetes"),
                DictEntry::replacement("wispr", "Wispr"),
                DictEntry::snippet("my address", "1 Main Street, Springfield"),
            ],
        };
        assert_eq!(d.vocabulary(), vec!["Kubernetes", "Wispr"]);
    }

    #[test]
    fn builtin_terms_follow_the_users_entries_without_repeats() {
        let d = Dictionary {
            entries: vec![
                DictEntry::word("Quillmate"),
                DictEntry::replacement("git hub", "github"),
            ],
        };
        let off = d.with_builtin_terms(false);
        assert_eq!(off.vocabulary(), vec!["Quillmate", "github"]);
        let on = d.with_builtin_terms(true);
        let v = on.vocabulary();
        assert_eq!(&v[..2], &["Quillmate", "github"]);
        assert_eq!(v.len(), 2 + BUILTIN_TERMS.len() - 1, "GitHub not repeated");
        assert!(v.contains(&"TickTick") && !v.contains(&"GitHub"));
    }
}
