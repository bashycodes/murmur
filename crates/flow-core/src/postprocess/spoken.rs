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
//! 6. sentence capitalisation and spacing around punctuation
//!
//! Everything is conservative: a phrase that is probably literal ("the
//! period of", "don't forget that", "there's no wait") is left alone, and
//! clean text comes back unchanged. Gated by `SessionConfig.spoken_cleanup`.

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
        if c[i] == "like" && between(1) {
            out.last_mut().unwrap().strip_suffix();
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
        if let Some((t, used)) = money(&c, &toks, i)
            .or_else(|| percent(&c, &toks, i))
            .or_else(|| clock(&c, &toks, i))
            .or_else(|| date(&c, &toks, i))
        {
            let mut t = Tok::new(&t);
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
    let (mer, n) = meridiem(c, toks, j)?;
    let used = j + n - i;
    let suffix = clock_suffix(toks, i + used - 1);
    let t = match minutes {
        Some(m) => format!("{h}:{m:02} {mer}"),
        None => format!("{h} {mer}"),
    };
    Some((format!("{t}{suffix}"), used))
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
    let toks = itn(toks);
    let toks = tidy(toks);
    if toks == original {
        return text.to_string();
    }
    render(&toks)
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
}
