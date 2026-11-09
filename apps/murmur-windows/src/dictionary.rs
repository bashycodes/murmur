//! Dictionary / snippets import and export.
//!
//! The exchange format is the one both shells share:
//! a JSON array of `{word, replacement, isSnippet}`. Import is tolerant
//! (`is_snippet` and a bare string are accepted; unknown fields are
//! ignored) but validated: blank words are dropped, a snippet needs a
//! body, and duplicates collapse case-insensitively (later wins, order of
//! first appearance kept, since the core's dictionary regex honours order).

use crate::core_api::DictEntry;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(untagged)]
enum AnyEntry {
    Word(String),
    Full {
        word: String,
        #[serde(default)]
        replacement: Option<String>,
        #[serde(default, rename = "isSnippet", alias = "is_snippet")]
        is_snippet: bool,
    },
}

/// Import summary for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct MergeReport {
    pub entries: Vec<DictEntry>,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

fn key(w: &str) -> String {
    w.trim().to_lowercase()
}

fn clean(word: String, replacement: Option<String>, is_snippet: bool) -> Option<DictEntry> {
    let word = word.trim().to_string();
    if word.is_empty() {
        return None;
    }
    let replacement = replacement
        .map(|r| if is_snippet { r } else { r.trim().to_string() })
        .filter(|r| !r.trim().is_empty());
    if is_snippet && replacement.is_none() {
        return None;
    }
    Some(DictEntry {
        word,
        replacement,
        is_snippet,
    })
}

/// Parse exchange JSON. Returns the valid entries and how many were dropped.
pub fn parse(json: &str) -> Result<(Vec<DictEntry>, usize), String> {
    let raw: Vec<AnyEntry> =
        serde_json::from_str(json.trim_start_matches('\u{feff}')).map_err(|e| {
            format!("not a dictionary export ([{{word, replacement, isSnippet}}]): {e}")
        })?;
    let total = raw.len();
    let mut out: Vec<DictEntry> = Vec::new();
    for e in raw {
        let e = match e {
            AnyEntry::Word(w) => clean(w, None, false),
            AnyEntry::Full {
                word,
                replacement,
                is_snippet,
            } => clean(word, replacement, is_snippet),
        };
        let Some(e) = e else { continue };
        match out.iter_mut().find(|o| key(&o.word) == key(&e.word)) {
            Some(o) => *o = e,
            None => out.push(e),
        }
    }
    let kept = out.len();
    Ok((out, total - kept))
}

/// Merge `incoming` into `existing`: same word (case-insensitive) replaces
/// in place, new words are appended.
pub fn merge(existing: &[DictEntry], incoming: Vec<DictEntry>) -> MergeReport {
    let mut entries = existing.to_vec();
    let (mut added, mut updated) = (0, 0);
    for e in incoming {
        match entries.iter_mut().find(|o| key(&o.word) == key(&e.word)) {
            Some(o) if *o == e => {}
            Some(o) => {
                *o = e;
                updated += 1;
            }
            None => {
                entries.push(e);
                added += 1;
            }
        }
    }
    MergeReport {
        entries,
        added,
        updated,
        skipped: 0,
    }
}

/// Parse + merge (or replace when `replace`), for the import button.
pub fn import(existing: &[DictEntry], json: &str, replace: bool) -> Result<MergeReport, String> {
    let (incoming, skipped) = parse(json)?;
    let mut r = if replace {
        MergeReport {
            added: incoming.len(),
            entries: incoming,
            updated: 0,
            skipped: 0,
        }
    } else {
        merge(existing, incoming)
    };
    r.skipped = skipped;
    Ok(r)
}

/// Pretty JSON in the shared exchange shape (Android imports it as-is).
pub fn export(entries: &[DictEntry]) -> String {
    serde_json::to_string_pretty(entries).expect("entries serialise")
}

/// Validate an edited list from the UI (same rules as import).
pub fn normalize(entries: Vec<DictEntry>) -> Vec<DictEntry> {
    let json = serde_json::to_string(&entries).expect("entries serialise");
    parse(&json).map(|(v, _)| v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(w: &str, r: Option<&str>, s: bool) -> DictEntry {
        DictEntry {
            word: w.into(),
            replacement: r.map(Into::into),
            is_snippet: s,
        }
    }

    #[test]
    fn export_is_the_shared_shape_and_round_trips() {
        let d = vec![
            e("Kubernetes", None, false),
            e("addr", Some("1 Main St\nTown"), true),
        ];
        let j = export(&d);
        assert!(j.contains(r#""isSnippet": true"#), "{j}");
        assert!(j.contains(r#""replacement": null"#), "{j}");
        assert_eq!(parse(&j).unwrap(), (d, 0));
    }

    #[test]
    fn parse_is_tolerant_but_validates() {
        let (v, skipped) = parse(
            r#"["Korra", {"word":"gh","replacement":"GitHub"},
                {"word":"sig","is_snippet":true,"replacement":"Best,\n Me","extra":1},
                {"word":"  "}, {"word":"empty","isSnippet":true},
                {"word":"GH","replacement":"GitHub Inc"}]"#,
        )
        .unwrap();
        assert_eq!(
            v,
            vec![
                e("Korra", None, false),
                e("GH", Some("GitHub Inc"), false),
                e("sig", Some("Best,\n Me"), true),
            ],
            "duplicate gh/GH: later wins in the first slot; snippet body kept verbatim"
        );
        assert_eq!(skipped, 3);
        assert!(parse("{\"word\":\"x\"}").unwrap_err().contains("isSnippet"));
        assert!(
            parse("\u{feff}[]").unwrap().0.is_empty(),
            "BOM from Notepad"
        );
    }

    #[test]
    fn merge_updates_in_place_and_appends() {
        let have = vec![e("gh", Some("GitHub"), false), e("Korra", None, false)];
        let r = import(
            &have,
            r#"[{"word":"GH","replacement":"GitHub!"},{"word":"korra"},{"word":"new"}]"#,
            false,
        )
        .unwrap();
        assert_eq!(r.added, 1);
        assert_eq!(r.updated, 2, "case change of Korra counts as an update");
        assert_eq!(r.entries[0], e("GH", Some("GitHub!"), false));
        assert_eq!(r.entries[2].word, "new");
        let r = import(&have, r#"[{"word":"only"}]"#, true).unwrap();
        assert_eq!(r.entries, vec![e("only", None, false)]);
    }

    #[test]
    fn normalize_cleans_ui_rows() {
        let v = normalize(vec![
            e(" a ", Some(" b "), false),
            e("", None, false),
            e("s", None, true),
        ]);
        assert_eq!(v, vec![e("a", Some("b"), false)]);
    }
}
