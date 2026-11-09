//! Fit the dictation to the text around the cursor (spec §5.1–§5.4,
//! §5.6).

use super::chars::{is_cjk, is_opening_mark, is_sentence_end, strip_sentence_end};
use super::starts_with_proper_noun;
use crate::dictionary::DictEntry;

/// The field around the cursor.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Field<'a> {
    pub before: &'a str,
    pub selected: &'a str,
    pub after: &'a str,
}

/// Words that keep their capital: user names and screen words, and the
/// dictionary.
#[derive(Debug, Clone, Copy)]
pub(super) struct Proper<'a> {
    pub names: &'a [String],
    pub dict: &'a [DictEntry],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Fitted {
    /// The text replaces a selection and has taken its shape; nothing more
    /// is added to it.
    Selection(String),
    /// The text sits at the cursor. `room_after`: the field reads as ready
    /// for more typing right after it (a trailing space may follow).
    Cursor { text: String, room_after: bool },
}

pub(super) fn fit(text: &str, field: Field, proper: Proper) -> Fitted {
    let t = text.trim();
    if !field.selected.is_empty() {
        return Fitted::Selection(over_selection(t, field.selected, proper));
    }
    if t.is_empty() {
        return Fitted::Cursor {
            text: String::new(),
            room_after: false,
        };
    }
    let mut out = if lowers_next(field.before) {
        lower_first(t, proper)
    } else {
        t.to_string()
    };
    let a = field.after.trim_start_matches([' ', '\t']);
    let continues = a
        .chars()
        .next()
        .is_some_and(|c| c.is_lowercase() || matches!(c, ',' | ';' | ':' | '.' | '?' | '!'));
    if continues {
        out = strip_sentence_end(&out).to_string();
    }
    if needs_leading_space(field.before, &out) {
        out.insert(0, ' ');
    }
    let room_after = field.after.is_empty()
        || field
            .after
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
    Fitted::Cursor {
        text: out,
        room_after,
    }
}

/// §5.1: a space between the text before the cursor and the dictation.
fn needs_leading_space(before: &str, t: &str) -> bool {
    let (Some(last), Some(first)) = (before.chars().last(), t.chars().next()) else {
        return false;
    };
    !last.is_whitespace()
        && !is_opening_mark(last)
        && !matches!(last, '-' | '/' | '@')
        && !is_cjk(last)
        && !is_cjk(first)
}

/// §5.2: the text before the cursor ends mid-sentence.
fn lowers_next(before: &str) -> bool {
    before
        .trim_end_matches([' ', '\u{a0}'])
        .chars()
        .last()
        .is_some_and(|c| {
            c.is_alphanumeric() || matches!(c, ',' | ';' | ':' | '-' | '–' | '—' | '/')
        })
}

/// Lower the first letter of `t` when its first word is a plain
/// capitalised word that is not a proper noun.
fn lower_first(t: &str, proper: Proper) -> String {
    let Some(word) = t.split_whitespace().next() else {
        return t.to_string();
    };
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return t.to_string();
    };
    if !first.is_uppercase()
        || chars.any(char::is_uppercase)
        || starts_with_proper_noun(t, proper.names, proper.dict)
    {
        return t.to_string();
    }
    let rest = &t[first.len_utf8()..];
    first.to_lowercase().chain(rest.chars()).collect()
}

fn upper_first(t: &str) -> String {
    let mut chars = t.chars();
    match chars.next() {
        Some(c) if c.is_lowercase() => c.to_uppercase().chain(chars).collect(),
        _ => t.to_string(),
    }
}

/// §5.6: the text takes the selection's case, ending and surrounding
/// whitespace.
fn over_selection(t: &str, selected: &str, proper: Proper) -> String {
    let core = selected.trim();
    let mut out = match core.chars().next() {
        Some(c) if c.is_lowercase() => lower_first(t, proper),
        Some(c) if c.is_uppercase() => upper_first(t),
        _ => t.to_string(),
    };
    if !core.chars().last().is_some_and(is_sentence_end) {
        out = strip_sentence_end(&out).to_string();
    }
    let lead = &selected[..selected.len() - selected.trim_start().len()];
    let trail = &selected[selected.trim_end().len()..];
    format!("{lead}{out}{trail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARCEL: &str = "The parcel arrived early.";

    fn proper<'a>(names: &'a [String], dict: &'a [DictEntry]) -> Proper<'a> {
        Proper { names, dict }
    }

    fn at(before: &str, text: &str, after: &str) -> (String, bool) {
        let f = Field {
            before,
            selected: "",
            after,
        };
        match fit(text, f, proper(&[], &[])) {
            Fitted::Cursor { text, room_after } => (text, room_after),
            Fitted::Selection(s) => panic!("selection {s}"),
        }
    }

    fn text_at(before: &str, text: &str) -> String {
        at(before, text, "").0
    }

    fn over(selected: &str, text: &str) -> String {
        let f = Field {
            before: "ignored ",
            selected,
            after: " ignored",
        };
        match fit(text, f, proper(&[], &[])) {
            Fitted::Selection(s) => s,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn leading_space() {
        assert_eq!(text_at("I think", PARCEL), " the parcel arrived early.");
        assert_eq!(text_at("Well,", PARCEL), " the parcel arrived early.");
        assert_eq!(text_at("Done.", PARCEL), " The parcel arrived early.");
        assert_eq!(text_at("42", "items."), " items.");
        assert_eq!(text_at("wait...", "Wow."), " Wow.");
        assert_eq!(text_at("wow!", "Wow."), " Wow.");
        assert_eq!(text_at("I think ", PARCEL), "the parcel arrived early.");
        for b in ["(", "“", "He said \"", "well-", "a/", "mail@"] {
            assert!(!text_at(b, PARCEL).starts_with(' '), "{b}");
        }
        assert!(!text_at("你好", "Hello there.").starts_with(' '));
        assert!(!text_at("Hello", "你好。").starts_with(' '));
        assert_eq!(text_at("", PARCEL), PARCEL);
    }

    #[test]
    fn lower_case_start_mid_sentence() {
        for b in [
            "Note: ", "so - ", "a ", "a\u{a0}", "x; ", "y — ", "a/", "42 ",
        ] {
            assert!(text_at(b, PARCEL).trim_start().starts_with("the"), "{b:?}");
        }
        for b in [
            "Done. ", "Really? ", "Wait... ", "line\n", "col\t", "(", "\"", "@", "",
        ] {
            assert!(text_at(b, PARCEL).trim_start().starts_with("The"), "{b:?}");
        }
        assert_eq!(text_at("and ", "NASA called."), "NASA called.");
        assert_eq!(text_at("and ", "IPhone works."), "IPhone works.");
        assert_eq!(text_at("and ", "McKay called."), "McKay called.");
        assert_eq!(text_at("and ", "42 people came."), "42 people came.");
        assert_eq!(text_at("ok ", "Done. Next step."), "done. Next step.");
        assert_eq!(
            text_at("Done. ", "the parcel arrived early."),
            "the parcel arrived early."
        );
        assert_eq!(text_at("and ", "I left early."), "I left early.");
        assert_eq!(text_at("and ", "I'm on my way."), "I'm on my way.");
        assert_eq!(text_at("and ", "I’ll be there."), "I’ll be there.");
    }

    #[test]
    fn proper_nouns_keep_their_capital() {
        let names = vec!["Mara".to_string(), "Quillon".to_string()];
        let dict = vec![DictEntry::word("Kubernetes")];
        let f = Field {
            before: "and ",
            ..Default::default()
        };
        let run =
            |t: &str, names: &[String], dict: &[DictEntry]| match fit(t, f, proper(names, dict)) {
                Fitted::Cursor { text, .. } => text,
                other => panic!("{other:?}"),
            };
        assert_eq!(run("Mara called.", &names, &[]), "Mara called.");
        assert_eq!(run("Mara called.", &[], &[]), "mara called.");
        assert_eq!(run("Quillon is down.", &names, &[]), "Quillon is down.");
        assert_eq!(run("Kubernetes failed.", &[], &dict), "Kubernetes failed.");
    }

    #[test]
    fn final_punctuation_and_room_after() {
        assert_eq!(at("", PARCEL, ""), (PARCEL.into(), true));
        assert_eq!(
            at("", PARCEL, "and then"),
            ("The parcel arrived early".into(), true)
        );
        assert_eq!(
            at("", PARCEL, " and then"),
            ("The parcel arrived early".into(), false)
        );
        assert_eq!(at("", PARCEL, "Then we left."), (PARCEL.into(), true));
        assert_eq!(at("", PARCEL, "42"), (PARCEL.into(), true));
        for a in [".", ",", ";", "?"] {
            assert_eq!(
                at("", PARCEL, a),
                ("The parcel arrived early".into(), false),
                "{a}"
            );
        }
        assert_eq!(at("", PARCEL, ")"), (PARCEL.into(), false));
        assert_eq!(at("", PARCEL, "\""), (PARCEL.into(), false));
        assert_eq!(at("", PARCEL, " "), (PARCEL.into(), false));
        assert_eq!(
            at("", PARCEL, ", then"),
            ("The parcel arrived early".into(), false)
        );
        assert_eq!(
            at("so ", "Is it ready?", " now"),
            ("is it ready".into(), false)
        );
        // Change: a newline is whitespace (no space before it); a closing
        // mark keeps the period.
        assert_eq!(at("", PARCEL, "\nNext line"), (PARCEL.into(), false));
        assert_eq!(at("", PARCEL, ") more"), (PARCEL.into(), false));
    }

    #[test]
    fn whitespace_only_text_is_empty() {
        assert_eq!(at("and", "   ", ""), (String::new(), false));
    }

    #[test]
    fn selection_shape() {
        assert_eq!(over("Bad words.", "New words."), "New words.");
        assert_eq!(over("Bad words", "New words."), "New words");
        assert_eq!(over("bad words.", "New words."), "new words.");
        assert_eq!(over("x", "New words."), "new words");
        assert_eq!(over("BAD", "New words."), "New words");
        assert_eq!(over("Bad words?", "New words."), "New words.");
        assert_eq!(over("bad", "New words. More here."), "new words. More here");
        assert_eq!(over("bad", "Is it new?"), "is it new");
        assert_eq!(over(" bad words ", "New words."), " new words ");
        assert_eq!(over("Bad", "new words."), "New words");
        assert_eq!(over("42", "New words."), "New words");
        // Change: the trimmed selection's end decides.
        assert_eq!(over(" Bad words. ", "New words."), " New words. ");
    }
}
