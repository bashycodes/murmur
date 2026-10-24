//! Deterministic "spoken cleanup" pass (clone extension). It does the parts of
//! cleanup that local LLMs mostly got right and that rules do reliably
//! (docs/model-benchmarks.md, "The 30–60% of fixes …"):
//!
//! 1. spoken punctuation: "comma", "period", "question mark", "new line", …
//! 2. fillers: um/uh/er/hmm; ", like," / ", you know," between commas;
//!    a "like" between a form of "be" and a number; a leading "so"/"okay so"
//! 3. stutters: "I I think" → "I think" (not "that that" / "had had")
//! 4. explicit backtracking: "scratch that", "never mind", "no wait",
//!    "actually I mean", … keep the correction, drop what it replaced
//! 5. inverse text normalisation for money, percentages, clock times and
//!    dates: "forty dollars" → "$40", "three thirty pm" → "3:30 PM",
//!    "March fifth" → "March 5th"
//! 6. spoken formatting commands: "quote … unquote", "in parentheses",
//!    "open bracket … close bracket", "dash" (`spoken_commands.rs`)
//! 7. sentence capitalisation and spacing around punctuation
//!
//! Everything is conservative: a phrase that is probably literal ("the
//! period of", "don't forget that", "there's no wait") is left alone, and
//! clean text comes back unchanged. Gated by `SessionConfig.spoken_cleanup`.

use std::sync::LazyLock;

use regex::Regex;

use crate::config::AppCategory;

#[path = "spoken_commands.rs"]
mod commands;

const FILLERS: &[&str] = &[
    "um", "umm", "uh", "uhh", "uhm", "erm", "er", "hmm", "hm", "mm", "mhm",
];

/// Words after which a punctuation word is a noun, not a command
/// ("the period", "a comma", "Oxford comma", "trial period").
const PUNCT_NOUN_BEFORE: &[&str] = &[
    "a",
    "an",
    "the",
    "this",
    "that",
    "each",
    "every",
    "one",
    "per",
    "oxford",
    "serial",
    "trial",
    "grace",
    "waiting",
    "cooling",
    "probation",
    "probationary",
    "reporting",
    "billing",
    "time",
    "test",
    "same",
    "first",
    "last",
    "next",
    "whole",
    "entire",
    "long",
    "short",
    "any",
    "no",
    "his",
    "her",
    "my",
    "your",
    "our",
    "their",
    "its",
    "of",
    "new",
    "extra",
    "missing",
];

/// (spoken words, symbol). Longest first.
const SPOKEN_PUNCT: &[(&[&str], &str)] = &[
    (&["new", "paragraph"], "\n\n"),
    (&["new", "line"], "\n"),
    (&["question", "mark"], "?"),
    (&["exclamation", "mark"], "!"),
    (&["exclamation", "point"], "!"),
    (&["full", "stop"], "."),
    (&["period"], "."),
    (&["comma"], ","),
    (&["semicolon"], ";"),
    (&["semi", "colon"], ";"),
    (&["colon"], ":"),
];

/// Retractions: drop the clause before. Corrections: replace what the
/// correction lines up with.
const RETRACT: &[&[&str]] = &[
    &["scratch", "that"],
    &["strike", "that"],
    &["delete", "that"],
    &["forget", "that"],
    &["cancel", "that"],
    &["never", "mind"],
    &["nevermind"],
];
const CORRECT: &[&[&str]] = &[
    &["actually", "i", "mean"],
    &["sorry", "i", "mean"],
    &["no", "wait"],
    &["wait", "no"],
    &["or", "rather"],
];
/// A retraction verb after these is literal ("don't forget that …").
const LITERAL_BEFORE_VERB: &[&str] = &[
    "don't", "dont", "do", "not", "never", "can't", "cannot", "to", "will", "won't", "please", "i",
    "you", "we", "they", "he", "she", "just", "didn't", "doesn't", "should", "could", "would",
    "must", "might", "let's", "lets", "and", "or", "can",
];
/// "no wait" after these is literal ("there's no wait").
const LITERAL_BEFORE_NO: &[&str] = &[
    "there's",
    "theres",
    "is",
    "was",
    "be",
    "a",
    "the",
    "with",
    "without",
    "had",
    "have",
    "has",
    "there",
    "means",
    "almost",
    "basically",
];
const BE: &[&str] = &[
    "is", "was", "are", "were", "be", "been", "it's", "that's", "its", "thats", "about",
];
/// Doubled words that are often grammatical.
const KEEP_DOUBLE: &[&str] = &[
    "that", "had", "is", "do", "very", "really", "so", "no", "bye", "ha",
];

const MONTHS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

#[derive(Debug, Clone, PartialEq)]
struct Tok {
    text: String,
    /// Capitalise the first letter when rendering (sentence start).
    cap: bool,
}

impl Tok {
    fn new(s: &str) -> Self {
        Tok {
            text: s.to_string(),
            cap: false,
        }
    }
    fn is_break(&self) -> bool {
        self.text.starts_with('\n')
    }
    /// Lowercase word without surrounding punctuation.
    fn core(&self) -> String {
        self.text
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '\u{2019}')
            .trim_matches(['\'', '\u{2019}'])
            .replace('\u{2019}', "'")
            .to_lowercase()
    }
    fn suffix(&self) -> &str {
        let end = self
            .text
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .len();
        &self.text[end..]
    }
    fn ends_sentence(&self) -> bool {
        self.is_break() || self.suffix().contains(['.', '?', '!'])
    }
    fn ends_clause(&self) -> bool {
        self.suffix().contains([',', ';', ':'])
    }
    fn strip_suffix(&mut self) {
        let end = self
            .text
            .trim_end_matches(|c: char| !c.is_alphanumeric())
            .len();
        // Keep closing quotes/brackets; drop sentence/clause punctuation.
        let tail: String = self.text[end..]
            .chars()
            .filter(|c| !matches!(c, ',' | '.' | ';' | ':' | '?' | '!'))
            .collect();
        self.text.truncate(end);
        self.text.push_str(&tail);
    }
}

fn tokenize(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut nl = 0;
    let flush_word = |w: &mut String, out: &mut Vec<Tok>| {
        if !w.is_empty() {
            out.push(Tok::new(w));
            w.clear();
        }
    };
    for c in s.chars() {
        if c == '\n' {
            flush_word(&mut word, &mut out);
            nl += 1;
        } else {
            if nl > 0 {
                out.push(Tok::new(if nl > 1 { "\n\n" } else { "\n" }));
                nl = 0;
            }
            if c.is_whitespace() {
                flush_word(&mut word, &mut out);
            } else {
                word.push(c);
            }
        }
    }
    flush_word(&mut word, &mut out);
    if nl > 0 {
        out.push(Tok::new(if nl > 1 { "\n\n" } else { "\n" }));
    }
    out
}

fn render(toks: &[Tok]) -> String {
    let mut s = String::new();
    let mut cap_next = true;
    for t in toks {
        if t.is_break() {
            while s.ends_with(' ') {
                s.pop();
            }
            s.push_str(&t.text);
            cap_next = true;
            continue;
        }
        if t.text.is_empty() {
            continue;
        }
        if !s.is_empty() && !s.ends_with('\n') {
            s.push(' ');
        }
        if cap_next || t.cap {
            s.push_str(&capitalise(&t.text));
        } else {
            s.push_str(&t.text);
        }
        cap_next = t.ends_sentence();
    }
    s
}

fn capitalise(w: &str) -> String {
    let mut out = String::with_capacity(w.len());
    let mut done = false;
    for c in w.chars() {
        if !done && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            done = true;
        } else {
            out.push(c);
        }
    }
    out
}

fn cores(toks: &[Tok]) -> Vec<String> {
    toks.iter().map(Tok::core).collect()
}

fn matches_at(c: &[String], i: usize, pat: &[&str]) -> bool {
    i + pat.len() <= c.len() && pat.iter().enumerate().all(|(k, p)| c[i + k] == *p)
}

/// 1. Spoken punctuation.
fn spoken_punctuation(toks: Vec<Tok>) -> Vec<Tok> {
    let c = cores(&toks);
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let mut i = 0;
    'outer: while i < toks.len() {
        for (pat, sym) in SPOKEN_PUNCT {
            if !matches_at(&c, i, pat) {
                continue;
            }
            let prev = out.last();
            let noun_use = prev.is_some_and(|p| {
                !p.is_break()
                    && p.suffix().is_empty()
                    && PUNCT_NOUN_BEFORE.contains(&p.core().as_str())
            });
            // A sentence-initial "Period." or "Comma" with nothing before
            // is not a command either (except line breaks).
            let nothing_before = prev.is_none_or(|p| p.is_break());
            if noun_use || (nothing_before && !sym.starts_with('\n')) {
                continue;
            }
            if sym.starts_with('\n') {
                out.push(Tok::new(sym));
            } else if let Some(p) = out.last_mut() {
                p.strip_suffix();
                p.text.push_str(sym);
            }
            i += pat.len();
            continue 'outer;
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

/// 2. Fillers and a leading "so".
fn fillers(toks: Vec<Tok>) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let c = cores(&toks);
    let mut i = 0;
    let mut removed_at_start = false;
    while i < toks.len() {
        let t = &toks[i];
        let at_start =
            out.iter().all(|t| t.text.is_empty()) || out.last().is_some_and(Tok::is_break);
        if FILLERS.contains(&c[i].as_str()) && !t.is_break() {
            // Move sentence-final punctuation to the word before.
            let suf: String = t
                .suffix()
                .chars()
                .filter(|c| matches!(c, '.' | '?' | '!'))
                .collect();
            if !suf.is_empty() {
                if let Some(p) = out
                    .last_mut()
                    .filter(|p| !p.is_break() && !p.ends_sentence())
                {
                    p.strip_suffix();
                    p.text.push_str(&suf);
                }
            }
            // "wanted to, uh, check in": the pause comma before the filler
            // goes with it when the word before cannot end a clause.
            // Another comma-wrapped filler next ("to, um, you know, say")
            // takes the comma instead.
            let more = c.get(i + 1).is_some_and(|n| {
                FILLERS.contains(&n.as_str())
                    || n == "like"
                    || (n == "you" && c.get(i + 2).is_some_and(|k| k == "know"))
            });
            if t.suffix() == "," && !more {
                if let Some(p) = out
                    .last_mut()
                    .filter(|p| p.suffix() == "," && NO_COMMA_AFTER.contains(&p.core().as_str()))
                {
                    p.strip_suffix();
                }
            }
            if at_start {
                removed_at_start = true;
            }
            i += 1;
            continue;
        }
        // ", you know," and ", like," between commas.
        let prev_comma = out.last().is_some_and(|p| p.suffix() == ",");
        let between = |len: usize| prev_comma && toks[i + len - 1].suffix() == ",";
        if c[i] == "you" && i + 1 < toks.len() && c[i + 1] == "know" {
            if between(2) {
                out.last_mut().unwrap().strip_suffix();
                i += 2;
                continue;
            }
            if at_start && removed_at_start && toks[i + 1].suffix() == "," {
                i += 2;
                continue;
            }
        }
        // "So, like, the build …": a leading "like," after a removed "so".
        if c[i] == "like" && (between(1) || (at_start && removed_at_start && t.suffix() == ",")) {
            if let Some(p) = out.last_mut().filter(|p| !p.is_break()) {
                p.strip_suffix();
            }
            i += 1;
            continue;
        }
        // "was like forty dollars": hedge before a quantity.
        if c[i] == "like"
            && t.suffix().is_empty()
            && out
                .last()
                .is_some_and(|p| BE.contains(&p.core().as_str()) && p.suffix().is_empty())
            && c.get(i + 1).is_some_and(|n| is_number_start(n))
        {
            i += 1;
            continue;
        }
        // Leading "okay so" / "so," / filler + "so".
        if at_start
            && c[i] == "okay"
            && c.get(i + 1).is_some_and(|n| n == "so")
            && t.suffix().is_empty()
        {
            removed_at_start = true;
            i += 1;
            continue;
        }
        if at_start && c[i] == "so" && (t.suffix() == "," || removed_at_start) {
            removed_at_start = true;
            i += 1;
            continue;
        }
        out.push(t.clone());
        i += 1;
    }
    out
}

/// Words a comma cannot follow inside a clause ("to,", "were,"): a pause
/// comma left after one of them by a removed filler is dropped.
const NO_COMMA_AFTER: &[&str] = &[
    "a", "an", "the", "to", "of", "for", "with", "at", "in", "on", "by", "from", "into", "is",
    "are", "was", "were", "be", "been", "am", "my", "your", "his", "her", "our", "their", "and",
    "or", "but", "will", "would", "could", "should", "can", "have", "has", "had", "do", "does",
    "did", "just", "really", "very", "about", "like", "i", "we", "you", "they", "he", "she",
];

fn is_number_start(w: &str) -> bool {
    w.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '$')
        || number_word(w).is_some()
        || w == "a"
}

/// 3. Stutters.
fn stutters(toks: Vec<Tok>) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    for t in toks {
        if let Some(p) = out.last() {
            let (a, b) = (p.core(), t.core());
            if !a.is_empty()
                && a == b
                && p.suffix().is_empty()
                && a.chars().all(char::is_alphabetic)
                && !KEEP_DOUBLE.contains(&a.as_str())
                // "five five five" is a phone number, not a stutter.
                && number_word(&a).is_none()
                && a != "oh"
            {
                out.pop();
            }
        }
        out.push(t);
    }
    out
}

/// 4. Backtracking. Resolves the first trigger, then repeats.
fn backtrack(mut toks: Vec<Tok>) -> Vec<Tok> {
    for _ in 0..8 {
        match backtrack_once(&toks) {
            Some(t) => toks = t,
            None => break,
        }
    }
    toks
}

fn find_trigger(toks: &[Tok]) -> Option<(usize, usize, bool)> {
    let c = cores(toks);
    for i in 1..c.len() {
        let prev = &c[i - 1];
        let prev_tok = &toks[i - 1];
        for pat in RETRACT {
            if matches_at(&c, i, pat) {
                let literal = pat.len() == 2
                    && pat[1] == "that"
                    && prev_tok.suffix().is_empty()
                    && LITERAL_BEFORE_VERB.contains(&prev.as_str());
                if !literal {
                    return Some((i, pat.len(), true));
                }
            }
        }
        for pat in CORRECT {
            if matches_at(&c, i, pat) {
                let literal = pat[0] == "no"
                    && prev_tok.suffix().is_empty()
                    && LITERAL_BEFORE_NO.contains(&prev.as_str());
                if !literal {
                    return Some((i, pat.len(), false));
                }
            }
        }
    }
    None
}

fn backtrack_once(toks: &[Tok]) -> Option<Vec<Tok>> {
    let (i, n, retract) = find_trigger(toks)?;
    let left = &toks[..i];
    let mut right: Vec<Tok> = toks[i + n..].to_vec();
    // The span the trigger can refer to: the current sentence, or the
    // previous one when the trigger opens a sentence.
    let sentence_start = |end: usize| -> usize {
        (0..end)
            .rev()
            .find(|&k| left[k].ends_sentence())
            .map_or(0, |k| k + 1)
    };
    let opens_sentence = left.last().is_some_and(Tok::ends_sentence);
    let s = if opens_sentence {
        sentence_start(left.len() - 1)
    } else {
        sentence_start(left.len())
    };
    let scope = &left[s..];
    let first = right.iter().find(|t| !t.is_break()).map(Tok::core);
    let mut keep: Vec<Tok> = left[..s].to_vec();
    let mut new_sentence = false;
    let anchor = first
        .as_ref()
        .filter(|w| !w.is_empty())
        .and_then(|w| scope.iter().rposition(|t| t.core() == *w));
    match anchor {
        Some(j) => keep.extend_from_slice(&scope[..j]),
        None if retract => {
            // Drop back to the clause boundary before the retracted clause.
            let k = scope[..scope.len().saturating_sub(1)]
                .iter()
                .rposition(Tok::ends_clause);
            if let Some(k) = k {
                keep.extend_from_slice(&scope[..=k]);
                new_sentence = right.first().is_some_and(|t| {
                    let w = t.text.trim_start_matches(|c: char| !c.is_alphanumeric());
                    w.chars().next().is_some_and(char::is_uppercase) && t.core() != "i"
                });
            }
        }
        None => {
            if right.is_empty() {
                keep.extend_from_slice(scope);
            } else {
                keep.extend_from_slice(&scope[..scope.len().saturating_sub(1)]);
            }
        }
    }
    // Join: the kept text loses its dangling comma; the correction takes
    // over the sentence ending.
    if let Some(last) = keep.last_mut().filter(|t| !t.is_break()) {
        if last.ends_clause() {
            last.strip_suffix();
        }
        if new_sentence {
            last.text.push('.');
        }
    }
    if right.is_empty() {
        // Trigger at the very end: the trigger's own ending applies.
        let end: String = toks[i + n - 1]
            .suffix()
            .chars()
            .filter(|c| matches!(c, '.' | '?' | '!'))
            .collect();
        if let Some(last) = keep.last_mut().filter(|t| !t.is_break()) {
            if !end.is_empty() && !last.ends_sentence() {
                last.strip_suffix();
                last.text.push_str(&end);
            }
        }
    } else if keep.last().is_none_or(Tok::ends_sentence) {
        right[0].cap = true;
    }
    keep.extend(right);
    Some(keep)
}

// ---- 5. Inverse text normalisation -------------------------------------

fn unit(w: &str) -> Option<u64> {
    let v = match w {
        "zero" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        _ => return None,
    };
    Some(v)
}

fn tens(w: &str) -> Option<u64> {
    let v = match w {
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    };
    Some(v)
}

fn number_word(w: &str) -> Option<u64> {
    unit(w).or_else(|| tens(w)).or_else(|| {
        let (a, b) = w.split_once('-')?;
        Some(tens(a)? + unit(b).filter(|&u| (1..10).contains(&u))?)
    })
}

fn ordinal_word(w: &str) -> Option<u64> {
    let v = match w {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "thirteenth" => 13,
        "fourteenth" => 14,
        "fifteenth" => 15,
        "sixteenth" => 16,
        "seventeenth" => 17,
        "eighteenth" => 18,
        "nineteenth" => 19,
        "twentieth" => 20,
        "thirtieth" => 30,
        _ => {
            let (a, b) = w.split_once('-')?;
            return Some(tens(a)? + ordinal_word(b).filter(|&u| u < 10)?);
        }
    };
    Some(v)
}

fn ordinal_suffix(n: u64) -> &'static str {
    match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

/// A cardinal number starting at `i`: (value, tokens used). Accepts digits,
/// "one hundred and twenty", "a hundred", "twenty five", "two thousand".
/// Stops after a token with punctuation.
fn parse_number(c: &[String], toks: &[Tok], i: usize) -> Option<(u64, usize)> {
    if let Ok(v) = c.get(i)?.replace(',', "").parse::<u64>() {
        return Some((v, 1));
    }
    let mut total = 0u64;
    let mut cur = 0u64;
    let mut j = i;
    let mut any = false;
    while j < c.len() {
        let w = c[j].as_str();
        let last_ok = j;
        if let Some(v) = number_word(w) {
            // "twenty" "five" composes; "five" "five" does not.
            if !cur.is_multiple_of(100) && (v >= 10 || !cur.is_multiple_of(10)) {
                break;
            }
            if any && cur != 0 && cur < 10 {
                break;
            }
            cur += v;
            any = true;
        } else if w == "a"
            && !any
            && c.get(j + 1)
                .is_some_and(|n| n == "hundred" || n == "thousand")
        {
            cur = 1;
        } else if w == "hundred" && (any || cur == 1) {
            cur = cur.max(1) * 100;
            any = true;
        } else if w == "thousand" && (any || cur == 1) {
            total += cur.max(1) * 1000;
            cur = 0;
            any = true;
        } else if w == "and"
            && any
            && c.get(j + 1).is_some_and(|n| number_word(n).is_some())
            && cur >= 100
        {
            // "one hundred and twenty"
        } else {
            break;
        }
        j += 1;
        if !toks[last_ok].suffix().is_empty() {
            break;
        }
    }
    any.then_some((total + cur, j - i))
}

fn last_suffix(toks: &[Tok], i: usize, n: usize) -> String {
    toks[i + n - 1].suffix().to_string()
}

fn itn(toks: Vec<Tok>) -> Vec<Tok> {
    let c = cores(&toks);
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if let Some((t, used)) = phone(&c, &toks, i)
            .or_else(|| money(&c, &toks, i))
            .or_else(|| percent(&c, &toks, i))
            .or_else(|| clock(&c, &toks, i))
            .or_else(|| date(&c, &toks, i))
            .or_else(|| date_of(&c, &toks, i))
            .or_else(|| decimal(&c, &toks, i))
            .or_else(|| cardinal(&c, &toks, i))
        {
            // An opening mark from a spoken command stays: "(forty dollars".
            let mut t = Tok::new(&format!("{}{t}", lead_marks(&toks[i].text)));
            t.cap = toks[i].cap;
            out.push(t);
            i += used;
            continue;
        }
        out.push(toks[i].clone());
        i += 1;
    }
    out
}

/// Opening quotes/brackets at the start of a token.
fn lead_marks(s: &str) -> &str {
    let n = s
        .find(|c: char| !matches!(c, '"' | '\'' | '(' | '[' | '{' | '\u{201c}' | '\u{2018}'))
        .unwrap_or(s.len());
    &s[..n]
}

/// Single digit word: "zero".."nine", and "oh" inside a digit string.
fn digit_word(w: &str, inside: bool) -> Option<u64> {
    if inside && w == "oh" {
        return Some(0);
    }
    unit(w).filter(|&d| d < 10)
}

/// "five five five, one two three, four five six seven" → "555-123-4567";
/// seven digits → "123-4567". Pause commas (or a spoken "dash") between
/// groups must split them 3-3-4 / 3-4.
fn phone(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    digit_word(&c[i], false)?;
    let mut groups: Vec<String> = vec![String::new()];
    let mut j = i;
    let mut last = i;
    while j < toks.len() && !toks[j].is_break() {
        let inside = groups.iter().any(|g| !g.is_empty());
        if c[j] == "dash" && inside && toks[j - 1].suffix().is_empty() {
            groups.push(String::new());
            j += 1;
            continue;
        }
        let Some(d) = digit_word(&c[j], inside) else {
            break;
        };
        groups.last_mut().unwrap().push(char::from(b'0' + d as u8));
        last = j;
        j += 1;
        match toks[last].suffix() {
            "" => {}
            "," => groups.push(String::new()),
            _ => break,
        }
    }
    groups.retain(|g| !g.is_empty());
    let digits: String = groups.concat();
    let sizes: Vec<usize> = groups.iter().map(String::len).collect();
    let body = match (digits.len(), sizes.len()) {
        (10, 1) => format!("{}-{}-{}", &digits[..3], &digits[3..6], &digits[6..]),
        (7, 1) => format!("{}-{}", &digits[..3], &digits[3..]),
        _ if sizes == [3, 3, 4] || sizes == [3, 4] => groups.join("-"),
        _ => return None,
    };
    Some((format!("{body}{}", toks[last].suffix()), last + 1 - i))
}

/// "two point three" → "2.3" (not "at one point").
fn decimal(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    if !toks[i].text.chars().any(char::is_alphabetic) {
        return None;
    }
    let (v, n) = parse_number(c, toks, i)?;
    if !toks[i + n - 1].suffix().is_empty() || c.get(i + n)? != "point" {
        return None;
    }
    if v == 1
        && i > 0
        && matches!(
            c[i - 1].as_str(),
            "at" | "one" | "some" | "the" | "a" | "that" | "this" | "any" | "each"
        )
    {
        return None;
    }
    let mut j = i + n + 1;
    let mut frac = String::new();
    while j < toks.len() && toks[j - 1].suffix().is_empty() {
        let Some(d) = digit_word(&c[j], !frac.is_empty()) else {
            break;
        };
        frac.push(char::from(b'0' + d as u8));
        j += 1;
    }
    if frac.is_empty() || !toks[i + n].suffix().is_empty() {
        return None;
    }
    Some((format!("{v}.{frac}{}", toks[j - 1].suffix()), j - i))
}

/// Words a bare cardinal does not turn into digits before: they make it
/// part of a larger phrase ("ten and", "twenty or so").
const CARDINAL_BLOCK_AFTER: &[&str] = &["and", "or", "o'clock", "am", "pm", "point", "percent"];

/// A spelled cardinal ≥ 10 before a word: "two hundred and fifty users"
/// → "250 users", "ten minutes" → "10 minutes". Numbers below ten stay
/// words ("five apples", AP style), and so does any number next to another
/// number word ("eight thirty", "nineteen eighty four") or at the end of a
/// phrase.
fn cardinal(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    if !toks[i].text.chars().any(char::is_alphabetic) {
        return None;
    }
    // "a hundred" stays a phrase.
    if c[i] == "a" || (i > 0 && (number_word(&c[i - 1]).is_some() || c[i - 1] == "point")) {
        return None;
    }
    // "ten and twelve", "five to fifteen": both stay words.
    if i > 1 && matches!(c[i - 1].as_str(), "and" | "or" | "to") && number_word(&c[i - 2]).is_some()
    {
        return None;
    }
    let (v, n) = parse_number(c, toks, i)?;
    if v < 10 || !toks[i + n - 1].suffix().is_empty() || toks[i + n - 1].is_break() {
        return None;
    }
    let next = c.get(i + n)?;
    let word = next.chars().next().is_some_and(char::is_alphabetic);
    if !word
        || toks[i + n].is_break()
        || number_word(next).is_some()
        || ordinal_word(next).is_some()
        || matches!(next.as_str(), "hundred" | "thousand")
        || CARDINAL_BLOCK_AFTER.contains(&next.as_str())
    {
        return None;
    }
    Some((v.to_string(), n))
}

fn money(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    let (v, n) = parse_number(c, toks, i)?;
    if !toks[i + n - 1].suffix().is_empty() {
        return None;
    }
    let unit = c.get(i + n)?;
    if !matches!(unit.as_str(), "dollars" | "dollar" | "bucks") {
        return None;
    }
    let mut used = n + 1;
    let mut cents = None;
    if toks[i + n].suffix().is_empty() && c.get(i + used).is_some_and(|w| w == "and") {
        if let Some((cv, cn)) = parse_number(c, toks, i + used + 1) {
            if cv < 100 && c.get(i + used + 1 + cn).is_some_and(|w| w == "cents") {
                cents = Some(cv);
                used += 1 + cn + 1;
            }
        }
    }
    let body = match cents {
        Some(ct) => format!("${v}.{ct:02}"),
        None => format!("${v}"),
    };
    Some((format!("{body}{}", last_suffix(toks, i, used)), used))
}

fn percent(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    let (v, n) = parse_number(c, toks, i)?;
    if !toks[i + n - 1].suffix().is_empty() || c.get(i + n)? != "percent" {
        return None;
    }
    Some((format!("{v}%{}", last_suffix(toks, i, n + 1)), n + 1))
}

/// "am"/"pm"/"a.m."/"p.m." at `j` (one token, or "a" "m" split): (AM|PM, tokens).
fn meridiem(c: &[String], toks: &[Tok], j: usize) -> Option<(&'static str, usize)> {
    let w = c.get(j)?;
    let raw = toks[j].text.to_lowercase();
    let m = if w == "pm" || raw.starts_with("p.m") {
        "PM"
    } else if w == "am"
        && (toks[j].text.contains('.')
            || toks[j].text.to_lowercase() == toks[j].text
            || raw == "am")
    {
        // "am" is also the verb ("I am"); callers only ask after an hour.
        "AM"
    } else {
        return None;
    };
    Some((m, 1))
}

fn clock(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    // Digit forms: "3pm", "4pm.", "3:30pm", "3 pm", "3:30 p.m."
    let w = &c[i];
    let digits: String = w
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == ':')
        .collect();
    if !digits.is_empty() && digits.chars().next().is_some_and(|d| d.is_ascii_digit()) {
        let rest = &w[digits.len()..];
        let (h, m) = match digits.split_once(':') {
            Some((h, m)) => (h.parse::<u64>().ok()?, Some(m.to_string())),
            None => (digits.parse::<u64>().ok()?, None),
        };
        if !(1..=12).contains(&h) {
            return None;
        }
        let (mer, used) = match rest {
            "pm" | "p.m" => ("PM", 1),
            "am" | "a.m" => ("AM", 1),
            "" if toks[i].suffix().is_empty() => {
                let (m, n) = meridiem(c, toks, i + 1)?;
                (m, 1 + n)
            }
            _ => return None,
        };
        let suffix = clock_suffix(toks, i + used - 1);
        let t = match m {
            Some(m) => format!("{h}:{m} {mer}"),
            None => format!("{h} {mer}"),
        };
        return Some((format!("{t}{suffix}"), used));
    }
    // Word forms: "three pm", "three thirty pm", "eight oh five am".
    let h = unit(w).filter(|h| (1..=12).contains(h))?;
    if !toks[i].suffix().is_empty() {
        return None;
    }
    let mut j = i + 1;
    let mut minutes = None;
    if c.get(j).is_some_and(|w| w == "oh" || w == "o") {
        let m = unit(c.get(j + 1)?).filter(|m| (1..10).contains(m))?;
        minutes = Some(m);
        j += 2;
    } else if let Some((m, n)) = parse_number(c, toks, j).filter(|(m, _)| (10..60).contains(m)) {
        if toks[j + n - 1].suffix().is_empty() {
            minutes = Some(m);
            j += n;
        }
    }
    let Some((mer, n)) = meridiem(c, toks, j) else {
        // "seven thirty tonight" → "7:30 tonight": minutes and a day part.
        let m = minutes?;
        if !toks[j - 1].suffix().is_empty() || !day_part(c, j) {
            return None;
        }
        return Some((format!("{h}:{m:02}"), j - i));
    };
    let used = j + n - i;
    let suffix = clock_suffix(toks, i + used - 1);
    let t = match minutes {
        Some(m) => format!("{h}:{m:02} {mer}"),
        None => format!("{h} {mer}"),
    };
    Some((format!("{t}{suffix}"), used))
}

/// "tonight", "tomorrow", "this morning", "in the evening" at `j`.
fn day_part(c: &[String], j: usize) -> bool {
    const PARTS: &[&str] = &["morning", "afternoon", "evening", "night"];
    match c.get(j).map(String::as_str) {
        Some("tonight" | "today" | "tomorrow") => true,
        Some("this") => c.get(j + 1).is_some_and(|w| PARTS.contains(&w.as_str())),
        Some("in") => {
            c.get(j + 1).is_some_and(|w| w == "the")
                && c.get(j + 2).is_some_and(|w| PARTS.contains(&w.as_str()))
        }
        _ => false,
    }
}

/// The punctuation after a meridiem token. The dots of "p.m." are not
/// punctuation, except that a final "p.m." also ends its sentence (last
/// token, or the next word is capitalised).
fn clock_suffix(toks: &[Tok], k: usize) -> String {
    let t = &toks[k];
    let suffix = t.suffix().to_string();
    let lower = t.text.to_lowercase();
    if !(lower.ends_with("a.m.") || lower.ends_with("p.m.")) {
        return suffix;
    }
    let rest: String = suffix.chars().skip_while(|c| *c == '.').collect();
    if !rest.is_empty() {
        return rest;
    }
    let ends = toks
        .get(k + 1)
        .is_none_or(|n| n.is_break() || n.text.chars().next().is_some_and(char::is_uppercase));
    if ends {
        ".".into()
    } else {
        String::new()
    }
}

fn date(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    if !MONTHS.contains(&c[i].as_str()) || !toks[i].suffix().is_empty() {
        return None;
    }
    let j = i + 1;
    let w = c.get(j)?;
    // "twenty first" / "twenty-first" / "fifth" / "the fifth"?
    let (day, n) = if let Some(d) = ordinal_word(w) {
        (d, 1)
    } else if let (Some(t), Some(d)) = (tens(w), c.get(j + 1).and_then(|n| ordinal_word(n))) {
        if d >= 10 || !toks[j].suffix().is_empty() {
            return None;
        }
        (t + d, 2)
    } else if let Some(d) = unit(w).or_else(|| tens(w)) {
        if (1..=31).contains(&d) && w != "may" {
            (d, 1)
        } else {
            return None;
        }
    } else {
        return None;
    };
    if !(1..=31).contains(&day) {
        return None;
    }
    let month = capitalise(&c[i]);
    let suffix = last_suffix(toks, j, n);
    Some((
        format!("{month} {day}{}{suffix}", ordinal_suffix(day)),
        n + 1,
    ))
}

/// "the twenty first of October" → "October 21st" (the "Month Nth"
/// convention of [`date`]).
fn date_of(c: &[String], toks: &[Tok], i: usize) -> Option<(String, usize)> {
    let mut j = i;
    if c[j] == "the" {
        if !toks[j].suffix().is_empty() {
            return None;
        }
        j += 1;
    }
    let w = c.get(j)?;
    let (day, n) = if let Some(d) = ordinal_word(w) {
        (d, 1)
    } else {
        let t = tens(w)?;
        let d = ordinal_word(c.get(j + 1)?).filter(|&d| d < 10)?;
        if !toks[j].suffix().is_empty() {
            return None;
        }
        (t + d, 2)
    };
    j += n;
    if !(1..=31).contains(&day) || !toks[j - 1].suffix().is_empty() || c.get(j)? != "of" {
        return None;
    }
    if !toks[j].suffix().is_empty() || !MONTHS.contains(&c.get(j + 1)?.as_str()) {
        return None;
    }
    let month = capitalise(&c[j + 1]);
    let suffix = toks[j + 1].suffix();
    Some((
        format!("{month} {day}{}{suffix}", ordinal_suffix(day)),
        j + 2 - i,
    ))
}

// ---- driver --------------------------------------------------------------

/// Tidy what the token passes leave: no empty tokens, no punctuation-only
/// tokens at the start, no doubled punctuation.
fn tidy(toks: Vec<Tok>) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    for t in toks {
        if t.text.is_empty() {
            continue;
        }
        // A token that is only punctuation joins the word before it.
        if !t.is_break() && t.text.chars().all(|c| ",.;:?!".contains(c)) {
            if let Some(p) = out
                .last_mut()
                .filter(|p| !p.is_break() && !p.ends_sentence())
            {
                p.strip_suffix();
                p.text.push_str(&t.text);
            }
            continue;
        }
        if t.is_break() && out.last().is_some_and(Tok::is_break) {
            let p = out.last_mut().unwrap();
            if t.text.len() > p.text.len() {
                p.text = t.text;
            }
            continue;
        }
        if t.is_break() && out.is_empty() {
            continue;
        }
        out.push(t);
    }
    while out.last().is_some_and(Tok::is_break) {
        out.pop();
    }
    out
}

/// Run the whole pass. Returns `text` unchanged when nothing applies.
pub fn spoken_cleanup(text: &str) -> String {
    let toks = tokenize(text);
    if toks.is_empty() {
        return String::new();
    }
    let original = toks.clone();
    let toks = spoken_punctuation(toks);
    let toks = fillers(toks);
    let toks = stutters(toks);
    let toks = backtrack(toks);
    let toks = commands::spoken_commands(toks);
    let toks = itn(toks);
    let toks = tidy(toks);
    if toks == original {
        return text.to_string();
    }
    render(&toks)
}

// ---- layout: numbered lists and email structure -------------------------------
//
// The
// list is written the way `cleanup::lists_to_plain` renders the LLM's
// `<ol>`: `1. item` lines.

/// Words after which an ordinal is an adjective ("the first time", "wait a
/// second"), never a list marker.
const ORDINAL_BLOCK: &[&str] = &[
    "the", "a", "an", "my", "your", "his", "her", "its", "our", "their", "this", "that", "these",
    "those", "whose", "which", "what", "at", "in", "on", "of", "for", "every", "each", "very",
    "came", "come", "comes", "is", "was", "were", "are", "be",
];
/// Closes an ordinal list after "first … second …": "and finally …".
/// Verbs a cardinal list can follow without a pause ("the goals are one, …").
const LIST_INTRO: &[&str] = &["are", "were", "include", "includes", "including"];
const FINAL_MARKERS: &[&str] = &["finally", "lastly"];
const JOINERS: &[&str] = &["and", "then"];
/// Longest item, in tokens: a longer "item" means the ordinals were prose.
const MAX_ITEM_TOKENS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MarkerKind {
    Ordinal,
    Cardinal,
}

#[derive(Debug, Clone, Copy)]
struct Marker {
    /// Token range of the marker ("number one", "first of all,").
    start: usize,
    end: usize,
    n: u64,
    kind: MarkerKind,
}

fn at_boundary(toks: &[Tok], i: usize) -> bool {
    i == 0 || {
        let p = &toks[i - 1];
        p.is_break() || p.ends_sentence() || p.ends_clause()
    }
}

/// Start of the separator before token `i`: an unpunctuated "and"/"then"
/// (", and third") belongs to neither item.
fn joiner_start(toks: &[Tok], c: &[String], mut i: usize) -> usize {
    while i > 0
        && JOINERS.contains(&c[i - 1].as_str())
        && toks[i - 1].suffix().is_empty()
        && !toks[i - 1].is_break()
    {
        i -= 1;
    }
    i
}

fn marker_at(toks: &[Tok], c: &[String], i: usize) -> Option<Marker> {
    if toks[i].is_break() {
        return None;
    }
    let w = c[i].as_str();
    let alpha = toks[i].text.chars().next().is_some_and(char::is_alphabetic);
    let ord = ordinal_word(w).or_else(|| ordinal_word(w.strip_suffix("ly")?));
    if let Some(n) = ord.filter(|_| alpha) {
        let blocked =
            i > 0 && ORDINAL_BLOCK.contains(&c[i - 1].as_str()) && toks[i - 1].suffix().is_empty();
        if blocked {
            return None;
        }
        let end = if n == 1 && toks[i].suffix().is_empty() && matches_at(c, i + 1, &["of", "all"]) {
            i + 3
        } else {
            i + 1
        };
        return Some(Marker {
            start: i,
            end,
            n,
            kind: MarkerKind::Ordinal,
        });
    }
    let n = if alpha {
        number_word(w).filter(|_| !w.contains('-'))
    } else {
        w.parse::<u64>().ok()
    }
    .filter(|n| (1..=20).contains(n))?;
    // "number one", "step two": the prefix is part of the marker.
    if i > 0 && matches!(c[i - 1].as_str(), "number" | "step") && toks[i - 1].suffix().is_empty() {
        let start = i - 1;
        // Anywhere but after a determiner ("my number one priority").
        let blocked = start > 0
            && ORDINAL_BLOCK.contains(&c[start - 1].as_str())
            && toks[start - 1].suffix().is_empty();
        return (!blocked).then_some(Marker {
            start,
            end: i + 1,
            n,
            kind: MarkerKind::Cardinal,
        });
    }
    // A bare number only at a clause start and followed by punctuation:
    // "One, clone the repo." / "1. Clone the repo."
    let punct = matches!(toks[i].suffix(), "," | "." | ":" | ")" | ".)");
    // "Our priorities are one, hiring, two, …": the first marker may follow
    // the verb that introduces the list.
    let intro = n == 1
        && i > 0
        && toks[i - 1].suffix().is_empty()
        && LIST_INTRO.contains(&c[i - 1].as_str());
    (punct && (intro || at_boundary(toks, joiner_start(toks, c, i)))).then_some(Marker {
        start: i,
        end: i + 1,
        n,
        kind: MarkerKind::Cardinal,
    })
}

/// Find the first run of ≥ 3 markers numbered 1, 2, 3, … (same kind) in
/// one paragraph, each followed by at least one word.
fn find_list(toks: &[Tok], c: &[String], from: usize) -> Option<Vec<Marker>> {
    let content = |a: usize, b: usize| {
        let words = toks[a..b]
            .iter()
            .filter(|t| !t.is_break() && !t.core().is_empty())
            .count();
        words > 0 && b - a <= MAX_ITEM_TOKENS
    };
    for i in from..toks.len() {
        let Some(first) = marker_at(toks, c, i).filter(|m| m.n == 1) else {
            continue;
        };
        let mut ms = vec![first];
        let mut k = first.end;
        while k < toks.len() {
            let last = *ms.last().unwrap();
            if toks[k].text == "\n\n" {
                break;
            }
            if k - last.end > MAX_ITEM_TOKENS {
                break;
            }
            if let Some(m) = marker_at(toks, c, k) {
                if m.kind == last.kind
                    && m.n == last.n + 1
                    && content(last.end, joiner_start(toks, c, m.start))
                {
                    ms.push(m);
                    k = m.end;
                    continue;
                }
            }
            let fin = last.kind == MarkerKind::Ordinal
                && ms.len() >= 2
                && FINAL_MARKERS.contains(&c[k].as_str())
                && content(last.end, joiner_start(toks, c, k));
            if fin && k + 1 < toks.len() {
                ms.push(Marker {
                    start: k,
                    end: k + 1,
                    n: last.n + 1,
                    kind: MarkerKind::Ordinal,
                });
                break;
            }
            k += 1;
        }
        if ms.len() >= 3 {
            return Some(ms);
        }
    }
    None
}

/// Trim an item: no leading/trailing break, no trailing clause comma.
fn finish_item(mut item: Vec<Tok>) -> Vec<Tok> {
    while item.first().is_some_and(Tok::is_break) {
        item.remove(0);
    }
    while item.last().is_some_and(Tok::is_break) {
        item.pop();
    }
    if let Some(t) = item.last_mut() {
        if t.ends_clause() && !t.ends_sentence() {
            t.strip_suffix();
        }
    }
    item
}

/// Numbered lists: "the steps are, first, clone the repo. Second, run the
/// install script. And third, start the server." → "The steps are:\n\n1.
/// Clone the repo.\n2. Run the install script.\n3. Start the server."
fn lists(toks: Vec<Tok>) -> Vec<Tok> {
    let c = cores(&toks);
    let Some(ms) = find_list(&toks, &c, 0) else {
        return toks;
    };
    let first = ms[0];
    let para = toks[..first.start]
        .iter()
        .rposition(|t| t.text == "\n\n")
        .map_or(0, |p| p + 1);
    let intro_end = joiner_start(&toks, &c, first.start);
    let mut out: Vec<Tok> = toks[..para].to_vec();
    let mut intro = finish_item(toks[para..intro_end].to_vec());
    if let Some(t) = intro.last_mut() {
        if !t.ends_sentence() {
            t.strip_suffix();
            t.text.push(':');
        }
    }
    // Items: up to the next marker's joiner; the last one to the end of
    // its sentence.
    let mut items: Vec<Vec<Tok>> = Vec::new();
    let mut rest_start = toks.len();
    for (k, m) in ms.iter().enumerate() {
        let end = match ms.get(k + 1) {
            Some(next) => joiner_start(&toks, &c, next.start),
            None => {
                let e = (m.end..toks.len())
                    .find(|&j| toks[j].is_break() || toks[j].ends_sentence())
                    .map_or(toks.len(), |j| j + usize::from(!toks[j].is_break()));
                rest_start = e;
                e
            }
        };
        let item = finish_item(toks[m.end..end].to_vec());
        if item.is_empty() {
            return toks;
        }
        items.push(item);
    }
    // Periods: all items get one when any non-last item has one; else the
    // last item's sentence period goes too.
    let dotted = items[..items.len() - 1]
        .iter()
        .any(|it| it.last().is_some_and(|t| t.suffix().ends_with('.')));
    for it in &mut items {
        let t = it.last_mut().unwrap();
        if dotted && !t.ends_sentence() {
            t.text.push('.');
        } else if !dotted && t.suffix() == "." {
            t.strip_suffix();
        }
    }
    if !intro.is_empty() {
        out.extend(intro);
        out.push(Tok::new("\n\n"));
    }
    for (k, it) in items.into_iter().enumerate() {
        if k > 0 {
            out.push(Tok::new("\n"));
        }
        out.push(Tok::new(&format!("{}.", k + 1)));
        out.extend(it);
    }
    let rest = lists(toks[rest_start..].to_vec());
    if rest.iter().any(|t| !t.is_break()) {
        out.push(Tok::new("\n\n"));
        out.extend(rest.into_iter().skip_while(Tok::is_break));
    }
    out
}

static GREETING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        // The name may hold a title abbreviation: "Dear Mr. Patel,".
        r"(?i)^(hi|hello|hey|dear|good (?:morning|afternoon|evening)|greetings)\b((?:\s(?:mr|mrs|ms|mx|dr|prof|st)\.|[^,.!?:;\n])*?)\s*(?:[,.!:]\s*|\n+\s*)(\S[\s\S]*)$",
    )
    .unwrap()
});
static SIGN_OFF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^([\s\S]*?[.!?\n])\s*(best regards|kind regards|warm regards|warmest regards|regards|best wishes|all the best|yours truly|yours sincerely|sincerely|many thanks|thanks so much|thanks again|thank you|thanks|cheers|best|talk soon|take care)(?:[,.!]?\s+((?-i:\p{Lu}[\p{L}'-]*)(?:\s+(?-i:\p{Lu}[\p{L}'-]*)){0,2}))?[.!]?\s*$",
    )
    .unwrap()
});
/// Sign-offs that stand alone without a name ("Best regards.").
const SIGN_OFF_ALONE: &[&str] = &[
    "best regards",
    "kind regards",
    "warm regards",
    "warmest regards",
    "regards",
    "best wishes",
    "all the best",
    "yours truly",
    "yours sincerely",
    "sincerely",
];
/// Capitalised words after "Thanks" that are not a name.
const NOT_A_NAME: &[&str] = &[
    "you",
    "all",
    "everyone",
    "everybody",
    "again",
    "so",
    "much",
    "for",
    "in",
    "a",
    "the",
    "guys",
];

/// Email structure (category email only): a spoken greeting gets its own
/// line ending in a comma, a spoken sign-off its own paragraph with the
/// name on the next line. Nothing is added that was not said.
pub fn email_layout(text: &str) -> String {
    let t = text.trim();
    let (greeting, rest) = match GREETING.captures(t) {
        Some(g) if g[2].split_whitespace().count() <= 3 => (
            Some(capitalise(&format!("{}{}", &g[1], g[2].trim_end())) + ","),
            g.get(3).unwrap().as_str(),
        ),
        _ => (None, t),
    };
    let (body, sign_off) = match SIGN_OFF.captures(rest) {
        Some(s) => {
            let phrase = s[2].to_lowercase();
            let name = s.get(3).map(|m| m.as_str());
            let named = name.is_some_and(|n| {
                let first = n.split_whitespace().next().unwrap_or("").to_lowercase();
                !NOT_A_NAME.contains(&first.as_str())
            });
            // A bare "Thanks." closes the email only after a spoken
            // greeting ("Hey John, … Thanks."); without one it is prose.
            let alone = SIGN_OFF_ALONE.contains(&phrase.as_str()) || greeting.is_some();
            if named || (name.is_none() && alone) {
                let line = capitalise(&s[2]) + ",";
                let block = match name.filter(|_| named) {
                    Some(n) => format!("{line}\n{n}"),
                    None => line,
                };
                (s[1].trim().to_string(), Some(block))
            } else {
                (rest.to_string(), None)
            }
        }
        None => (rest.to_string(), None),
    };
    if greeting.is_none() && sign_off.is_none() {
        return text.to_string();
    }
    let body = capitalise(body.trim());
    [greeting, Some(body).filter(|b| !b.is_empty()), sign_off]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Layout pass after [`spoken_cleanup`]: numbered lists (not in
/// terminals) and, in email apps, greeting / sign-off structure. Returns
/// `text` unchanged when neither applies.
pub fn layout(text: &str, category: AppCategory) -> String {
    let mut out = text.to_string();
    if category != AppCategory::Terminal {
        let toks = tokenize(&out);
        let listed = lists(toks.clone());
        if listed != toks {
            out = render(&tidy(listed));
        }
    }
    if category == AppCategory::Email {
        out = email_layout(&out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> String {
        spoken_cleanup(s)
    }

    #[test]
    fn clean_text_is_unchanged() {
        for s in [
            "The quarterly results exceeded expectations, driven mainly by growth in the European market.",
            "I actually really liked the new design, it's actually much cleaner than the old one.",
            "Can you tell me what the capital of France is?",
            "Okay, ship it.",
            "Hello - world « quoted »",
            "",
            "  spaced  out  ",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn spoken_punctuation_becomes_symbols() {
        assert_eq!(
            t("dear diary comma today was a long day period i finished the project question mark not quite period"),
            "Dear diary, today was a long day. I finished the project? Not quite."
        );
        assert_eq!(
            t("Shopping notes. New line. Remember to compare prices at both stores. New paragraph. Also check."),
            "Shopping notes.\nRemember to compare prices at both stores.\n\nAlso check."
        );
        assert_eq!(t("wow exclamation point"), "Wow!");
    }

    #[test]
    fn punctuation_nouns_stay_words() {
        for s in [
            "The trial period ends soon.",
            "Use the Oxford comma here.",
            "Add a period at the end.",
            "Period.",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn fillers_and_stutters_go() {
        assert_eq!(
            t("Um, so I I think the the build is uh broken again on main, can you take a look?"),
            "I think the build is broken again on main, can you take a look?"
        );
        assert_eq!(
            t("So, you know, the new hire, like, starts on Monday, and we should, you know, get her laptop ready."),
            "The new hire starts on Monday, and we should get her laptop ready."
        );
        assert_eq!(t("Um, uh, hmm."), "");
        assert_eq!(
            t("I know that that is wrong."),
            "I know that that is wrong."
        );
        assert_eq!(t("I'd like five apples."), "I'd like five apples.");
    }

    #[test]
    fn backtracking_keeps_the_correction() {
        assert_eq!(
            t("Let's meet at the cafe at 3pm. Scratch that, let's meet at the library at 4pm."),
            "Let's meet at the library at 4 PM."
        );
        assert_eq!(
            t("let's meet at the cafe at two scratch that at three and bring the slides"),
            "Let's meet at the cafe at three and bring the slides"
        );
        assert_eq!(
            t("Hello team, the meeting is moved to Tuesday, no wait, Wednesday at 10."),
            "Hello team, the meeting is moved to Wednesday at 10."
        );
        assert_eq!(
            t("I'll pick you up at seven, no wait, at eight thirty."),
            "I'll pick you up at eight thirty."
        );
        assert_eq!(
            t("Send the report to Sarah, actually I mean send it to Michael, by Friday."),
            "Send it to Michael, by Friday."
        );
        assert_eq!(
            t("by the 12th, oh and the blog post, never mind, Lisa's handling the blog post."),
            "By the 12th. Lisa's handling the blog post."
        );
    }

    #[test]
    fn literal_backtrack_words_are_kept() {
        for s in [
            "Don't forget that the meeting is at noon.",
            "There's no wait at the restaurant.",
            "Never mind the weather, we're going.",
            "I don't mind.",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn money_times_dates_percent() {
        assert_eq!(
            t("the dinner was like one hundred and twenty dollars so you owe me about forty dollars"),
            "The dinner was $120 so you owe me about $40"
        );
        assert_eq!(
            t("Remind me to call the dentist at three thirty pm on March fifth."),
            "Remind me to call the dentist at 3:30 PM on March 5th."
        );
        assert_eq!(t("Meet at 4pm."), "Meet at 4 PM.");
        assert_eq!(t("Meet at 4 p.m."), "Meet at 4 PM.");
        assert_eq!(
            t("It costs twenty five dollars and fifty cents."),
            "It costs $25.50."
        );
        assert_eq!(t("Sales grew twenty percent."), "Sales grew 20%.");
        assert_eq!(t("Due March twenty first."), "Due March 21st.");
        assert_eq!(t("Lunch at eight oh five am"), "Lunch at 8:05 AM");
        // Not money/time: left alone.
        assert_eq!(t("I am one of three people."), "I am one of three people.");
        assert_eq!(t("I may go in May."), "I may go in May.");
    }

    #[test]
    fn idempotent_on_its_own_output() {
        for s in [
            "dear diary comma today was a long day period",
            "Um, so I I think the the build is uh broken",
            "Remind me at three thirty pm on March fifth.",
        ] {
            let once = t(s);
            assert_eq!(t(&once), once, "{s}");
        }
    }

    // ---- layout ----

    fn lay(s: &str, cat: AppCategory) -> String {
        layout(&spoken_cleanup(s), cat)
    }

    #[test]
    fn ordinal_enumeration_becomes_a_numbered_list() {
        assert_eq!(
            lay("The steps are, first, clone the repo. Second, run the install script. And third, uh, start the dev server.", AppCategory::Other),
            "The steps are:\n\n1. Clone the repo.\n2. Run the install script.\n3. Start the dev server."
        );
        // Unpunctuated ASR (the llm_e2e case the 1.5B model missed).
        assert_eq!(
            lay("first we need to update the docs second fix the login bug and third ship the release", AppCategory::Document),
            "1. We need to update the docs\n2. Fix the login bug\n3. Ship the release"
        );
        assert_eq!(
            lay("For the trip, first, book the flights, second, reserve the hotel, and finally, rent a car.", AppCategory::Other),
            "For the trip:\n\n1. Book the flights\n2. Reserve the hotel\n3. Rent a car"
        );
        assert_eq!(
            lay("First of all, thanks. Secondly, the budget. Thirdly, the hires. Let me know what you think.", AppCategory::WorkChat),
            "1. Thanks.\n2. The budget.\n3. The hires.\n\nLet me know what you think."
        );
    }

    #[test]
    fn cardinal_enumeration_becomes_a_numbered_list() {
        assert_eq!(
            lay(
                "I need three things. One, eggs. Two, milk. Three, bread.",
                AppCategory::Other
            ),
            "I need three things.\n\n1. Eggs.\n2. Milk.\n3. Bread."
        );
        assert_eq!(
            lay(
                "number one call mom number two pay rent and number three water the plants",
                AppCategory::Other
            ),
            "1. Call mom\n2. Pay rent\n3. Water the plants"
        );
        assert_eq!(
            lay(
                "Agenda: 1. Budget. 2. Hiring. 3. Launch.",
                AppCategory::Other
            ),
            "Agenda:\n\n1. Budget.\n2. Hiring.\n3. Launch."
        );
    }

    #[test]
    fn prose_ordinals_and_short_enumerations_stay_prose() {
        for s in [
            "The first time I saw it, the second time I loved it, and the third time I bought it.",
            "Wait a second, I first need to check the third floor.",
            "First, eggs, second, milk.",
            "I have one, maybe two, three dogs.",
            "We came first, second and third in the races.",
            "One of them called, two of them wrote and three never answered.",
            "first ls second pwd third whoami",
            "It is my number one priority, and number two on her list.",
        ] {
            let cat = if s.starts_with("first ls") {
                AppCategory::Terminal
            } else {
                AppCategory::Other
            };
            let want = spoken_cleanup(s);
            assert_eq!(layout(&want, cat), want, "{s}");
        }
    }

    #[test]
    fn email_gets_greeting_and_sign_off_lines() {
        assert_eq!(
            lay("Hi Tom, thanks for sending over the contract. I had a quick look and um it all looks good to me. I'll sign it tomorrow morning. Best regards, Anna.", AppCategory::Email),
            "Hi Tom,\n\nThanks for sending over the contract. I had a quick look and it all looks good to me. I'll sign it tomorrow morning.\n\nBest regards,\nAnna"
        );
        assert_eq!(
            lay(
                "hi bob new paragraph thanks for the update new paragraph best regards",
                AppCategory::Email
            ),
            "Hi bob,\n\nThanks for the update\n\nBest regards,"
        );
        assert_eq!(
            email_layout("Can you send the deck? Thanks, Raj."),
            "Can you send the deck?\n\nThanks,\nRaj"
        );
    }

    #[test]
    fn email_layout_never_invents_structure() {
        for s in [
            "Can you send the deck by Friday?",
            "Thanks for the update.",
            "Hey can you send me the file when you get a chance.",
            "Please review it. Thanks.",
            "Please review it. Thank you so much.",
            "Please review it. Thanks Everyone.",
        ] {
            assert_eq!(email_layout(s), s, "{s}");
        }
        // Not an email app: no email layout.
        let s = "Hi Tom, the build is green. Thanks, Anna.";
        assert_eq!(layout(s, AppCategory::WorkChat), s);
    }

    /// The 16 literal-use negatives of the 2026-10-24 formatting eval
    /// (`tools/probes/prototypes/formatting_eval_2026_10_24/cases.jsonl`).
    #[test]
    fn eval_negatives_are_unchanged() {
        for s in [
            "The word quote comes from Latin.",
            "Can you send me a quote for the new roof?",
            "The trial period ends next week.",
            "In British English, a period is called a full stop.",
            "Always use the Oxford comma in lists.",
            "Don't forget that we have dinner at eight.",
            "Put a comma after the greeting.",
            "There's no wait at the new place, let's go there.",
            "They launched a new line of sneakers.",
            "The question mark at the end looks wrong.",
            "I mean it when I say this is our best quarter.",
            "Quote me on that.",
            "The formula is in parentheses on the board.",
            "Put the area code in parentheses.",
            "The answer is in brackets at the bottom of the page.",
            "Can you get me a quote by end of day?",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn repeated_numbers_are_not_stutters() {
        assert_eq!(
            t("My number is five five five, one two three, four five six seven."),
            "My number is 555-123-4567."
        );
        assert_eq!(t("call five five five one two one two"), "Call 555-1212");
        assert_eq!(t("The code is two two."), "The code is two two.");
        assert_eq!(t("it was a long long day"), "It was a long day");
    }

    #[test]
    fn digit_strings_need_a_phone_shape() {
        for s in [
            "Count one two three four five.",
            "Give me five five, one two three four five, six.",
            "It's one two three.",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn filler_takes_its_pause_comma_along() {
        assert_eq!(
            t("Um, I just wanted to, uh, check in about the invoice."),
            "I just wanted to check in about the invoice."
        );
        assert_eq!(
            t("The results were, um, basically the same as last quarter."),
            "The results were basically the same as last quarter."
        );
        // A real clause boundary keeps its comma.
        assert_eq!(
            t("Hi everyone, um, just a reminder."),
            "Hi everyone, just a reminder."
        );
        assert_eq!(
            t("I wanted to, um, you know, say thanks."),
            "I wanted to say thanks."
        );
        assert_eq!(
            t("I was thinking, uh, we could go."),
            "I was thinking, we could go."
        );
    }

    #[test]
    fn leading_so_like_goes() {
        assert_eq!(
            t("So, like, the build is, you know, kind of broken right now."),
            "The build is kind of broken right now."
        );
        // "like" with meaning stays.
        assert_eq!(t("Like I said, it's fine."), "Like I said, it's fine.");
    }

    #[test]
    fn itn_times_cardinals_decimals_and_dates() {
        assert_eq!(
            t("Meet me at seven thirty tonight."),
            "Meet me at 7:30 tonight."
        );
        assert_eq!(
            t("call at eight fifteen in the morning"),
            "Call at 8:15 in the morning"
        );
        assert_eq!(
            t("We have about two hundred and fifty users on the beta."),
            "We have about 250 users on the beta."
        );
        assert_eq!(
            t("I'll be there in ten minutes."),
            "I'll be there in 10 minutes."
        );
        assert_eq!(
            t("Version two point three is live."),
            "Version 2.3 is live."
        );
        assert_eq!(
            t("It weighs zero point five kilos."),
            "It weighs 0.5 kilos."
        );
        assert_eq!(
            t("The meeting is on the twenty first of October at two PM."),
            "The meeting is on October 21st at 2 PM."
        );
        assert_eq!(t("Due the fifth of May."), "Due May 5th.");
        // Left as words.
        for s in [
            "At one point three people left.",
            "I'll pick you up at eight thirty.",
            "I'd like five apples.",
            "We counted to twenty.",
            "A hundred people came.",
            "In nineteen eighty four it rained.",
            "Ten and twelve are even.",
            "The first of many.",
            "Seven thirty works.",
        ] {
            assert_eq!(t(s), s, "{s}");
        }
    }

    #[test]
    fn email_greeting_keeps_a_title_abbreviation() {
        assert_eq!(
            lay("Dear Mr. Patel, thank you for your time yesterday. I've attached the contract for your review. Sincerely, Laura Chen.", AppCategory::Email),
            "Dear Mr. Patel,\n\nThank you for your time yesterday. I've attached the contract for your review.\n\nSincerely,\nLaura Chen"
        );
        assert_eq!(
            email_layout("Hi Dr. Lee, see you soon."),
            "Hi Dr. Lee,\n\nSee you soon."
        );
    }

    #[test]
    fn bare_thanks_closes_an_email_with_a_greeting() {
        assert_eq!(
            lay(
                "Hey John, just a quick note. The meeting moved to three PM. Thanks.",
                AppCategory::Email
            ),
            "Hey John,\n\nJust a quick note. The meeting moved to 3 PM.\n\nThanks,"
        );
        // No greeting: prose (email_layout_never_invents_structure).
        assert_eq!(
            email_layout("Please review it. Thanks."),
            "Please review it. Thanks."
        );
    }

    #[test]
    fn cardinal_list_after_an_intro_verb() {
        assert_eq!(
            lay(
                "Our priorities are one, hiring, two, onboarding, three, retention.",
                AppCategory::Other
            ),
            "Our priorities are:\n\n1. Hiring\n2. Onboarding\n3. Retention"
        );
        let s = "My answers are one, maybe two.";
        assert_eq!(lay(s, AppCategory::Other), s);
    }

    #[test]
    fn spoken_command_split_by_a_segment_join_still_works() {
        // D22: a pause between "new" and "line" can end a whisper segment
        // with a period and start the next with a capital.
        assert_eq!(
            spoken_cleanup("Buy milk new. Line eggs too."),
            spoken_cleanup("Buy milk new line eggs too.")
        );
    }
}
