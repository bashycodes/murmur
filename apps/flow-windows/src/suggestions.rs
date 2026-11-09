//! Dictionary suggestions from learn-from-edits, waiting for the user.
//!
//! Learned words are never added automatically: a candidate lands here, the tray and the Dictionary tab show a
//! count, and only an explicit accept turns it into a dictionary entry.
//! Dismissed words are remembered so they are not suggested again.
//!
//! Stored in `%APPDATA%\FlowClone\suggestions.json` (words only, no
//! transcripts), written atomically like `prefs.json`.

use crate::core_api::{DictEntry, LearnCandidate};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "suggestions.json";
pub const MAX_PENDING: usize = 50;
pub const MAX_DISMISSED: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// Proposed dictionary word (what the user typed).
    pub word: String,
    /// What the dictation produced instead.
    pub heard: String,
    /// Process the correction was seen in.
    pub app: String,
    /// Last seen, Unix epoch ms.
    pub ts: i64,
    /// Times the same correction was seen.
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Suggestions {
    pub pending: Vec<Suggestion>,
    /// Lowercased words the user said no to.
    pub dismissed: Vec<String>,
}

fn key(w: &str) -> String {
    w.trim().to_lowercase()
}

fn in_dictionary(word: &str, dict: &[DictEntry]) -> bool {
    let k = key(word);
    dict.iter()
        .any(|e| key(&e.word) == k || e.replacement.as_deref().is_some_and(|r| key(r) == k))
}

impl Suggestions {
    /// Add candidates; returns how many are new. Words already in the
    /// dictionary or dismissed are skipped; a repeat bumps `count`.
    pub fn add(
        &mut self,
        cands: &[LearnCandidate],
        app: &str,
        ts: i64,
        dict: &[DictEntry],
    ) -> usize {
        let mut added = 0;
        for c in cands {
            let word = c.word.trim();
            if word.is_empty() || in_dictionary(word, dict) || self.dismissed.contains(&key(word)) {
                continue;
            }
            if let Some(s) = self.pending.iter_mut().find(|s| key(&s.word) == key(word)) {
                s.count += 1;
                s.ts = ts;
                s.heard = c.heard.clone();
                continue;
            }
            self.pending.insert(
                0,
                Suggestion {
                    word: word.to_string(),
                    heard: c.heard.clone(),
                    app: app.to_string(),
                    ts,
                    count: 1,
                },
            );
            added += 1;
        }
        self.pending.truncate(MAX_PENDING);
        added
    }

    /// Remove `word` from pending and return the dictionary entry to add:
    /// a vocabulary word (biases ASR and the LLM), or `heard → word` when
    /// `as_replacement`.
    pub fn accept(&mut self, word: &str, as_replacement: bool) -> Option<DictEntry> {
        let i = self
            .pending
            .iter()
            .position(|s| key(&s.word) == key(word))?;
        let s = self.pending.remove(i);
        Some(if as_replacement && !s.heard.trim().is_empty() {
            DictEntry {
                word: s.heard,
                replacement: Some(s.word),
                is_snippet: false,
            }
        } else {
            DictEntry {
                word: s.word,
                replacement: None,
                is_snippet: false,
            }
        })
    }

    pub fn dismiss(&mut self, word: &str) -> bool {
        let before = self.pending.len();
        self.pending.retain(|s| key(&s.word) != key(word));
        let k = key(word);
        if !k.is_empty() && !self.dismissed.contains(&k) {
            self.dismissed.push(k);
            if self.dismissed.len() > MAX_DISMISSED {
                self.dismissed.remove(0);
            }
        }
        self.pending.len() != before
    }

    /// Drop pending words that have since been added by hand.
    pub fn prune_known(&mut self, dict: &[DictEntry]) {
        self.pending.retain(|s| !in_dictionary(&s.word, dict));
    }
}

/// [`Suggestions`] bound to a file (`None` = in memory only, for tests).
#[derive(Debug, Default)]
pub struct SuggestionStore {
    path: Option<PathBuf>,
    pub data: Suggestions,
}

pub fn default_path(appdata: Option<&str>) -> PathBuf {
    appdata
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(crate::prefs::APP_DIR)
        .join(FILE_NAME)
}

impl SuggestionStore {
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Load (a missing or unreadable file = empty).
    pub fn open(path: &Path) -> Self {
        let data = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        SuggestionStore {
            path: Some(path.to_path_buf()),
            data,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(&self.data).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(word: &str, heard: &str) -> LearnCandidate {
        LearnCandidate {
            word: word.into(),
            heard: heard.into(),
            replacement: None,
        }
    }

    fn dict(words: &[&str]) -> Vec<DictEntry> {
        words
            .iter()
            .map(|w| DictEntry {
                word: (*w).into(),
                replacement: None,
                is_snippet: false,
            })
            .collect()
    }

    #[test]
    fn add_skips_known_and_dismissed_and_counts_repeats() {
        let mut s = Suggestions::default();
        let n = s.add(
            &[
                cand("Korra", "cora"),
                cand("Kubernetes", "cooper netties"),
            ],
            "slack.exe",
            1,
            &dict(&["kubernetes"]),
        );
        assert_eq!(n, 1);
        assert_eq!(s.pending.len(), 1);
        assert_eq!(s.pending[0].word, "Korra");
        assert_eq!(s.add(&[cand("korra", "cora")], "x", 2, &[]), 0);
        assert_eq!(s.pending[0].count, 2);
        assert!(s.dismiss("KORRA"));
        assert!(s.pending.is_empty());
        assert_eq!(
            s.add(&[cand("Korra", "cora")], "x", 3, &[]),
            0,
            "dismissed"
        );
    }

    #[test]
    fn accept_returns_a_word_or_a_replacement() {
        let mut s = Suggestions::default();
        s.add(
            &[cand("Korra", "cora"), cand("GitHub", "get hub")],
            "a",
            1,
            &[],
        );
        let e = s.accept("korra", false).unwrap();
        assert_eq!((e.word.as_str(), e.replacement), ("Korra", None));
        let e = s.accept("GitHub", true).unwrap();
        assert_eq!(e.word, "get hub");
        assert_eq!(e.replacement.as_deref(), Some("GitHub"));
        assert!(s.accept("GitHub", false).is_none());
    }

    #[test]
    fn pending_is_capped_newest_first_and_pruned() {
        let mut s = Suggestions::default();
        for i in 0..(MAX_PENDING + 5) {
            s.add(&[cand(&format!("w{i}"), "x")], "a", i as i64, &[]);
        }
        assert_eq!(s.pending.len(), MAX_PENDING);
        assert_eq!(s.pending[0].word, format!("w{}", MAX_PENDING + 4));
        s.prune_known(&dict(&[&s.pending[0].word.clone()]));
        assert_eq!(s.pending.len(), MAX_PENDING - 1);
    }

    #[test]
    fn store_round_trips_through_a_file() {
        let d = std::env::temp_dir().join(format!("flow-sugg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join(FILE_NAME);
        let mut st = SuggestionStore::open(&p);
        assert!(st.data.pending.is_empty());
        st.data.add(&[cand("Korra", "cora")], "a", 1, &[]);
        st.data.dismiss("nope");
        st.save().unwrap();
        let st2 = SuggestionStore::open(&p);
        assert_eq!(st2.data, st.data);
        let _ = std::fs::remove_dir_all(&d);
    }
}
