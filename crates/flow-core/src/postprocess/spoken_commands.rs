//! Spoken formatting commands (D27 layer 2, docs/findings/2026-10-24-formatting-gap.md
//! §6-7): quotation marks, parentheses, brackets and dashes said out loud.
//! A child module of `spoken` (it shares the token model); runs inside
//! [`super::spoken_cleanup`] after spoken punctuation, fillers, stutters
//! and backtracking, before ITN.
//!
//! | Spoken | Written |
//! |---|---|
//! | quote … unquote / close quote / end quote / end of quote | `"…"` |
//! | open quote / begin quote … (same closers) | `"…"` |
//! | quote unquote X | `"X"` |
//! | ", quote, X," / ", in quotes, X," (no closer) | `"X"` |
//! | open paren(s/thesis/theses) … close / end paren… | `(…)` |
//! | ", in parentheses, X," / ", open paren, X," (no closer) | `(X)` |
//! | open bracket(s) … close / end bracket(s); ", in brackets, X," | `[…]` |
//! | X dash Y, X em dash Y | `X — Y` |
//!
//! **Boundaries.** A closed pair converts exactly (span ≤ [`MAX_SPAN`]
//! tokens, same paragraph). A command without a closer converts only when
//! Parakeet's pause commas wrap it (", in parentheses, the USB-C one, to
//! …"): the span runs to the next pause comma or sentence end, at most
//! [`MAX_OPEN_SPAN`] tokens. Without the pause commas the words stay as
//! spoken ("put the area code in parentheses", "the formula is in
//! parentheses on the board"), with one narrow exception for the
//! aside-before-a-preposition shape the user dictates ("the charger in
//! parentheses the USB-C one to the meeting"): see [`bare_aside`].
//!
//! **Punctuation (US style).** A period or comma after a closing quote
//! goes inside it; `?`/`!` stay outside unless the quotation had them.
//! Parentheses take nothing inside. The pause comma before an opening mark
//! is dropped, except before a quotation of two or more words after a
//! speech verb ("She said, "I'll be there soon," and left."), whose first
//! word is then capitalised; that form keeps its closing comma too.
//!
//! **Dash.** Spoken "dash" is an em dash with a space on each side
//! (" — "), the dictation convention (and AP's): it reads the same in
//! chat and documents, and survives apps that would join a closed dash to
//! the next word. Not after a determiner or a subject ("a dash of salt",
//! "I'll dash out"), not before "of"/"to"/"out"/… or a number, and never
//! at the start or end of the text.

use super::{capitalise, cores, matches_at, number_word, Tok};

/// Longest closed span, in tokens.
const MAX_SPAN: usize = 40;
/// Longest span of a command without a closer, in tokens.
const MAX_OPEN_SPAN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mark {
    Quote,
    Paren,
    Bracket,
}

impl Mark {
    fn open(self) -> &'static str {
        match self {
            Mark::Quote => "\"",
            Mark::Paren => "(",
            Mark::Bracket => "[",
        }
    }
    fn close(self) -> &'static str {
        match self {
            Mark::Quote => "\"",
            Mark::Paren => ")",
            Mark::Bracket => "]",
        }
    }
}

/// Verbs that introduce a quotation with a comma ("he said, "…"").
const SPEECH_VERBS: &[&str] = &[
    "said",
    "says",
    "say",
    "saying",
    "wrote",
    "writes",
    "write",
    "asked",
    "asks",
    "replied",
    "replies",
    "added",
    "adds",
    "answered",
    "answers",
    "yelled",
    "shouted",
    "texted",
    "tweeted",
    "posted",
    "goes",
    "went",
    "told",
    "tells",
    "announced",
    "explained",
    "messaged",
    "emailed",
];

/// Words before a bare "quote" that make it a noun ("a quote", "the
/// famous quote", "price quote").
const QUOTE_NOUN_BEFORE: &[&str] = &[
    "a",
    "an",
    "the",
    "this",
    "that",
    "these",
    "those",
    "my",
    "your",
    "his",
    "her",
    "our",
    "their",
    "its",
    "one",
    "no",
    "any",
    "each",
    "every",
    "another",
    "famous",
    "favorite",
    "favourite",
    "good",
    "great",
    "nice",
    "short",
    "long",
    "final",
    "written",
    "free",
    "price",
    "insurance",
    "best",
    "new",
    "first",
    "last",
    "word",
    "same",
    "of",
    "for",
    "get",
    "got",
];
/// Words after a bare "quote" that make it a verb or noun ("quote me", "a
/// quote for", "quote from").
const QUOTE_LITERAL_AFTER: &[&str] = &[
    "me", "him", "her", "them", "us", "for", "from", "of", "on", "by", "marks", "mark", "is",
    "was", "and", "or", "unquoted",
];

/// Determiners that open the aside in [`bare_aside`].
const DETERMINERS: &[&str] = &[
    "the", "a", "an", "my", "your", "his", "her", "our", "their", "this", "that", "these", "those",
];
/// Prepositions that end the aside in [`bare_aside`].
const PREPOSITIONS: &[&str] = &[
    "to", "for", "at", "on", "in", "with", "from", "by", "into", "onto", "during", "before",
    "after", "until", "under", "over", "near", "inside", "behind",
];
/// Words before a bare "in parentheses" that make it literal ("is in
/// parentheses", "put it in parentheses", "written in brackets").
const ASIDE_BLOCK_BEFORE: &[&str] = &[
    "is", "are", "was", "were", "be", "been", "being", "it", "them", "this", "that", "put", "puts",
    "write", "writes", "wrote", "written", "shown", "listed", "given", "go", "goes", "appear",
    "appears", "appeared", "stay", "stays", "say", "says", "said", "number", "numbers", "and",
    "or", "not", "also", "just", "only",
];

/// Words before "dash" that make it a noun or a verb.
const DASH_BLOCK_BEFORE: &[&str] = &[
    "a", "an", "the", "this", "that", "my", "your", "his", "her", "our", "their", "its", "mad",
    "quick", "little", "last", "i", "you", "we", "they", "he", "she", "i'll", "we'll", "you'll",
    "they'll", "he'll", "she'll", "to", "will", "can", "could", "would", "should", "must", "gotta",
    "just", "mrs", "mr", "ms", "en", "em", "dot", "hyphen", "slash", "dash", "and", "or",
];
/// Words after "dash" that make it a noun or a verb ("dash of", "dash
/// cam", "dash out").
const DASH_BLOCK_AFTER: &[&str] = &[
    "of", "for", "to", "off", "out", "over", "across", "through", "into", "away", "back", "home",
    "cam", "board", "around", "down", "up", "and", "or", "dash",
];

fn is_paren_word(w: &str) -> bool {
    matches!(w, "paren" | "parens" | "parenthesis" | "parentheses")
}
fn is_bracket_word(w: &str) -> bool {
    matches!(w, "bracket" | "brackets")
}

/// A closer for `mark` at `k`: tokens used.
fn closer_at(c: &[String], k: usize, mark: Mark) -> Option<usize> {
    match mark {
        Mark::Quote => {
            if c[k] == "unquote" {
                Some(1)
            } else if matches_at(c, k, &["end", "of", "quote"]) {
                Some(3)
            } else if (c[k] == "close" || c[k] == "end")
                && c.get(k + 1).is_some_and(|w| w == "quote")
            {
                Some(2)
            } else {
                None
            }
        }
        Mark::Paren | Mark::Bracket => {
            let word = c.get(k + 1)?;
            let ok = if mark == Mark::Paren {
                is_paren_word(word)
            } else {
                is_bracket_word(word)
            };
            ((c[k] == "close" || c[k] == "end") && ok).then_some(2)
        }
    }
}

/// An explicit opener at `i` ("open quote", "open paren", "open
/// bracket", "begin quote"): (mark, tokens used).
fn explicit_opener(c: &[String], i: usize) -> Option<(Mark, usize)> {
    if !matches!(c[i].as_str(), "open" | "begin") {
        return None;
    }
    let w = c.get(i + 1)?.as_str();
    if w == "quote" || w == "quotes" {
        Some((Mark::Quote, 2))
    } else if c[i] == "open" && is_paren_word(w) {
        Some((Mark::Paren, 2))
    } else if c[i] == "open" && is_bracket_word(w) {
        Some((Mark::Bracket, 2))
    } else {
        None
    }
}

/// "in parentheses" / "in brackets" / "in quotes" at `i`.
fn in_opener(c: &[String], i: usize) -> Option<(Mark, usize)> {
    if c[i] != "in" {
        return None;
    }
    let w = c.get(i + 1)?.as_str();
    if is_paren_word(w) && w != "paren" {
        Some((Mark::Paren, 2))
    } else if w == "brackets" {
        Some((Mark::Bracket, 2))
    } else if w == "quotes" {
        Some((Mark::Quote, 2))
    } else {
        None
    }
}

/// The previous output token is a pause (ends in a comma) or there is
/// nothing before the command in its sentence.
fn pause_before(out: &[Tok]) -> bool {
    out.last()
        .is_none_or(|p| p.is_break() || p.ends_sentence() || p.suffix() == ",")
}

/// Index of the token that ends the span starting at `from`: the first
/// token with a pause comma or sentence punctuation, within `max` tokens.
fn pause_end(toks: &[Tok], from: usize, max: usize) -> Option<usize> {
    (from..toks.len().min(from + max))
        .find(|&k| {
            let t = &toks[k];
            !t.is_break() && (t.ends_clause() || t.ends_sentence())
        })
        .filter(|&k| !toks[from..=k].iter().any(Tok::is_break))
}

/// Split a token into (word, trailing punctuation it carries).
fn split_tail(t: &Tok) -> (String, String) {
    let end = t
        .text
        .trim_end_matches(|c: char| !c.is_alphanumeric())
        .len();
    (t.text[..end].to_string(), t.text[end..].to_string())
}

/// The spoken "sentence" punctuation in `s` (closing marks of an earlier,
/// nested command are kept by the caller).
fn punct_of(s: &str) -> String {
    s.chars().filter(|c| ",.;:?!".contains(*c)).collect()
}

struct Wrap {
    mark: Mark,
    /// Punctuation said after the construct (the closer's, or the span
    /// end's).
    after: String,
}

/// Emit `content` wrapped in `w.mark` onto `out`, fixing the punctuation
/// around it. `next` is the word that follows the construct, if any.
fn emit(out: &mut Vec<Tok>, mut content: Vec<Tok>, w: Wrap, next: Option<&str>) {
    let words = content.iter().filter(|t| !t.core().is_empty()).count();
    // The pause comma before the opening mark.
    let lead_comma = out
        .last()
        .is_some_and(|p| !p.is_break() && p.suffix() == ",");
    let speech = w.mark == Mark::Quote
        && lead_comma
        && words >= 2
        && out
            .last()
            .is_some_and(|p| SPEECH_VERBS.contains(&p.core().as_str()));
    if lead_comma && !speech {
        out.last_mut().unwrap().strip_suffix();
    }
    // Punctuation inside the span's last token: a quotation keeps its own
    // "?"/"!"; pause commas and periods go.
    let last = content.last_mut().unwrap();
    let (word, tail) = split_tail(last);
    let nested: String = tail.chars().filter(|c| !",.;:?!".contains(*c)).collect();
    let inner_q: String = punct_of(&tail)
        .chars()
        .filter(|c| matches!(c, '?' | '!'))
        .take(1)
        .collect();
    let mut after = punct_of(&w.after);
    let keep_comma = speech || next.is_some_and(|n| matches!(n, "but" | "so" | "yet"));
    if after.contains(',') && !keep_comma {
        after.retain(|c| c != ',');
    }
    // One mark after the construct: the strongest said.
    let after: String = ['?', '!', '.', ';', ':', ',']
        .into_iter()
        .find(|c| after.contains(*c))
        .map(String::from)
        .unwrap_or_default();
    let text = if w.mark == Mark::Quote {
        if !inner_q.is_empty() {
            // "Are you okay?" — the quotation's own mark; a period after it
            // is dropped.
            let outer = if matches!(after.as_str(), "." | ",") {
                ""
            } else {
                after.as_str()
            };
            format!("{word}{nested}{inner_q}\"{outer}")
        } else if matches!(after.as_str(), "." | ",") {
            format!("{word}{nested}{after}\"")
        } else {
            format!("{word}{nested}\"{after}")
        }
    } else {
        format!("{word}{nested}{}{after}", w.mark.close())
    };
    last.text = text;
    let first = content.first_mut().unwrap();
    first.text = format!("{}{}", w.mark.open(), first.text);
    if speech {
        first.text = capitalise(&first.text);
    }
    out.extend(content);
}

/// "the charger in parentheses the USB-C one to the meeting": an aside
/// with no pause commas. Fires only when the word before is a plain noun
/// (no punctuation, not a verb or copula, see [`ASIDE_BLOCK_BEFORE`]), the
/// aside starts with a determiner and is 2-6 words, and a preposition
/// right after it continues the sentence. Returns the index of the aside's
/// last token.
fn bare_aside(toks: &[Tok], c: &[String], out: &[Tok], from: usize) -> Option<usize> {
    let prev = out
        .last()
        .filter(|p| !p.is_break() && p.suffix().is_empty())?;
    let pc = prev.core();
    if pc.is_empty() || ASIDE_BLOCK_BEFORE.contains(&pc.as_str()) || number_word(&pc).is_some() {
        return None;
    }
    if !DETERMINERS.contains(&c.get(from)?.as_str()) {
        return None;
    }
    // The aside: unpunctuated words up to a preposition.
    for k in from + 1..toks.len().min(from + 6) {
        if toks[k].is_break() || !toks[k - 1].suffix().is_empty() {
            return None;
        }
        if PREPOSITIONS.contains(&c[k].as_str()) {
            // A word must follow the preposition (no "to." at the end).
            return (k >= from + 2 && k + 1 < toks.len()).then_some(k - 1);
        }
    }
    None
}

/// Apply every spoken formatting command. Returns the tokens unchanged
/// when nothing applies.
pub(super) fn spoken_commands(toks: Vec<Tok>) -> Vec<Tok> {
    let c = cores(&toks);
    if !c.iter().any(|w| {
        matches!(
            w.as_str(),
            "quote" | "unquote" | "quotes" | "dash" | "quote-unquote"
        ) || is_paren_word(w)
            || is_bracket_word(w)
    }) {
        return toks;
    }
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if let Some(used) = try_command(&toks, &c, i, &mut out) {
            i += used;
            continue;
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

/// The command at `i`, emitted onto `out`: tokens consumed.
fn try_command(toks: &[Tok], c: &[String], i: usize, out: &mut Vec<Tok>) -> Option<usize> {
    if toks[i].is_break() {
        return None;
    }
    let next_word = |k: usize| c.get(k).map(String::as_str);
    // ---- quote unquote X -------------------------------------------------
    let qu = if c[i] == "quote-unquote" {
        Some(1)
    } else if c[i] == "quote" && next_word(i + 1) == Some("unquote") {
        Some(2)
    } else {
        None
    };
    if let Some(n) = qu {
        let from = i + n;
        if from >= toks.len() || toks[from].is_break() || c[from].is_empty() {
            return None;
        }
        // After a pause the quotation runs to the next one; else one word.
        let end = if toks[from - 1].suffix().contains(',') {
            pause_end(toks, from, 6).unwrap_or(from)
        } else {
            from
        };
        let (_, tail) = split_tail(&toks[end]);
        let content = toks[from..=end].to_vec();
        if out.last().is_some_and(|p| p.suffix() == ",") {
            out.last_mut().unwrap().strip_suffix();
        }
        let mut wrapped = Vec::new();
        emit(
            &mut wrapped,
            content,
            Wrap {
                mark: Mark::Quote,
                after: tail,
            },
            next_word(end + 1),
        );
        out.extend(wrapped);
        return Some(end + 1 - i);
    }
    // ---- dash --------------------------------------------------------------
    let dash = if c[i] == "dash" {
        Some(1)
    } else if c[i] == "em" && next_word(i + 1) == Some("dash") {
        Some(2)
    } else {
        None
    };
    if let Some(n) = dash {
        return dash_at(toks, c, i, n, out);
    }
    // ---- paired / pause-wrapped / bare-aside marks -------------------------
    let opener = explicit_opener(c, i)
        .map(|(m, n)| (m, n, true))
        .or_else(|| in_opener(c, i).map(|(m, n)| (m, n, false)))
        .or_else(|| bare_quote(toks, c, i, out).then_some((Mark::Quote, 1, true)))?;
    let (mark, n, pairable) = opener;
    let from = i + n;
    if from >= toks.len() || toks[from].is_break() {
        return None;
    }
    if pairable {
        // A closer within the paragraph.
        let lim = toks.len().min(from + MAX_SPAN);
        for k in from + 1..lim {
            if toks[k].text == "\n\n" {
                break;
            }
            if let Some(cn) = closer_at(c, k, mark) {
                let content = toks[from..k].to_vec();
                if content.iter().all(|t| t.core().is_empty()) {
                    return None;
                }
                let after = toks[k + cn - 1].suffix().to_string();
                emit(out, content, Wrap { mark, after }, next_word(k + cn));
                return Some(k + cn - i);
            }
        }
    }
    // No closer: only between pause commas.
    let wrapped = pause_before(out) && toks[from - 1].suffix() == ",";
    let end = if wrapped {
        pause_end(toks, from, MAX_OPEN_SPAN)?
    } else if mark != Mark::Quote && toks[from - 1].suffix().is_empty() && c[i] == "in" {
        bare_aside(toks, c, out, from)?
    } else {
        return None;
    };
    let (_, tail) = split_tail(&toks[end]);
    let content = toks[from..=end].to_vec();
    emit(out, content, Wrap { mark, after: tail }, next_word(end + 1));
    Some(end + 1 - i)
}

/// A bare "quote" that can open a quotation: not a noun ("a quote") or a
/// verb ("quote me").
fn bare_quote(toks: &[Tok], c: &[String], i: usize, out: &[Tok]) -> bool {
    if c[i] != "quote" {
        return false;
    }
    let noun = out.last().is_some_and(|p| {
        !p.is_break() && p.suffix().is_empty() && QUOTE_NOUN_BEFORE.contains(&p.core().as_str())
    });
    let literal_after = toks[i].suffix().is_empty()
        && c.get(i + 1)
            .is_some_and(|n| QUOTE_LITERAL_AFTER.contains(&n.as_str()));
    !noun && !literal_after
}

fn is_numberish(w: &str) -> bool {
    number_word(w).is_some() || w.chars().next().is_some_and(|c| c.is_ascii_digit())
}

fn dash_at(toks: &[Tok], c: &[String], i: usize, n: usize, out: &mut Vec<Tok>) -> Option<usize> {
    let prev = out.last()?;
    let next = toks.get(i + n)?;
    let nc = c[i + n].as_str();
    let pc = prev.core();
    // The closing dash of a pair may follow a determiner: "the plan — if
    // you can call it that — is ready".
    let open_dash = out
        .iter()
        .rev()
        .take_while(|t| !t.ends_sentence())
        .any(|t| t.text == "\u{2014}");
    let blocked = prev.is_break()
        || prev.ends_sentence()
        || pc.is_empty()
        || next.is_break()
        || nc.is_empty()
        || (!open_dash && prev.suffix().is_empty() && DASH_BLOCK_BEFORE.contains(&pc.as_str()))
        || DASH_BLOCK_AFTER.contains(&nc)
        || is_numberish(&pc)
        || is_numberish(nc)
        // "dash" itself ends the sentence: nothing to join.
        || toks[i + n - 1].ends_sentence();
    if blocked {
        return None;
    }
    if prev.suffix() == "," {
        out.last_mut().unwrap().strip_suffix();
    }
    out.push(Tok::new("\u{2014}"));
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::super::spoken_cleanup as t;

    #[test]
    fn closed_quote_pairs_convert_exactly() {
        assert_eq!(
            t("She said, quote, I'll be there soon, unquote, and then never showed up."),
            "She said, \"I'll be there soon,\" and then never showed up."
        );
        assert_eq!(
            t("The client wrote, open quote, we need this by Friday, close quote, so let's prioritize it."),
            "The client wrote, \"We need this by Friday,\" so let's prioritize it."
        );
        assert_eq!(
            t("Can you change the button text to, quote, Get started, end quote?"),
            "Can you change the button text to \"Get started\"?"
        );
        assert_eq!(
            t("Quote, the only way out is through, unquote. Robert Frost."),
            "\"The only way out is through.\" Robert Frost."
        );
        assert_eq!(
            t("The error message says, quote, file not found, unquote."),
            "The error message says, \"File not found.\""
        );
        // Unpunctuated ASR.
        assert_eq!(
            t("he said quote I'll be there unquote and left"),
            "He said \"I'll be there\" and left"
        );
        assert_eq!(
            t("the sign says begin quote no parking end of quote"),
            "The sign says \"no parking\""
        );
    }

    #[test]
    fn quotation_keeps_its_own_question_mark() {
        assert_eq!(
            t("He asked, quote, are you okay?, unquote."),
            "He asked, \"Are you okay?\""
        );
    }

    #[test]
    fn quote_unquote_quotes_the_next_word_or_pause() {
        assert_eq!(
            t("He called it, quote, unquote, innovative."),
            "He called it \"innovative.\""
        );
        assert_eq!(t("It was, quote unquote, fine."), "It was \"fine.\"");
        assert_eq!(
            t("he is a quote unquote expert on this"),
            "He is a \"expert\" on this"
        );
    }

    #[test]
    fn unclosed_quote_needs_pause_commas() {
        assert_eq!(
            t("He literally just said, quote, whatever."),
            "He literally just said \"whatever.\""
        );
        assert_eq!(
            t("She was, in quotes, busy, all weekend."),
            "She was \"busy\" all weekend."
        );
        // No pause commas, no closer: as spoken.
        for s in [
            "He said quote whatever.",
            "Quote me on that.",
            "The word quote comes from Latin.",
            "Can you send me a quote for the new roof?",
            "Can you get me a quote by end of day?",
            "Put it in quotes.",
            "That was a famous quote, unquote.",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn closed_paren_and_bracket_pairs() {
        assert_eq!(
            t("The deadline, open paren, which is flexible, close paren, is Friday."),
            "The deadline (which is flexible) is Friday."
        );
        assert_eq!(
            t("The new build, open parenthesis, version two point three, close parenthesis, is live."),
            "The new build (version 2.3) is live."
        );
        assert_eq!(
            t("see the appendix open bracket page four close bracket for details"),
            "See the appendix [page four] for details"
        );
        assert_eq!(
            t("It costs a lot, open paren, more than I hoped, close paren."),
            "It costs a lot (more than I hoped)."
        );
    }

    #[test]
    fn pause_wrapped_parentheses_and_brackets() {
        assert_eq!(
            t("I'll bring the charger, in parentheses, the USB-C one, to the meeting."),
            "I'll bring the charger (the USB-C one) to the meeting."
        );
        assert_eq!(
            t("Add the new field, in brackets, optional, to the form."),
            "Add the new field [optional] to the form."
        );
        assert_eq!(
            t("Grab the blue folder, in parentheses, the one on my desk."),
            "Grab the blue folder (the one on my desk)."
        );
        assert_eq!(
            t("Call Sam, open paren, my brother, tonight."),
            "Call Sam (my brother) tonight."
        );
    }

    #[test]
    fn the_users_own_example_without_pauses() {
        assert_eq!(
            t("I'll bring the charger in parentheses the USB-C one to the meeting."),
            "I'll bring the charger (the USB-C one) to the meeting."
        );
    }

    #[test]
    fn literal_parentheses_and_brackets_stay_words() {
        for s in [
            "The formula is in parentheses on the board.",
            "Put the area code in parentheses.",
            "The answer is in brackets at the bottom of the page.",
            "Write the year in parentheses after the name.",
            "Put the number in parentheses the way I showed you.",
            "Tax brackets went up this year.",
            "Close the parentheses at the end.",
            "We are in brackets, sadly, again.",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn spoken_dash_is_a_spaced_em_dash() {
        assert_eq!(
            t("The deadline dash which nobody likes dash is Friday."),
            "The deadline \u{2014} which nobody likes \u{2014} is Friday."
        );
        assert_eq!(
            t("The plan dash if you can call it that dash is ready."),
            "The plan \u{2014} if you can call it that \u{2014} is ready."
        );
        assert_eq!(t("It worked, dash, mostly."), "It worked \u{2014} mostly.");
        assert_eq!(
            t("one more thing em dash bring snacks"),
            "One more thing \u{2014} bring snacks"
        );
    }

    #[test]
    fn literal_dash_stays_a_word() {
        for s in [
            "Add a dash of salt.",
            "I'll dash out to the store.",
            "The dash cam caught it.",
            "Make a mad dash for the exit.",
            "Check the dash.",
            "Dash is my dog's name.",
            "Call five dash six.",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn idempotent() {
        for s in [
            "She said, quote, I'll be there soon, unquote, and then never showed up.",
            "I'll bring the charger in parentheses the USB-C one to the meeting.",
            "The deadline dash which nobody likes dash is Friday.",
        ] {
            let once = t(s);
            assert_eq!(t(&once), once, "{s}");
        }
    }
}
