//! Fit the dictation to the text around the cursor (spec §5.1–§5.4,
//! §5.6).

use super::chars::{is_cjk, is_closing_quote, is_emoji, is_sentence_end};
use super::starts_with_proper_noun;
use crate::dictionary::DictEntry;

/// The text around the cursor (no selection).
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Field<'a> {
    pub before: &'a str,
    pub after: &'a str,
}

/// Words that keep their capital: user names and screen words, and the
/// dictionary.
#[derive(Debug, Clone, Copy)]
pub(super) struct Proper<'a> {
    pub names: &'a [String],
    pub dict: &'a [DictEntry],
}

/// The dictation at the cursor (no selection), already trimmed and fitted.
/// `room_after`: §5.4's trailing-space condition on `after` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AtCursor {
    pub text: String,
    pub room_after: bool,
}

/// §5.1–§5.4 for a non-empty trimmed `t` and an empty selection.
pub(super) fn at_cursor(t: &str, field: Field, proper: Proper) -> AtCursor {
    let mut out = if lowers_next(field.before) {
        lower_first(t, proper)
    } else {
        t.to_string()
    };
    if continues(field.after) {
        out = out
            .trim_end_matches(['.', '!', '?', '…'])
            .trim_end()
            .to_string();
    }
    if needs_leading_space(field.before, t) {
        out.insert(0, ' ');
    }
    AtCursor {
        text: out,
        room_after: room_after(field.after),
    }
}

/// §5.1: a space between the text before the cursor and the dictation.
fn needs_leading_space(before: &str, t: &str) -> bool {
    let Some(first) = t.chars().next() else {
        return false;
    };
    let mut rev = before.chars().rev();
    let Some(c) = rev.next() else {
        return false;
    };
    let prev = rev.next();
    if is_cjk(first) {
        return false;
    }
    (c.is_alphanumeric() && !is_cjk(c))
        || matches!(
            c,
            ',' | '.'
                | ';'
                | ':'
                | '!'
                | '?'
                | ')'
                | ']'
                | '}'
                | '%'
                | '&'
                | '+'
                | '>'
                | '|'
                | '*'
                | '='
                | '~'
        )
        || (is_closing_quote(c) && prev.is_some_and(|p| !p.is_whitespace()))
        || is_emoji(c)
        || (c == '-' && prev.is_none_or(char::is_whitespace))
}

/// §5.2: the text before the cursor ends mid-sentence.
fn lowers_next(before: &str) -> bool {
    let mut b = before.trim_end_matches([' ', '\u{a0}']);
    loop {
        let mut rev = b.chars().rev();
        let Some(c) = rev.next() else {
            return false;
        };
        let transparent = matches!(
            c,
            ' ' | ',' | ';' | ':' | '-' | '—' | '/' | '(' | ')' | '[' | ']' | '{' | '}'
        ) || is_emoji(c)
            || (is_closing_quote(c) && rev.next().is_some_and(|p| !p.is_whitespace()));
        if !transparent {
            return c.is_alphanumeric();
        }
        b = &b[..b.len() - c.len_utf8()];
    }
}

/// An all-caps word: ignoring a trailing `'s` / `’s`, at least two letters
/// and every letter upper-case.
fn is_all_caps(word: &str) -> bool {
    let w = word
        .strip_suffix("'s")
        .or_else(|| word.strip_suffix("’s"))
        .unwrap_or(word);
    let mut n = 0;
    for c in w.chars().filter(|c| c.is_alphabetic()) {
        if !c.is_uppercase() {
            return false;
        }
        n += 1;
    }
    n >= 2
}

/// Lower the first character of `t` when it is an upper-case letter and
/// the first word is neither a proper noun nor all caps.
fn lower_first(t: &str, proper: Proper) -> String {
    let Some(first) = t.chars().next().filter(|c| c.is_uppercase()) else {
        return t.to_string();
    };
    let word = t.split_whitespace().next().unwrap_or("");
    if is_all_caps(word) || starts_with_proper_noun(t, proper.names, proper.dict) {
        return t.to_string();
    }
    first
        .to_lowercase()
        .chain(t[first.len_utf8()..].chars())
        .collect()
}

fn upper_first(t: &str) -> String {
    let mut chars = t.chars();
    match chars.next() {
        Some(c) if c.is_lowercase() => c.to_uppercase().chain(chars).collect(),
        _ => t.to_string(),
    }
}

/// §5.4: the dictation runs on into `after`.
fn continues(after: &str) -> bool {
    after
        .trim_start_matches([' ', '\u{a0}'])
        .trim_start_matches([' ', '-', '—', '(', ')', '[', ']', '{', '}', '/'])
        .chars()
        .next()
        .is_some_and(|c| c.is_lowercase() || matches!(c, ',' | ';' | ':' | '.' | '?' | '!'))
}

/// §5.4: `after` leaves room for a trailing space.
fn room_after(after: &str) -> bool {
    after.is_empty()
        || after.chars().next().is_some_and(char::is_alphanumeric)
        || after.starts_with("- ")
}

/// §5.6: the trimmed `t` takes the selection's case, ending and
/// surrounding whitespace.
pub(super) fn over_selection(t: &str, selected: &str, proper: Proper) -> String {
    let core = selected.trim();
    let mut out = match core.chars().next() {
        Some(c) if c.is_lowercase() => lower_first(t, proper),
        Some(c) if c.is_uppercase() => upper_first(t),
        _ => t.to_string(),
    };
    if !core.chars().last().is_some_and(is_sentence_end) {
        out = out.trim_end_matches(is_sentence_end).trim_end().to_string();
    }
    let lead = &selected[..selected.len() - selected.trim_start().len()];
    let trail = &selected[selected.trim_end().len()..];
    format!("{lead}{out}{trail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARCEL: &str = "The parcel arrived early.";

    fn none<'a>() -> Proper<'a> {
        Proper {
            names: &[],
            dict: &[],
        }
    }

    fn at(before: &str, text: &str, after: &str) -> (String, bool) {
        let f = Field { before, after };
        let r = at_cursor(text, f, none());
        (r.text, r.room_after)
    }

    fn text_at(before: &str, text: &str) -> String {
        at(before, text, "").0
    }

    fn over(selected: &str, text: &str) -> String {
        over_selection(text, selected, none())
    }

    #[test]
    fn leading_space() {
        assert_eq!(text_at("I think", PARCEL), " the parcel arrived early.");
        assert_eq!(text_at("Well,", PARCEL), " the parcel arrived early.");
        assert_eq!(text_at("Done.", PARCEL), " The parcel arrived early.");
        assert_eq!(text_at("42", "items."), " items.");
        assert_eq!(text_at("wait...", "Wow."), " Wow.");
        assert_eq!(text_at("wow!", "Wow."), " Wow.");
        for b in [
            "x)", "x]", "x}", "x\"", "x😀", "50%", "a&", "a+", "a>", "a|", "a*", "a=", "a~", "-",
            "x -", "x;", "x:", "x?",
        ] {
            assert!(text_at(b, PARCEL).starts_with(' '), "{b:?}");
        }
        for b in [
            "I think ",
            "(",
            "“",
            "He said \"",
            "\"",
            "a \"",
            "well-",
            "a-",
            "a/",
            "mail@",
            "x—",
            "x–",
            "x…",
            "$",
            "#",
            "_",
            "^",
            "•",
            "<",
            "«",
            "»",
            "¿",
            "¡",
            "‘",
            "[",
            "{",
            "a\u{a0}",
            "a\t",
            "a\n",
            "你好",
        ] {
            assert!(!text_at(b, PARCEL).starts_with(' '), "{b:?}");
        }
        assert!(!text_at("Hello", "你好。").starts_with(' '));
        assert_eq!(text_at("", PARCEL), PARCEL);
    }

    #[test]
    fn lowered_after() {
        for b in [
            "Note: ",
            "Well, ",
            "so - ",
            "x - ",
            "a- ",
            "a/ ",
            "wait — ",
            "a—",
            "see (",
            "x(",
            "x ( ",
            "done) ",
            "x;) ",
            "x)]",
            "x [",
            "x {",
            "great 😀 ",
            "x©",
            "x\" ",
            "x’",
            "“x” ",
            "x ",
            "a\u{a0}",
            "42 ",
            "你好",
        ] {
            assert!(text_at(b, PARCEL).trim_start().starts_with("the"), "{b:?}");
        }
    }

    #[test]
    fn left_after() {
        for b in [
            "Done. ",
            "Done. (",
            "Done.) ",
            "Done. - ",
            "Done. — ",
            "! (",
            "(",
            "( (",
            "- ",
            ", ",
            ": ",
            "— ",
            "😀 ",
            "\" ",
            "Done.\n(",
            "x\n) ",
            "Line one\n",
            "a\t",
            "a ’",
            "a \"",
            "x‘ ",
            "x“ ",
            "He said \"",
            "“",
            "x«",
            "x»",
            "a –",
            "a–",
            "x$",
            "x#",
            "x_",
            "x*",
            "x=",
            "x~",
            "x%",
            "x&",
            "x+",
            "x<",
            "x>",
            "x|",
            "x^",
            "x•",
            "x…",
            "x@",
            "x¿",
            "x¡",
            "x°",
            "x\u{a0}(",
            "Really? ",
            "Wait... ",
            "",
        ] {
            assert!(text_at(b, PARCEL).trim_start().starts_with("The"), "{b:?}");
        }
    }

    #[test]
    fn lowering_rules() {
        // Change: all-caps words keep their case.
        for w in [
            "NASA", "OK", "AI", "CEO", "TV", "U.S.", "I.B.M.", "NASA's", "USA’s",
        ] {
            let t = format!("{w} called.");
            assert_eq!(text_at("and ", &t), t, "{w}");
        }
        assert_eq!(text_at("and ", "IPhone works."), "iPhone works.");
        assert_eq!(text_at("and ", "EBay works."), "eBay works.");
        assert_eq!(text_at("and ", "McKay called."), "mcKay called.");
        assert_eq!(text_at("and ", "JavaScript runs."), "javaScript runs.");
        assert_eq!(text_at("and ", "PhD done."), "phD done.");
        assert_eq!(text_at("and ", "A plan."), "a plan.");
        assert_eq!(text_at("and ", "X is fine."), "x is fine.");
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
        let run = |t: &str, names: &[String], dict: &[DictEntry]| {
            at_cursor(t, f, Proper { names, dict }).text
        };
        assert_eq!(run("Mara called.", &names, &[]), "Mara called.");
        assert_eq!(run("Mara called.", &[], &[]), "mara called.");
        assert_eq!(run("Quillon is down.", &names, &[]), "Quillon is down.");
        assert_eq!(run("Kubernetes failed.", &[], &dict), "Kubernetes failed.");
    }

    #[test]
    fn continues_into_after() {
        for a in [
            "and then",
            " and then",
            " later",
            "— and",
            " — and",
            "—and",
            "-and",
            "- and",
            "-- and",
            "—  and",
            "— (and)",
            "— ,",
            "(and",
            "( and",
            ") and",
            ") more",
            "{x",
            "]x",
            "[x",
            "/x",
            ": x",
            "; more",
            "... and",
            ".",
            ",",
            "?",
            " and",
            "\u{a0}and",
        ] {
            assert!(continues(a), "{a:?}");
        }
        for a in [
            "– and",
            "— And",
            "- And",
            "Then",
            "— 5",
            "-5",
            "-",
            "—",
            "— ",
            "…and",
            "\" and",
            "’ and",
            "😀 and",
            "$5",
            "_x",
            "@x",
            "\tand",
            "—\tand",
            "-\nand",
            "\nNext line",
            " Next",
            "",
        ] {
            assert!(!continues(a), "{a:?}");
        }
    }

    #[test]
    fn final_punctuation_and_room_after() {
        assert_eq!(at("", PARCEL, ""), (PARCEL.into(), true));
        let early = "The parcel arrived early";
        assert_eq!(at("", PARCEL, "and then"), (early.into(), true));
        assert_eq!(at("", PARCEL, " and then"), (early.into(), false));
        assert_eq!(at("", PARCEL, "Then we left."), (PARCEL.into(), true));
        assert_eq!(at("", PARCEL, "42"), (PARCEL.into(), true));
        for a in [".", ",", ";", "?", "— and"] {
            assert_eq!(at("", PARCEL, a), (early.into(), false), "{a:?}");
        }
        for a in [")", "\"", "—", "😀", "…", "$5", "\tand", " "] {
            assert_eq!(at("", PARCEL, a), (PARCEL.into(), false), "{a:?}");
        }
        assert_eq!(at("", PARCEL, "- And"), (PARCEL.into(), true));
        assert_eq!(at("", PARCEL, "- and"), (early.into(), true));
        assert_eq!(at("", PARCEL, "-  and"), (early.into(), true));
        assert_eq!(at("", "Wait!?", "x"), ("Wait".into(), true));
        assert_eq!(at("", "Wait…", "x"), ("Wait".into(), true));
        // CJK marks are not stripped.
        assert_eq!(at("", "世界很好。", "再见"), ("世界很好。".into(), true));
        assert_eq!(at("", "好的。", "x"), ("好的。".into(), true));
        // Change: no space before a newline.
        assert_eq!(at("", PARCEL, "\nNext line"), (PARCEL.into(), false));
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
        assert_eq!(over("bad", "NASA called."), "NASA called");
        assert_eq!(over("好的。", "New words。"), "New words。");
        // Change: the trimmed selection's end decides.
        assert_eq!(over(" Bad words. ", "New words."), " New words. ");
    }
}
