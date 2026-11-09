//! Dictionary replacements and whole-utterance snippets (spec §4).

use std::collections::{HashMap, HashSet};

use super::chars::is_word_char;
use crate::dictionary::DictEntry;

/// One element of a compiled key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Piece {
    /// A character, case-folded.
    Char(char),
    /// One or more whitespace characters.
    Space,
}

#[derive(Debug, Clone)]
struct Rule {
    pieces: Vec<Piece>,
    /// The key starts with a word character, so the text before the match
    /// must not be one.
    word_start: bool,
    /// The key ends with a word character, so the text after the match
    /// must not be one.
    word_end: bool,
    replacement: String,
}

impl Rule {
    /// The end (char index) of this rule's match at `start`, if any.
    fn match_end(&self, chars: &[char], start: usize) -> Option<usize> {
        let mut j = start;
        for p in &self.pieces {
            match p {
                Piece::Char(k) => {
                    if j < chars.len() && fold(chars[j]) == *k {
                        j += 1;
                    } else {
                        return None;
                    }
                }
                Piece::Space => {
                    let from = j;
                    while j < chars.len() && chars[j].is_whitespace() {
                        j += 1;
                    }
                    if j == from {
                        return None;
                    }
                }
            }
        }
        Some(j)
    }
}

/// The compiled dictionary: built once per distinct dictionary.
#[derive(Debug, Clone, Default)]
pub struct DictMatcher {
    /// Longest key first, so the first rule that matches at a position wins.
    rules: Vec<Rule>,
    /// Rule indices (into `rules`, in order) by the key's first folded char.
    by_first: HashMap<char, Vec<usize>>,
    /// Normalised snippet key and body, in priority order.
    snippets: Vec<(String, String)>,
}

impl DictMatcher {
    /// Compile `personal` and `team` entries; a personal entry wins over a
    /// team entry with the same key, and an earlier entry over a later one.
    pub fn new(personal: &[DictEntry], team: &[DictEntry]) -> Self {
        let mut rules = Vec::new();
        let mut seen_keys = HashSet::new();
        let mut snippets: Vec<(String, String)> = Vec::new();
        for e in personal.iter().chain(team) {
            let Some(replacement) = &e.replacement else {
                continue;
            };
            let key = e.word.trim();
            if key.is_empty() {
                continue;
            }
            let pieces = compile(key);
            if seen_keys.insert(pieces.clone()) {
                let first = key.chars().next().is_some_and(is_word_char);
                let last = key.chars().last().is_some_and(is_word_char);
                rules.push(Rule {
                    pieces,
                    word_start: first,
                    word_end: last,
                    replacement: replacement.clone(),
                });
            }
            if e.is_snippet {
                let norm = normalise_snippet(key);
                if !norm.is_empty() && !snippets.iter().any(|(k, _)| *k == norm) {
                    snippets.push((norm, replacement.clone()));
                }
            }
        }
        rules.sort_by_key(|r| std::cmp::Reverse(r.pieces.len()));
        let mut by_first: HashMap<char, Vec<usize>> = HashMap::new();
        for (i, r) in rules.iter().enumerate() {
            if let Some(Piece::Char(c)) = r.pieces.first() {
                by_first.entry(*c).or_default().push(i);
            }
        }
        Self {
            rules,
            by_first,
            snippets,
        }
    }

    /// True when no entry rewrites anything.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The snippet body when the whole of `text`, normalised, is a snippet
    /// key.
    pub fn whole_snippet(&self, text: &str) -> Option<String> {
        let norm = normalise_snippet(text);
        if norm.is_empty() {
            return None;
        }
        self.snippets
            .iter()
            .find(|(k, _)| *k == norm)
            .map(|(_, body)| body.clone())
    }

    /// Replace every whole-word occurrence of every key. A text that is, as
    /// a whole, a snippet trigger becomes the snippet body (the trigger's
    /// own punctuation goes with it).
    pub fn find_and_replace(&self, text: &str) -> String {
        if let Some(body) = self.whole_snippet(text) {
            return body;
        }
        if self.rules.is_empty() {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < chars.len() {
            match self.match_at(&chars, i) {
                Some((end, rule)) => {
                    out.push_str(&rule.replacement);
                    i = end;
                }
                None => {
                    out.push(chars[i]);
                    i += 1;
                }
            }
        }
        out
    }

    fn match_at(&self, chars: &[char], i: usize) -> Option<(usize, &Rule)> {
        let candidates = self.by_first.get(&fold(chars[i]))?;
        let after_word = i > 0 && is_word_char(chars[i - 1]);
        candidates.iter().find_map(|&idx| {
            let rule = &self.rules[idx];
            if rule.word_start && after_word {
                return None;
            }
            let end = rule.match_end(chars, i)?;
            if rule.word_end && end < chars.len() && is_word_char(chars[end]) {
                return None;
            }
            if starts_contraction(&chars[end..]) {
                return None;
            }
            Some((end, rule))
        })
    }
}

/// `'t` / `’t` right after a match: the key is the stem of a contraction
/// ("don" in "don't"), not a word of its own.
fn starts_contraction(rest: &[char]) -> bool {
    matches!(rest, [q, t, ..] if matches!(q, '\'' | '’') && matches!(t, 't' | 'T'))
}

fn compile(key: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for c in key.chars() {
        if c.is_whitespace() {
            if pieces.last() != Some(&Piece::Space) {
                pieces.push(Piece::Space);
            }
        } else {
            pieces.push(Piece::Char(fold(c)));
        }
    }
    pieces
}

/// Simple case folding: the single-character lower case, with the few
/// letters whose fold differs from their lower case.
fn fold(c: char) -> char {
    match c {
        'ς' => 'σ',
        'ſ' => 's',
        'ϐ' => 'β',
        'ϑ' => 'θ',
        'ϕ' => 'φ',
        'ϖ' => 'π',
        'ϰ' => 'κ',
        'ϱ' => 'ρ',
        'ϵ' => 'ε',
        'ẛ' => 'ṡ',
        _ => {
            let mut l = c.to_lowercase();
            match (l.next(), l.next()) {
                (Some(x), None) => x,
                _ => c,
            }
        }
    }
}

/// Lowercase, keep only letters, digits and whitespace, collapse the
/// whitespace, trim.
fn normalise_snippet(s: &str) -> String {
    let kept: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(entries: &[DictEntry]) -> DictMatcher {
        DictMatcher::new(entries, &[])
    }

    fn kube() -> DictMatcher {
        m(&[DictEntry::replacement("cubernetes", "Kubernetes")])
    }

    #[test]
    fn vocabulary_words_never_rewrite() {
        let d = m(&[DictEntry::word("Kubernetes")]);
        assert!(d.is_empty());
        assert_eq!(
            d.find_and_replace("deploy the kubernetes job."),
            "deploy the kubernetes job."
        );
    }

    #[test]
    fn replacement_is_case_insensitive_and_inserted_as_stored() {
        let d = kube();
        assert!(!d.is_empty());
        for w in ["cubernetes", "Cubernetes", "CUBERNETES"] {
            assert_eq!(d.find_and_replace(w), "Kubernetes");
        }
        assert_eq!(
            d.find_and_replace("deploy the cubernetes job."),
            "deploy the Kubernetes job."
        );
        assert_eq!(d.find_and_replace("is it cubernetes?"), "is it Kubernetes?");
        assert_eq!(
            d.find_and_replace("cubernetes and cubernetes."),
            "Kubernetes and Kubernetes."
        );
    }

    #[test]
    fn whole_words_only_in_any_script() {
        assert_eq!(kube().find_and_replace("cubernetesx"), "cubernetesx");
        let d = m(&[DictEntry::replacement("café", "Café Lumio")]);
        assert_eq!(d.find_and_replace("écafé"), "écafé");
        assert_eq!(d.find_and_replace("at café now"), "at Café Lumio now");
        // A combining mark right after the key is part of the word.
        assert_eq!(d.find_and_replace("café\u{301}"), "café\u{301}");
    }

    #[test]
    fn keys_with_symbols_match_literally() {
        let d = m(&[
            DictEntry::replacement("c plus plus", "C++"),
            DictEntry::replacement("c++", "C plus plus"),
            DictEntry::replacement("node.js", "Node.js"),
            DictEntry::replacement("42", "forty-two"),
        ]);
        assert_eq!(
            d.find_and_replace("email me at c plus plus."),
            "email me at C++."
        );
        assert_eq!(
            d.find_and_replace("we like c++ a lot."),
            "we like C plus plus a lot."
        );
        assert_eq!(d.find_and_replace("open node.js now."), "open Node.js now.");
        assert_eq!(
            d.find_and_replace("room 42 is free."),
            "room forty-two is free."
        );
        // The word side of "c++" still needs a boundary.
        assert_eq!(d.find_and_replace("abc++ x"), "abc++ x");
    }

    #[test]
    fn contraction_guard_and_possessive() {
        let d = m(&[DictEntry::replacement("don", "Don")]);
        assert_eq!(d.find_and_replace("I don't know"), "I don't know");
        assert_eq!(d.find_and_replace("I don’t know"), "I don’t know");
        assert_eq!(d.find_and_replace("ask don today"), "ask Don today");
        assert_eq!(
            kube().find_and_replace("the cubernetes's job"),
            "the Kubernetes's job"
        );
    }

    #[test]
    fn key_spaces_match_any_whitespace_run() {
        let d = m(&[DictEntry::replacement("zen trix", "Zentrix")]);
        assert_eq!(d.find_and_replace("try zen  trix now"), "try Zentrix now");
        assert_eq!(d.find_and_replace("try zen\ttrix now"), "try Zentrix now");
    }

    #[test]
    fn longest_key_wins_and_no_chaining() {
        let d = m(&[
            DictEntry::replacement("zen trix", "Zentrix"),
            DictEntry::replacement("zen trix hub", "Zentrix Hub"),
        ]);
        assert_eq!(d.find_and_replace("open zen trix hub"), "open Zentrix Hub");
        let d = m(&[
            DictEntry::replacement("mara", "Lumio"),
            DictEntry::replacement("lumio", "Mara"),
        ]);
        assert_eq!(d.find_and_replace("mara and lumio"), "Lumio and Mara");
    }

    #[test]
    fn replacement_is_literal() {
        let d = m(&[DictEntry::replacement("fee", "$1 fee")]);
        assert_eq!(d.find_and_replace("the fee"), "the $1 fee");
    }

    #[test]
    fn key_is_trimmed_and_empty_key_ignored() {
        let d = m(&[
            DictEntry::replacement("  hello  ", "Hi"),
            DictEntry::replacement("   ", "x"),
        ]);
        assert_eq!(d.find_and_replace("hello there."), "Hi there.");
        assert!(m(&[DictEntry::replacement(" ", "x")]).is_empty());
    }

    #[test]
    fn personal_wins_over_team() {
        let d = DictMatcher::new(
            &[DictEntry::replacement("lumo", "Lumio")],
            &[DictEntry::replacement("LUMO", "Lumo Team")],
        );
        assert_eq!(d.find_and_replace("try lumo"), "try Lumio");
        let d = DictMatcher::new(&[], &[DictEntry::replacement("lumo", "Lumo Team")]);
        assert_eq!(d.find_and_replace("try lumo"), "try Lumo Team");
    }

    #[test]
    fn whole_snippet_normalises() {
        let d = m(&[
            DictEntry::snippet("my address", "12 Elm Road"),
            DictEntry::replacement("my office", "Room 4"),
        ]);
        for t in [
            "my address",
            "My address",
            "my address.",
            "  my address  ",
            "my  address",
            "My Address!",
            "MY ADDRESS?",
        ] {
            assert_eq!(d.whole_snippet(t).as_deref(), Some("12 Elm Road"), "{t}");
        }
        assert_eq!(d.whole_snippet("say my address"), None);
        assert_eq!(d.whole_snippet("my addresses"), None);
        assert_eq!(d.whole_snippet("my office"), None);
    }

    #[test]
    fn whole_snippet_priority() {
        let d = DictMatcher::new(
            &[
                DictEntry::snippet("sig", "first"),
                DictEntry::snippet("Sig!", "second"),
            ],
            &[DictEntry::snippet("sig", "team")],
        );
        assert_eq!(d.whole_snippet("sig").as_deref(), Some("first"));
        let d = DictMatcher::new(&[], &[DictEntry::snippet("sig", "team")]);
        assert_eq!(d.whole_snippet("sig").as_deref(), Some("team"));
    }

    #[test]
    fn snippets_also_rewrite_inline() {
        let d = m(&[DictEntry::snippet("my address", "12 Elm Road")]);
        assert_eq!(
            d.find_and_replace("send it to my address please"),
            "send it to 12 Elm Road please"
        );
        assert_eq!(d.find_and_replace("My address."), "12 Elm Road");
    }
}
