//! Shared dictionary / snippet export format: `[{word, replacement, isSnippet}]`.
//!
//! Matches the Android Room `dictionary_words` table and the Windows app's
//! dictionary. An entry with no replacement is a pure vocabulary word (ASR
//! bias only); with a replacement it is a deterministic rewrite; with
//! `isSnippet` the replacement is an expansion body.

use serde::{Deserialize, Serialize};

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shared_json_format() {
        let json = r#"[
            {"word": "zephyr", "replacement": "Zephyr", "isSnippet": false},
            {"word": "my address", "replacement": "1 Main St", "isSnippet": true},
            {"word": "Kubernetes"}
        ]"#;
        let d = Dictionary::from_json(json).unwrap();
        assert_eq!(d.entries.len(), 3);
        assert_eq!(d.entries[0].replacement.as_deref(), Some("Zephyr"));
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
                DictEntry::replacement("zephyr", "Zephyr"),
                DictEntry::snippet("my address", "1 Main Street, Springfield"),
            ],
        };
        assert_eq!(d.vocabulary(), vec!["Kubernetes", "Zephyr"]);
    }
}
