//! Edit learning (spec `docs/specs/text-postprocess.md` §11): when the user
//! corrects inserted text, the corrected words become dictionary
//! candidates.

use serde::{Deserialize, Serialize};

use crate::align::{align_words, trim_punct, Op, Step};
use crate::dictionary::Dictionary;

/// A run with more words than this on either side is a rewrite, not
/// vocabulary.
pub const MAX_RUN_WORDS: usize = 4;
/// At most this many candidates per edit.
pub const MAX_CANDIDATES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub original: String,
    pub corrected: String,
    /// The correction reads like a person's or product's name.
    #[serde(default)]
    pub looks_like_name: bool,
}

/// Every word starts with an upper-case letter and has a lower-case
/// letter, and it is not a single word at the start of a sentence.
pub fn looks_like_name(corrected: &str, sentence_initial: bool) -> bool {
    let words: Vec<&str> = corrected.split_whitespace().collect();
    if words.is_empty() || (sentence_initial && words.len() == 1) {
        return false;
    }
    words.iter().all(|w| {
        w.chars().next().is_some_and(char::is_uppercase) && w.chars().any(char::is_lowercase)
    })
}

/// Dictionary candidates from the user's edit of `pasted` into `edited`,
/// in text order.
pub fn learn_edit(pasted: &str, edited: &str, dict: &Dictionary) -> Vec<Candidate> {
    let steps = align_words(pasted, edited);
    let mut out: Vec<Candidate> = Vec::new();
    let mut prev_edited: Option<&str> = None;
    let mut i = 0;
    while i < steps.len() {
        if steps[i].op == Op::C {
            prev_edited = steps[i].to.as_deref();
            i += 1;
            continue;
        }
        let start = i;
        while i < steps.len() && steps[i].op != Op::C {
            i += 1;
        }
        let block = &steps[start..i];
        let initial = prev_edited.is_none_or(ends_sentence);
        if let Some(c) = candidate(block, initial, dict) {
            let full = out.len() >= MAX_CANDIDATES;
            match out
                .iter_mut()
                .find(|o| o.original == c.original && o.corrected == c.corrected)
            {
                Some(o) => o.looks_like_name |= c.looks_like_name,
                None if !full => out.push(c),
                None => {}
            }
        }
        if let Some(w) = block.iter().rev().find_map(|s| s.to.as_deref()) {
            prev_edited = Some(w);
        }
    }
    out
}

/// The candidate of one block of non-matching steps. A block without a
/// substitution (insertions or deletions alone) gives nothing.
fn candidate(block: &[Step], sentence_initial: bool, dict: &Dictionary) -> Option<Candidate> {
    if !block.iter().any(|s| s.op == Op::S) {
        return None;
    }
    let words = |side: fn(&Step) -> Option<&str>| -> Vec<&str> {
        block
            .iter()
            .filter_map(side)
            .map(trim_punct)
            .filter(|w| !w.is_empty())
            .collect()
    };
    let orig = words(|s| s.from.as_deref());
    let corr = words(|s| s.to.as_deref());
    if orig.is_empty()
        || corr.is_empty()
        || orig.len() > MAX_RUN_WORDS
        || corr.len() > MAX_RUN_WORDS
    {
        return None;
    }
    let original = orig.join(" ");
    let corrected = corr.join(" ");
    let sentence_case = sentence_initial && original.to_lowercase() == corrected.to_lowercase();
    if sentence_case
        || corr.iter().all(|w| is_stopword(w))
        || [&original, &corrected]
            .iter()
            .any(|s| s.contains(['\'', '’']))
        || in_dictionary(&corrected, dict)
        || !corrected.chars().any(char::is_alphabetic)
    {
        return None;
    }
    Some(Candidate {
        looks_like_name: looks_like_name(&corrected, sentence_initial),
        original,
        corrected,
    })
}

/// The word ends in `.`, `!` or `?`: the next word starts a sentence.
fn ends_sentence(w: &str) -> bool {
    w.ends_with(['.', '!', '?'])
}

/// Corrected is already a dictionary word or a replacement target.
fn in_dictionary(corrected: &str, dict: &Dictionary) -> bool {
    let same = |a: &str| a.trim().to_lowercase() == corrected.to_lowercase();
    dict.entries
        .iter()
        .any(|e| same(&e.word) || e.replacement.as_deref().is_some_and(same))
}

const STOPWORDS: &[&str] = &[
    "um", "uh", "like", "okay", "ok", "yes", "no", "just", "very", "there", "here", "so", "then",
    "not", "the", "a", "an", "and", "or", "but", "to", "of", "in", "on", "at", "for", "with", "is",
    "it", "this", "that", "i", "you", "we", "they", "he", "she", "my", "your", "be", "have", "do",
    "was", "are", "will", "can", "would", "should", "if", "what", "who", "me", "us", "him", "her",
    "them", "its", "our", "their", "these", "those", "which", "as", "by", "from", "were", "been",
    "has", "had", "does", "did", "could",
];

fn is_stopword(w: &str) -> bool {
    STOPWORDS.contains(&w.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::DictEntry;

    fn words(c: &[Candidate]) -> Vec<(&str, &str)> {
        c.iter()
            .map(|c| (c.original.as_str(), c.corrected.as_str()))
            .collect()
    }

    fn names(c: &[Candidate]) -> Vec<(&str, bool)> {
        c.iter()
            .map(|c| (c.corrected.as_str(), c.looks_like_name))
            .collect()
    }

    fn learn(a: &str, b: &str) -> Vec<Candidate> {
        learn_edit(a, b, &Dictionary::default())
    }

    #[test]
    fn proper_noun_fix_is_a_candidate() {
        let c = learn("I use zentrix hub daily", "I use Zentrix Hub daily");
        assert_eq!(words(&c), vec![("zentrix hub", "Zentrix Hub")]);
    }

    #[test]
    fn no_edit_no_candidates() {
        assert!(learn("same text", "same text").is_empty());
    }

    #[test]
    fn stopword_substitutions_are_dropped() {
        assert!(learn("send it to the team", "send it to a team").is_empty());
    }

    #[test]
    fn existing_dictionary_words_are_dropped() {
        let dict = Dictionary {
            entries: vec![DictEntry::word("Kubernetes")],
        };
        assert!(learn_edit(
            "deploy to cubernetes now",
            "deploy to Kubernetes now",
            &dict
        )
        .is_empty());
    }

    #[test]
    fn replacement_targets_are_dropped_but_keys_are_kept() {
        let dict = Dictionary {
            entries: vec![DictEntry::replacement("lumo", "Lumio")],
        };
        assert!(learn_edit("ask lomu now", "ask lumio now", &dict).is_empty());
        assert_eq!(
            words(&learn_edit("ask lumo now", "ask Lumia now", &dict)),
            vec![("lumo", "Lumia")]
        );
    }

    #[test]
    fn apostrophes_on_either_side_are_dropped() {
        assert!(learn("I do not know", "I don't know").is_empty());
        assert!(learn("so it is fine", "so it's fine").is_empty());
        assert!(learn("I dont know", "I don’t know").is_empty());
        assert!(learn("ask lumo’s team", "ask Lumio team").is_empty());
    }

    #[test]
    fn either_side_over_the_run_limit_is_dropped() {
        assert!(learn("ask lumo now", "ask one two three four five now").is_empty());
        assert!(learn("x a b c d e y", "x Lumio y").is_empty());
        assert_eq!(
            words(&learn("x a b c d y", "x Lumio y")),
            vec![("a b c d", "Lumio")]
        );
    }

    #[test]
    fn stopwords_are_exactly_the_list() {
        for w in [
            "um", "uh", "like", "okay", "ok", "yes", "just", "very", "here", "could", "those",
        ] {
            assert!(
                learn("we zentra fine", &format!("we {w} fine")).is_empty(),
                "{w}"
            );
        }
        for w in [
            "his", "yeah", "well", "oh", "now", "really", "where", "am", "may", "er",
        ] {
            assert_eq!(
                words(&learn("we zentra fine", &format!("we {w} fine"))),
                vec![("zentra", w)],
                "{w}"
            );
        }
        // Case-insensitive, and only the corrected side counts.
        assert!(learn("we zentra fine", "we The fine").is_empty());
        assert_eq!(
            words(&learn("we um fine", "we Zentra fine")),
            vec![("um", "Zentra")]
        );
        assert_eq!(
            words(&learn("we zentra fine", "we the Lumio fine")),
            vec![("zentra", "the Lumio")]
        );
    }

    #[test]
    fn long_rewrites_are_not_vocabulary() {
        assert!(learn(
            "please call me back tomorrow morning at nine",
            "ring my office line whenever you are free later"
        )
        .is_empty());
    }

    #[test]
    fn punctuation_only_changes_are_ignored() {
        assert!(learn("hello world", "Hello, world.").is_empty());
    }

    #[test]
    fn sentence_case_is_not_a_correction() {
        assert!(learn("zentrix is down", "Zentrix is down").is_empty());
        assert!(learn("lumo hub one is here", "Lumo Hub One is here").is_empty());
        assert!(learn("Lumo is here", "lumo is here").is_empty());
        assert!(learn("Done. lumo is here", "Done. Lumo is here").is_empty());
        assert!(learn("Done? lumo hub", "Done? Lumo Hub").is_empty());
        assert_eq!(
            words(&learn("the zentrix is down", "the Zentrix is down")),
            vec![("zentrix", "Zentrix")]
        );
        for (a, b) in [
            ("ok lumo", "ok Lumo"),
            ("ok\nlumo", "ok\nLumo"),
            ("ok: lumo", "ok: Lumo"),
            ("wait… lumo", "wait… Lumo"),
        ] {
            assert_eq!(words(&learn(a, b)), vec![("lumo", "Lumo")], "{a:?}");
        }
    }

    #[test]
    fn sentence_initial_single_word_is_not_a_name() {
        let c = learn("lumo is here", "Lumio is here");
        assert_eq!(words(&c), vec![("lumo", "Lumio")]);
        assert_eq!(names(&c), vec![("Lumio", false)]);
    }

    #[test]
    fn a_capitalised_fix_mid_sentence_is_a_name() {
        let c = learn("I met gina at the station", "I met Jenna at the station");
        assert_eq!(names(&c), vec![("Jenna", true)]);
        assert_eq!(words(&c), vec![("gina", "Jenna")]);
        let c = learn("ask jenna about it", "ask Jenna about it");
        assert_eq!(names(&c), vec![("Jenna", true)]);
        let c = learn("we use zentrix hub", "we use Zentrix Hub");
        assert_eq!(names(&c), vec![("Zentrix Hub", true)]);
    }

    #[test]
    fn lower_case_and_sentence_case_fixes_are_not_names() {
        let c = learn("deploy the cubernetes job", "deploy the kubernetes job");
        assert_eq!(names(&c), vec![("kubernetes", false)]);
        let c = learn("Weather we go is open.", "Whether we go is open.");
        assert_eq!(names(&c), vec![("Whether", false)]);
        let c = learn("Done. Weather we go", "Done. Whether we go");
        assert_eq!(names(&c), vec![("Whether", false)]);
        let c = learn("call doc marlo today", "call Dr marlowe today");
        assert_eq!(names(&c), vec![("Dr marlowe", false)]);
        assert_eq!(words(&c), vec![("doc marlo", "Dr marlowe")]);
    }

    #[test]
    fn looks_like_name_rules() {
        assert!(looks_like_name("Jenna", false));
        assert!(looks_like_name("Mara Quill", true));
        assert!(!looks_like_name("Jenna", true));
        assert!(!looks_like_name("I", false));
        assert!(!looks_like_name("NASA", false));
        assert!(!looks_like_name("jenna", false));
        assert!(!looks_like_name("", false));
        assert!(!looks_like_name("42", false));
    }

    #[test]
    fn name_flag_defaults_to_false_in_old_json() {
        let c: Candidate = serde_json::from_str(r#"{"original":"a","corrected":"B"}"#).unwrap();
        assert!(!c.looks_like_name);
    }

    #[test]
    fn capped_at_four() {
        let c = learn(
            "alfa one bravo two charlie three delta four echo five",
            "Alpha one Bravo two Charly three Delta four Echoo five",
        );
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].corrected, "Alpha");
        assert_eq!(c[3].corrected, "Delta");
    }

    #[test]
    fn two_names_in_one_edit() {
        let c = learn("ask gina about lumo", "ask Jenna about Lumio");
        assert_eq!(names(&c), vec![("Jenna", true), ("Lumio", true)]);
    }

    #[test]
    fn mixed_run_is_not_a_name() {
        let c = learn("go to the lumo", "go to a Lumio");
        assert_eq!(words(&c), vec![("the lumo", "a Lumio")]);
        assert_eq!(names(&c), vec![("a Lumio", false)]);
    }

    #[test]
    fn insertions_next_to_a_substitution_join_it() {
        let c = learn("see you at lumo", "see you at Lumio tomorrow then bye");
        assert_eq!(words(&c), vec![("lumo", "Lumio tomorrow then bye")]);
        // Insertions or deletions alone give nothing.
        assert!(learn("see you at lumo", "see you at lumo again").is_empty());
        assert!(learn("see you at lumo again", "see you at lumo").is_empty());
    }

    #[test]
    fn numbers_are_not_vocabulary() {
        // Change: a correction without a letter is dropped.
        assert!(learn("room 42 is free", "room 43 is free").is_empty());
    }

    #[test]
    fn duplicates_merge_into_the_first() {
        // Change: one candidate per (original, corrected), named if any was.
        let c = learn("lumo said hi to lumo", "Lumio said hi to Lumio");
        assert_eq!(names(&c), vec![("Lumio", true)]);
        assert_eq!(c.len(), 1);
    }
}
