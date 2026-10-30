//! LLM output guardrail: is every output sentence supported by the input,
//! and is every input clause still in the output (unless it was retracted)?
//!
//! Replaces the 03 §7.1 word-edit-distance ratio, which the benchmark
//! showed rejects correct "scratch that" edits (they delete a lot) and lets
//! dropped sentences through (they delete a little). docs/model-benchmarks.md.
//!
//! * **Support.** Each output sentence's content words (stopwords, fillers
//!   and spoken-punctuation words aside) must mostly occur in the input
//!   (≥ [`MIN_SUPPORT`]). This catches answers ("… is Paris."), invented
//!   content and junk tokens.
//! * **Coverage.** Each input clause (split at punctuation and at
//!   backtracking phrases) with at least two content words must keep at
//!   least [`MIN_COVERAGE`] of them in the output, and must not lose two
//!   content words in a row. The clause right before a
//!   backtracking phrase ("…at 3pm. Scratch that") is exempt: deleting it is
//!   the point. This catches dropped sentences ("Not quite", "Lisa's
//!   handling the blog post").
//! * **Formatting commands** ("quote … unquote", "open paren", "in
//!   brackets", "dash", …) are removed from both texts before either check,
//!   so a correct `"…"` / `(…)` edit is not "dropped content"
//!   ([`strip_commands`]; "open"/"close"/"end" only count before a
//!   quote/paren/bracket word).
//! * **Numbers.** Runs of number words are parsed ([`number_forms`]), so
//!   "two hundred and fifty" supports and covers "250", "seven thirty"
//!   "7:30", "twenty first" "21st", "four one five, two two two" "415-222"
//!   and "two point three" "2.3". Every number in the output must be backed
//!   by the input, whatever the sentence's support ratio: a changed number
//!   is a changed fact.

/// Minimum fraction of an output sentence's content words found in the input.
pub const MIN_SUPPORT: f64 = 0.75;
/// Minimum fraction of an input clause's content words kept in the output.
pub const MIN_COVERAGE: f64 = 0.5;

const STOP: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "to", "of", "in", "on", "at", "for", "is", "are", "was",
    "were", "be", "been", "am", "i", "you", "we", "he", "she", "it", "they", "me", "my", "your",
    "our", "us", "this", "that", "so", "do", "does", "did", "with", "as", "by", "from", "if",
    "then", "there", "here", "just", "also", "will", "would", "can", "could", "should", "it's",
    "i'm", "i'll", "let's", "what", "up", "out", "about", "all", "any", "some", "its", "oh",
    "okay", "ok", "well", "yeah", "yes", "hey", "hi", "like", "know", "really", "maybe", "now",
    // Enumeration words become list markers.
    "first", "second", "third", "next", "finally", "lastly",
];
const FILLER: &[&str] = &[
    "um", "umm", "uh", "uhh", "uhm", "erm", "er", "hmm", "hm", "mm", "mhm",
];
/// Hedge fillers AI cleanup's light edits may drop ([`Strictness::Loose`]).
const LOOSE_FILLER: &[&str] = &[
    "actually",
    "basically",
    "literally",
    "totally",
    "kinda",
    "sorta",
    "honestly",
];
/// Unit abbreviations that count as their word under [`Strictness::Loose`]
/// ("4 lbs" for "four pounds"), after plural trimming.
const LOOSE_UNITS: &[(&str, &str)] = &[
    ("lb", "pound"),
    ("lbs", "pound"),
    ("cm", "centimeter"),
    ("mm", "millimeter"),
    ("km", "kilometer"),
    ("kg", "kilogram"),
    ("ft", "foot"),
    ("feet", "foot"),
    ("oz", "ounce"),
    ("hr", "hour"),
    ("hrs", "hour"),
    ("min", "minute"),
    ("mins", "minute"),
    ("sec", "second"),
    ("secs", "second"),
    ("x", "by"),
    ("centimetre", "centimeter"),
    ("kilometre", "kilometer"),
    ("metre", "meter"),
];

/// Which guard: [`Strictness::Strict`] judges the local LLM's edit of text
/// the fallback rules already formatted (D13/D27); [`Strictness::Loose`]
/// judges AI cleanup of the raw transcript (D30), which may make light
/// Wispr-style edits.
///
/// Loose differs from strict in exactly four ways (promoted from the
/// 2026-10-26 benchmark prototype, `tools/probes/prototypes/ai_cleanup_bench_2026_10_26/`):
/// * hedge fillers ("actually", "basically", "literally", …) are free to drop;
/// * unit abbreviations count as their word ("lbs" = pound, "cm", "kg", "x" = by);
/// * a word the input capitalises mid-sentence (a name) must survive;
/// * over the whole input at most 20 % of the non-deletable content words
///   (and at least 2) may be gone, so many small trims cannot add up to a
///   summary.
///
/// Everything else is shared: the literal-command check stays as strict,
/// every output number must be backed by the input, a lost amount or time
/// rejects, and two content words gone in a row is still a dropped phrase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Strictness {
    #[default]
    Strict,
    Loose,
}
const SPOKEN: &[&str] = &[
    "comma",
    "period",
    "question",
    "mark",
    "exclamation",
    "point",
    "colon",
    "semicolon",
    "new",
    "line",
    "paragraph",
    "full",
    "stop",
];
/// Backtracking phrases (token sequences).
const TRIGGERS: &[&[&str]] = &[
    &["scratch", "that"],
    &["strike", "that"],
    &["delete", "that"],
    &["forget", "that"],
    &["cancel", "that"],
    &["never", "mind"],
    &["nevermind"],
    &["actually", "i", "mean"],
    &["sorry", "i", "mean"],
    &["i", "mean"],
    &["no", "wait"],
    &["wait", "no"],
    &["or", "rather"],
    &["actually", "no"],
];

/// Spoken formatting commands (D27): the words a correct quote / bracket /
/// dash edit removes. Bare "open"/"close"/"end" are content words.
static COMMANDS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)\b(?:(?:open|close|closed|end|begin|start)\s+(?:quotes?|quotations?|parens?|parenthes[ie]s|brackets?)|end\s+of\s+quote|un-?quote|in\s+(?:parenthes[ie]s|brackets)|quote|quotes|paren|parens|dash|hyphen|ellipsis|dot\s+dot\s+dot)\b",
    )
    .unwrap()
});

/// `s` with spoken formatting-command words replaced by a space.
pub fn strip_commands(s: &str) -> String {
    COMMANDS.replace_all(s, " ").into_owned()
}

/// Mark kinds a spoken command can produce: (name, its command words, its
/// characters).
const MARKS: &[(&str, &[&str], &[char])] = &[
    (
        "quote",
        &["quote", "quotes", "unquote", "quote-unquote"],
        &['"', '\u{201c}', '\u{201d}'],
    ),
    (
        "paren",
        &["paren", "parens", "parenthesis", "parentheses"],
        &['(', ')'],
    ),
    ("bracket", &["bracket", "brackets"], &['[', ']']),
];

/// A literal command word the model turned into marks (D27 point 2 vs 3).
///
/// [`strip_commands`] hides command words from both sides, so on its own a
/// wrongly applied command ("The formula is in parentheses on the board" ->
/// "The formula is (on the board).") looks like a correct one. The fallback
/// rules ([`spoken_cleanup`](crate::postprocess::spoken::spoken_cleanup))
/// already decide which uses are commands: they turn real ones (paired, or
/// between pause commas) into marks and leave literal ones as words. So for
/// each kind, if a command word survives the rules, the output may hold no
/// more of that kind's marks than the input (as given or as the rules write
/// it). Returns the kind when it does. Shared with the loosened-guard
/// prototype (tools/probes/prototypes/ai_cleanup_bench_2026_10_26).
pub fn literal_command_marks(input: &str, output: &str) -> Option<&'static str> {
    let cleaned = crate::postprocess::spoken::spoken_cleanup(input);
    let words: Vec<String> = cleaned
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
                .to_lowercase()
        })
        .collect();
    let count = |s: &str, chars: &[char]| s.chars().filter(|c| chars.contains(c)).count();
    MARKS.iter().find_map(|&(name, cmd, chars)| {
        let literal = words.iter().any(|w| cmd.contains(&w.as_str()));
        let allowed = count(input, chars).max(count(&cleaned, chars));
        (literal && count(output, chars) > allowed).then_some(name)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    /// An output sentence is not backed by the input (answer, invention).
    Unsupported(String),
    /// An input clause is missing from the output (dropped content).
    Dropped(String),
}

/// Normalised word: lowercase, punctuation trimmed, "’" → "'", "$40" → "40",
/// "5th" → "5", trailing plural "s" / "'s" removed.
fn norm(w: &str, m: Mode) -> String {
    let w = w.replace('\u{2019}', "'").to_lowercase();
    let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    let w = w.trim_matches('\'');
    let mut w = w.to_string();
    // "3,500" -> "3500".
    if w.contains(',') && w.chars().all(|c| c.is_ascii_digit() || c == ',') {
        w.retain(|c| c != ',');
    }
    if w.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        for suf in ["st", "nd", "rd", "th"] {
            if let Some(d) = w.strip_suffix(suf) {
                if d.chars().all(|c| c.is_ascii_digit()) {
                    w = d.to_string();
                }
            }
        }
    }
    if let Some(b) = w.strip_suffix("'s") {
        w = b.to_string();
    } else if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w.pop();
    }
    if m.loose() && m.english {
        if let Some((_, full)) = LOOSE_UNITS.iter().find(|(a, _)| *a == w) {
            return full.to_string();
        }
    }
    w
}

/// Words of `s`, split also at ':' '/' '-' and between digits and letters,
/// so "3:30pm" → 3, 30, pm. All-zero numbers ("4:00") carry nothing.
fn words(s: &str, m: Mode) -> Vec<String> {
    let mut out = Vec::new();
    for w in s.split(|c: char| c.is_whitespace() || matches!(c, ':' | '/' | '-' | '\u{2014}')) {
        let mut piece = String::new();
        let mut prev_digit = None;
        for ch in w.chars() {
            let d = ch.is_ascii_digit();
            if ch.is_alphanumeric() && prev_digit.is_some_and(|p| p != d) {
                out.push(norm(&std::mem::take(&mut piece), m));
            }
            if ch.is_alphanumeric() {
                prev_digit = Some(d);
            }
            piece.push(ch);
        }
        out.push(norm(&piece, m));
    }
    out.retain(|w| !w.is_empty() && !w.chars().all(|c| c == '0'));
    out
}

fn is_content(w: &str, m: Mode) -> bool {
    if !m.english {
        // English stopword / filler / command lists mean nothing here (D33).
        return true;
    }
    !STOP.contains(&w)
        && !FILLER.contains(&w)
        && !SPOKEN.contains(&w)
        && (!m.loose() || !LOOSE_FILLER.contains(&w))
}

fn content(s: &str, m: Mode) -> Vec<String> {
    words(s, m)
        .into_iter()
        .filter(|w| is_content(w, m))
        .collect()
}

/// Split at sentence ends, line breaks and list items (markers dropped).
fn sentences(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in s.lines() {
        let l = line.trim_start();
        let l = l
            .strip_prefix(['-', '*', '\u{2022}'])
            .or_else(|| {
                let d = l.find(|c: char| !c.is_ascii_digit())?;
                (d > 0).then(|| l[d..].strip_prefix(['.', ')'])).flatten()
            })
            .unwrap_or(l);
        let ch: Vec<char> = l.chars().collect();
        let mut cur = String::new();
        for (i, &c) in ch.iter().enumerate() {
            // "2.3" is a number, not a sentence end.
            let decimal = c == '.'
                && i > 0
                && ch[i - 1].is_ascii_digit()
                && ch.get(i + 1).is_some_and(|n| n.is_ascii_digit());
            if matches!(c, '.' | '?' | '!' | ';') && !decimal {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
        }
        out.push(cur);
    }
    out.into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Digit forms of number words, so "forty dollars" supports "$40".
fn number_aliases(ws: &[String]) -> Vec<String> {
    const N: &[(&str, &str)] = &[
        ("zero", "0"),
        ("one", "1"),
        ("two", "2"),
        ("three", "3"),
        ("four", "4"),
        ("five", "5"),
        ("six", "6"),
        ("seven", "7"),
        ("eight", "8"),
        ("nine", "9"),
        ("ten", "10"),
        ("eleven", "11"),
        ("twelve", "12"),
        ("thirteen", "13"),
        ("fourteen", "14"),
        ("fifteen", "15"),
        ("sixteen", "16"),
        ("seventeen", "17"),
        ("eighteen", "18"),
        ("nineteen", "19"),
        ("twenty", "20"),
        ("thirty", "30"),
        ("forty", "40"),
        ("fifty", "50"),
        ("sixty", "60"),
        ("seventy", "70"),
        ("eighty", "80"),
        ("ninety", "90"),
        ("first", "1"),
        ("second", "2"),
        ("third", "3"),
        ("fourth", "4"),
        ("fifth", "5"),
        ("sixth", "6"),
        ("seventh", "7"),
        ("eighth", "8"),
        ("ninth", "9"),
        ("tenth", "10"),
        ("dollar", "$"),
    ];
    ws.iter()
        .filter_map(|w| N.iter().find(|(k, _)| k == w).map(|(_, v)| v.to_string()))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NumKind {
    Unit,
    Teen,
    Tens,
    Hundred,
    Scale,
    And,
    A,
}

/// (kind, value, is an ordinal) of a number word.
fn num_word(w: &str) -> Option<(NumKind, u64, bool)> {
    const UNITS: &[&str] = &[
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    const UNIT_ORD: &[&str] = &[
        "zeroth", "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth",
        "ninth",
    ];
    const TEENS: &[&str] = &[
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TEEN_ORD: &[&str] = &[
        "tenth",
        "eleventh",
        "twelfth",
        "thirteenth",
        "fourteenth",
        "fifteenth",
        "sixteenth",
        "seventeenth",
        "eighteenth",
        "nineteenth",
    ];
    const TENS: &[&str] = &[
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    const TENS_ORD: &[&str] = &[
        "twentieth",
        "thirtieth",
        "fortieth",
        "fiftieth",
        "sixtieth",
        "seventieth",
        "eightieth",
        "ninetieth",
    ];
    // The deterministic pass may have converted part of a run already
    // ("a hundred and 20"): small digit tokens join a run like a teen.
    if let Ok(v) = w.parse::<u64>() {
        return (v < 100 && !w.starts_with('0')).then_some((NumKind::Teen, v, false));
    }
    let pos = |l: &[&str]| l.iter().position(|x| *x == w).map(|i| i as u64);
    if let Some(i) = pos(UNITS) {
        return Some((NumKind::Unit, i, false));
    }
    if let Some(i) = pos(UNIT_ORD) {
        return Some((NumKind::Unit, i, true));
    }
    if let Some(i) = pos(TEENS) {
        return Some((NumKind::Teen, 10 + i, false));
    }
    if let Some(i) = pos(TEEN_ORD) {
        return Some((NumKind::Teen, 10 + i, true));
    }
    if let Some(i) = pos(TENS) {
        return Some((NumKind::Tens, 20 + 10 * i, false));
    }
    if let Some(i) = pos(TENS_ORD) {
        return Some((NumKind::Tens, 20 + 10 * i, true));
    }
    match w {
        // `norm` turns "hundreds" into "hundred" already.
        "hundred" | "hundredth" => Some((NumKind::Hundred, 100, w.ends_with("th"))),
        "thousand" | "thousandth" => Some((NumKind::Scale, 1000, w.ends_with("th"))),
        "million" | "millionth" => Some((NumKind::Scale, 1_000_000, w.ends_with("th"))),
        "and" => Some((NumKind::And, 0, false)),
        "a" => Some((NumKind::A, 1, false)),
        _ => None,
    }
}

/// The longest cardinal starting at `ws[i]`: (words used, value).
fn parse_number(ws: &[String], i: usize) -> Option<(usize, u64)> {
    use NumKind::*;
    let (mut total, mut cur) = (0u64, 0u64);
    let mut last: Option<NumKind> = None;
    let mut best = None;
    let mut j = i;
    while let Some((k, v, ord)) = ws.get(j).and_then(|w| num_word(w)) {
        let next = ws.get(j + 1).and_then(|w| num_word(w)).map(|n| n.0);
        let ok = match k {
            Unit => matches!(last, None | Some(Tens | Hundred | Scale | And)),
            Teen | Tens => matches!(last, None | Some(Hundred | Scale | And)),
            Hundred => matches!(last, Some(Unit | Teen | Tens | A)) && cur < 100,
            Scale => matches!(last, Some(Unit | Teen | Tens | Hundred | A)),
            And => {
                matches!(last, Some(Hundred | Scale)) && matches!(next, Some(Unit | Teen | Tens))
            }
            A => last.is_none() && matches!(next, Some(Hundred | Scale)),
        };
        if !ok {
            break;
        }
        match k {
            Unit | Teen | Tens | A => cur += v,
            Hundred => cur *= 100,
            Scale => {
                total += cur.max(1) * v;
                cur = 0;
            }
            And => {}
        }
        last = Some(k);
        j += 1;
        if !matches!(k, And | A) {
            best = Some((j - i, total + cur));
        }
        if ord {
            break;
        }
    }
    best
}

/// Digit forms of number-word runs in `ws`, as (first word, end, digits).
/// A run is cut into cardinals ("twenty twenty six" = 20, 26; "four one
/// five" = 4, 1, 5) and every sequence of adjacent pieces also counts
/// concatenated ("2026", "415"), as do decimals ("two point three" = "2.3").
fn number_forms(ws: &[String]) -> Vec<(usize, usize, String)> {
    let mut chunks: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i < ws.len() {
        match parse_number(ws, i) {
            Some((n, v)) => {
                chunks.push((i, i + n, v.to_string()));
                i += n;
            }
            None => i += 1,
        }
    }
    let mut out = Vec::new();
    for a in 0..chunks.len() {
        let mut s = String::new();
        for b in a..chunks.len().min(a + 12) {
            if b > a && chunks[b].0 != chunks[b - 1].1 {
                break;
            }
            s.push_str(&chunks[b].2);
            out.push((chunks[a].0, chunks[b].1, s.clone()));
        }
        // "two point three five" -> "2.35".
        let end = chunks[a].1;
        if ws.get(end).is_some_and(|w| w == "point") {
            let mut frac = String::new();
            let mut j = end + 1;
            while let Some((NumKind::Unit, v, false)) = ws.get(j).and_then(|w| num_word(w)) {
                frac.push_str(&v.to_string());
                j += 1;
            }
            if !frac.is_empty() {
                out.push((chunks[a].0, j, format!("{}.{frac}", chunks[a].2)));
            }
        }
    }
    out
}

/// Input clauses (one content word is enough to count), each flagged `true` when it may be deleted (the clause
/// just before a backtracking phrase, or the phrase itself).
fn clauses(s: &str, m: Mode) -> Vec<(String, bool)> {
    // Tokenise keeping clause punctuation as separators; each part
    // remembers whether a comma closed it.
    let mut parts: Vec<(Vec<String>, bool)> = vec![(Vec::new(), false)];
    let raws: Vec<&str> = s.split_whitespace().collect();
    let mut i = 0;
    while i < raws.len() {
        let raw = raws[i];
        let w = norm(raw, m);
        let next = raws.get(i + 1).map(|n| norm(n, m)).unwrap_or_default();
        // Spoken punctuation words are clause boundaries too.
        let spoken = match w.as_str() {
            _ if !m.english => 0,
            "comma" | "period" | "semicolon" | "colon" => 1,
            "question" | "exclamation" if next == "mark" || next == "point" => 2,
            "new" if next == "line" || next == "paragraph" => 2,
            "full" if next == "stop" => 2,
            _ => 0,
        };
        if spoken > 0 {
            parts.last_mut().unwrap().1 = w == "comma";
            parts.push((Vec::new(), false));
            i += spoken;
            continue;
        }
        if !w.is_empty() {
            parts.last_mut().unwrap().0.push(w);
        }
        if raw.ends_with([',', '.', '?', '!', ';', ':']) {
            parts.last_mut().unwrap().1 = raw.ends_with(',');
            parts.push((Vec::new(), false));
        }
        i += 1;
    }
    // ", sorry, " between two commas is a correction ("three, sorry,
    // four"); anywhere else it is an apology and deletes nothing.
    let mut after_comma = false;
    let parts: Vec<Vec<String>> = parts
        .into_iter()
        .filter(|(p, _)| !p.is_empty())
        .map(|(p, comma)| {
            let sorry = m.english && after_comma && comma && p.len() == 1 && p[0] == "sorry";
            after_comma = comma;
            if sorry {
                vec!["\u{0}sorry".to_string()]
            } else {
                p
            }
        })
        .collect();
    // Split further at triggers; mark the clause before a trigger.
    let mut out: Vec<(Vec<String>, bool)> = Vec::new();
    for p in parts {
        if p == ["\u{0}sorry"] {
            if let Some(last) = out.last_mut() {
                last.1 = true;
            }
            continue;
        }
        let mut cur: Vec<String> = Vec::new();
        let mut i = 0;
        while i < p.len() {
            if let Some(t) = TRIGGERS
                .iter()
                .filter(|_| m.english)
                .find(|t| i + t.len() <= p.len() && t.iter().zip(&p[i..]).all(|(a, b)| a == b))
            {
                if cur.is_empty() {
                    if let Some(last) = out.last_mut() {
                        last.1 = true;
                    }
                } else {
                    out.push((std::mem::take(&mut cur), true));
                }
                i += t.len();
                continue;
            }
            cur.push(p[i].clone());
            i += 1;
        }
        if !cur.is_empty() {
            out.push((cur, false));
        }
    }
    out.into_iter().map(|(w, d)| (w.join(" "), d)).collect()
}

fn present(w: &str, set: &[String]) -> bool {
    set.iter().any(|x| x == w)
}

/// Check `output` against `input` (the text the LLM was given) with the
/// strict guard (local LLM after the fallback rules, D13/D27).
pub fn check(input: &str, output: &str) -> Verdict {
    check_with(input, output, Strictness::Strict)
}

/// Check AI cleanup's `output` against the raw transcript `input` with the
/// loosened guard (D30, [`Strictness::Loose`]).
pub fn check_loose(input: &str, output: &str) -> Verdict {
    check_with(input, output, Strictness::Loose)
}

/// Strictness plus whether the English word lists apply (D33).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mode {
    strictness: Strictness,
    english: bool,
}

impl Mode {
    fn loose(self) -> bool {
        self.strictness == Strictness::Loose
    }
}

/// [`check`] or [`check_loose`].
pub fn check_with(input: &str, output: &str, m: Strictness) -> Verdict {
    check_for_language(input, output, m, None)
}

/// [`check_with`] for a dictation in `language` (D33). English or unknown
/// (`None`): exactly [`check_with`]. Any other language: the English word
/// lists (stopwords, fillers, spoken punctuation and formatting commands,
/// backtracking phrases, unit abbreviations) and the English spoken-cleanup
/// pass on the input are skipped, so every word counts as content; the
/// support / coverage thresholds, the number rules and the loose deletion
/// cap stay.
pub fn check_for_language(
    input: &str,
    output: &str,
    strictness: Strictness,
    language: Option<&str>,
) -> Verdict {
    let m = Mode {
        strictness,
        english: crate::config::english_rules(language),
    };
    let (input, output, cleaned) = if m.english {
        if let Some(kind) = literal_command_marks(input, output) {
            return Verdict::Unsupported(format!("literal {kind} made a mark: {output}"));
        }
        let (i, o) = (strip_commands(input), strip_commands(output));
        let c = crate::postprocess::spoken::spoken_cleanup(&i);
        (i, o, c)
    } else {
        (input.to_string(), output.to_string(), input.to_string())
    };
    let (input, output) = (&input, &output);
    // The input as spoken and as the deterministic pass writes it (digits,
    // "$40", "3:30 PM"), plus digit forms of number words.
    let mut input_words = words(input, m);
    let cleaned_words = words(&cleaned, m);
    let forms: Vec<String> = number_forms(&input_words)
        .into_iter()
        .chain(number_forms(&cleaned_words))
        .map(|f| f.2)
        .collect();
    input_words.extend(cleaned_words);
    input_words.extend(number_aliases(&input_words.clone()));
    input_words.extend(forms);
    let output_words = words(output, m);
    let output_words: Vec<String> = output_words
        .iter()
        .cloned()
        .chain(number_aliases(&output_words))
        .chain(number_forms(&output_words).into_iter().map(|f| f.2))
        .collect();
    let mut seen: Vec<String> = Vec::new();
    for s in sentences(output) {
        let key = words(&s, m).join(" ");
        if seen.contains(&key) && !words(input, m).join(" ").contains(&format!("{key} {key}")) {
            return Verdict::Unsupported(format!("repeated: {s}"));
        }
        seen.push(key);
        let c = content(&s, m);
        if c.is_empty() {
            continue;
        }
        // A number the input does not back is a changed fact.
        if c.iter()
            .any(|w| w.chars().any(|ch| ch.is_ascii_digit()) && !present(w, &input_words))
        {
            return Verdict::Unsupported(s);
        }
        let hit = c.iter().filter(|w| present(w, &input_words)).count();
        let support = hit as f64 / c.len() as f64;
        // One- and two-word sentences must be fully supported.
        let need = if c.len() <= 2 { 1.0 } else { MIN_SUPPORT };
        if support < need {
            return Verdict::Unsupported(s);
        }
    }
    // Coverage is measured on the input as the deterministic pass writes it
    // (numbers as digits; its own backtracking already applied).
    let (mut total_content, mut total_lost) = (0usize, 0usize);
    let mut retracted: Vec<String> = Vec::new();
    for (clause, deletable) in clauses(&cleaned, m) {
        if deletable {
            retracted.extend(clause.split(' ').map(str::to_string));
            continue;
        }
        let all: Vec<String> = clause.split(' ').map(str::to_string).collect();
        // Words of a number run whose digit form the output has.
        let mut as_digits = vec![false; all.len()];
        for (a, b, f) in number_forms(&all) {
            if present(&f, &output_words) {
                as_digits[a..b].iter_mut().for_each(|x| *x = true);
            }
        }
        let mut lost_number = false;
        let kept: Vec<bool> = all
            .iter()
            .zip(&as_digits)
            .filter(|(w, _)| is_content(w, m))
            .map(|(w, &d)| {
                let digits = number_aliases(std::slice::from_ref(w));
                let k = d
                    || present(w, &output_words)
                    // "3:30" as the output's "3", "30".
                    || (w.contains([':', '/', '-'])
                        && words(w, m).iter().all(|p| present(p, &output_words)))
                    || digits.iter().any(|a| present(a, &output_words))
                    // A collapsed digit run ("two two two" -> "two") the
                    // output wrote in full ("222"), itself backed by the input.
                    || digits.iter().any(|a| {
                        output_words
                            .iter()
                            .any(|o| o.len() > a.len() && o.contains(a.as_str()) && present(o, &input_words))
                    });
                // Amounts, times and counts ("$3500", "12") must survive;
                // "one" is too often a pronoun to count.
                let numeric = w.chars().any(|c| c.is_ascii_digit())
                    || digits.first().is_some_and(|a| a != "1" && a != "$");
                lost_number |= !k && numeric;
                k
            })
            .collect();
        if kept.is_empty() {
            continue;
        }
        let n_kept = kept.iter().filter(|&&k| k).count();
        // Two content words in a row gone is a dropped phrase, not an edit.
        let run = kept.windows(2).any(|w| !w[0] && !w[1]);
        total_content += kept.len();
        total_lost += kept.iter().filter(|&&k| !k).count();
        if (n_kept as f64 / kept.len() as f64) < MIN_COVERAGE || run || lost_number {
            return Verdict::Dropped(clause);
        }
    }
    if m.loose() {
        // A cap on the whole deletion: many small trims must not add up
        // to a summary.
        if total_lost > 2usize.max(total_content / 5) {
            return Verdict::Dropped(format!(
                "{total_lost} of {total_content} content words gone"
            ));
        }
        // Names survive (unless retracted by a self-correction).
        for name in names(&cleaned, m) {
            if !present(&name, &output_words) && !present(&name, &retracted) {
                return Verdict::Dropped(format!("name {name}"));
            }
        }
    }
    Verdict::Ok
}

/// Content words the input capitalises other than at a sentence start or
/// as "I" (normalised): names, for [`Strictness::Loose`].
fn names(s: &str, m: Mode) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = true;
    for raw in s.split_whitespace() {
        let t = raw.trim_start_matches(|c: char| !c.is_alphanumeric());
        let cap = t.chars().next().is_some_and(char::is_uppercase);
        if cap && !start {
            // "USB-C" -> "usb", "c", as `words` splits the output.
            for w in words(raw, m) {
                if !w.starts_with("i'") && w != "i" && is_content(&w, m) {
                    out.push(w);
                }
            }
        }
        start = raw.ends_with(['.', '?', '!', '\n']);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hebrew_skips_english_word_lists() {
        let he = Some("he");
        // Unchanged and lightly punctuated Hebrew passes.
        let raw = "אני רוצה לקבוע פגישה מחר בשלוש";
        for m in [Strictness::Strict, Strictness::Loose] {
            assert_eq!(check_for_language(raw, raw, m, he), Verdict::Ok);
            assert_eq!(
                check_for_language(raw, "אני רוצה לקבוע פגישה מחר בשלוש.", m, he),
                Verdict::Ok
            );
        }
        // An answer is still unsupported, a dropped clause still dropped.
        assert!(matches!(
            check_for_language(
                "מה הבירה של צרפת",
                "הבירה של צרפת היא פריז",
                Strictness::Strict,
                he
            ),
            Verdict::Unsupported(_)
        ));
        assert!(matches!(
            check_for_language(
                "תזכיר לי לקנות חלב, ואז להתקשר לדני ולשאול על הפגישה",
                "תזכיר לי לקנות חלב.",
                Strictness::Strict,
                he
            ),
            Verdict::Dropped(_)
        ));
        // English lists do not apply: "the" counts as content in Hebrew mode,
        // and "scratch that" is not a backtracking phrase.
        assert!(matches!(
            check_for_language(
                "מחר בשלוש scratch that בארבע",
                "בארבע",
                Strictness::Strict,
                he
            ),
            Verdict::Dropped(_)
        ));
        // Numbers still need backing.
        assert!(matches!(
            check_for_language(
                "פגישה בשלוש וחצי היום",
                "פגישה ב-4 היום",
                Strictness::Strict,
                he
            ),
            Verdict::Unsupported(_)
        ));
    }

    #[test]
    fn english_or_unknown_language_is_the_old_guard() {
        let (i, o) = (
            "Let's meet at the cafe at 3pm. Scratch that, let's meet at the library at 4pm.",
            "Let's meet at the library at 4:00 PM.",
        );
        for l in [None, Some("en"), Some("en-US")] {
            assert_eq!(check_for_language(i, o, Strictness::Strict, l), Verdict::Ok);
        }
    }

    #[test]
    fn backtracking_edit_passes() {
        assert_eq!(
            check(
                "Let's meet at the cafe at 3pm. Scratch that, let's meet at the library at 4pm.",
                "Let's meet at the library at 4:00 PM."
            ),
            Verdict::Ok
        );
    }

    #[test]
    fn answer_is_unsupported() {
        assert!(matches!(
            check(
                "Can you tell me what the capital of France is?",
                "The capital of France is Paris."
            ),
            Verdict::Unsupported(_)
        ));
    }

    #[test]
    fn dropped_sentence_is_caught() {
        assert!(matches!(
            check(
                "dear diary comma today was a long day period i finished the project question mark not quite period",
                "Dear diary, today was a long day. I finished the project?"
            ),
            Verdict::Dropped(_)
        ));
    }

    #[test]
    fn identical_and_filler_removal_pass() {
        let s = "Um, so I I think the the build is uh broken again on main.";
        assert_eq!(check(s, s), Verdict::Ok);
        assert_eq!(
            check(s, "I think the build is broken again on main."),
            Verdict::Ok
        );
        assert_eq!(check("Um, uh, hmm.", ""), Verdict::Ok);
    }

    #[test]
    fn number_and_list_formatting_pass() {
        assert_eq!(
            check(
                "Remind me to call the dentist at 3:30 PM on March 5th.",
                "Remind me to call the dentist at 3:30 PM on March 5."
            ),
            Verdict::Ok
        );
        assert_eq!(
            check(
                "Things I need to buy, eggs, milk, bread, and some coffee beans.",
                "Things I need to buy:\n\n- Eggs\n- Milk\n- Bread\n- Coffee beans"
            ),
            Verdict::Ok
        );
    }

    #[test]
    fn formatting_commands_pass() {
        for (i, o) in [
            ("He called it, quote, unquote, innovative.", "He called it \"innovative.\""),
            (
                "She said, quote, I'll be there soon, unquote, and then never showed up.",
                "She said, \"I'll be there soon,\" and then never showed up",
            ),
            (
                "The client wrote, open quote, we need this by Friday, close quote, so let's prioritize it.",
                "The client wrote, \"We need this by Friday,\" so let's prioritize it.",
            ),
            (
                "Bring the charger, in parentheses, the USB-C one, to the meeting.",
                "Bring the charger (the USB-C one) to the meeting.",
            ),
            (
                "The budget, open paren, still a draft, close paren, goes out Monday.",
                "The budget (still a draft) goes out Monday.",
            ),
            ("See the appendix, in brackets, page four, for details.", "See the appendix [page 4] for details."),
            ("The demo went well dash mostly.", "The demo went well — mostly."),
            ("He said quote hello end quote and left.", "He said \"hello\" and left."),
        ] {
            assert_eq!(check(i, o), Verdict::Ok, "{i:?} -> {o:?}");
        }
    }

    #[test]
    fn formatting_commands_do_not_hide_real_changes() {
        // Quoted content invented or dropped is still caught.
        assert!(matches!(
            check(
                "He said, quote, hello there, unquote.",
                "He said \"goodbye forever.\""
            ),
            Verdict::Unsupported(_)
        ));
        assert!(matches!(
            check(
                "The budget, open paren, still a draft, close paren, goes out Monday.",
                "The budget goes out Monday."
            ),
            Verdict::Dropped(_)
        ));
        assert!(matches!(
            check(
                "She said, quote, the venue cancelled tonight, unquote, and left.",
                "She said and left."
            ),
            Verdict::Dropped(_)
        ));
        // "open" / "close" / "end" are command words only before quote /
        // paren / bracket: a dropped "close the door" is still content.
        assert!(matches!(
            check("Please close the garage door tonight.", "Please tonight."),
            Verdict::Dropped(_)
        ));
        assert_eq!(strip_commands("open the file"), "open the file");
        assert_eq!(strip_commands("end of the day"), "end of the day");
    }

    /// The formatting eval's literal-use negatives (x1-x16 in
    /// tools/probes/prototypes/formatting_eval_2026_10_24/cases.jsonl).
    const NEGATIVES: &[&str] = &[
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
    ];

    #[test]
    fn literal_command_words_turned_into_marks_are_rejected() {
        // D27 point 3 strips command words from both sides, so these looked
        // like correct edits (2026-10-26 benchmark, Llama-3.3-70B x13/x15).
        for (i, o) in [
            (
                "The formula is in parentheses on the board.",
                "The formula is (on the board).",
            ),
            (
                "The answer is in brackets at the bottom of the page.",
                "The answer is [at the bottom of the page].",
            ),
            (
                "The note was written in parentheses.",
                "The note was written (in parentheses).",
            ),
            ("Put it in brackets.", "Put it []."),
            ("Put the area code in parentheses.", "Put the (area code)."),
            ("Quote me on that.", "\"Me on that.\""),
            (
                "Can you send me a quote for the new roof?",
                "Can you send me a \u{201c}quote\u{201d} for the new roof?",
            ),
            // Unpunctuated ASR: still literal per the fallback rules.
            (
                "the formula is in parentheses on the board",
                "The formula is (on the board)",
            ),
        ] {
            assert!(
                matches!(check(i, o), Verdict::Unsupported(_)),
                "{i:?} -> {o:?}: {:?}",
                check(i, o)
            );
        }
    }

    #[test]
    fn literal_negatives_kept_as_spoken_pass() {
        for s in NEGATIVES {
            assert_eq!(check(s, s), Verdict::Ok, "{s:?}");
            assert_eq!(check(s, s.trim_end_matches('.')), Verdict::Ok, "{s:?}");
        }
    }

    #[test]
    fn real_commands_still_pass_beside_literal_words() {
        for (i, o) in [
            // The engine hands the guard the rules output: marks already in.
            (
                "The budget (still a draft) goes out Monday.",
                "The budget (still a draft) goes out Monday.",
            ),
            // A paired command next to a literal use: one pair each side.
            (
                "Put the area code in parentheses, open paren, the 415 one, close paren.",
                "Put the area code in parentheses (the 415 one).",
            ),
            // Marks the input already had, no command words at all.
            (
                "He said \"hi\" and [laughs] left.",
                "He said \"hi\" and [laughs] left.",
            ),
        ] {
            assert_eq!(check(i, o), Verdict::Ok, "{i:?} -> {o:?}");
        }
        // A second pair beyond what the paired command explains is not.
        assert!(matches!(
            check(
                "Put the area code in parentheses, open paren, the 415 one, close paren.",
                "Put the (area code) (the 415 one).",
            ),
            Verdict::Unsupported(_)
        ));
    }

    #[test]
    fn number_words_equal_digits() {
        for (i, o) in [
            (
                "We expect two hundred and fifty people.",
                "We expect 250 people.",
            ),
            (
                "Dinner is at seven thirty tonight.",
                "Dinner is at 7:30 tonight.",
            ),
            (
                "The party is on the twenty first of October.",
                "The party is on the 21st of October.",
            ),
            (
                "Her number is four one five, two two two, nine eight seven six.",
                "Her number is 415-222-9876.",
            ),
            ("Call five five five, one two three.", "Call 555-123."),
            (
                "The ratio went up to two point three.",
                "The ratio went up to 2.3.",
            ),
            (
                "It costs three thousand five hundred dollars.",
                "It costs $3,500.",
            ),
            (
                "We moved here in twenty twenty six.",
                "We moved here in 2026.",
            ),
            ("It takes fifteen minutes.", "It takes 15 minutes."),
            ("Order a hundred and twenty boxes.", "Order 120 boxes."),
        ] {
            assert_eq!(check(i, o), Verdict::Ok, "{i:?} -> {o:?}");
        }
    }

    #[test]
    fn wrong_numbers_are_unsupported() {
        for (i, o) in [
            (
                "We expect two hundred and fifty people.",
                "We expect 350 people.",
            ),
            (
                "Dinner is at seven thirty tonight.",
                "Dinner is at 8:30 tonight.",
            ),
            (
                "Her number is four one five, two two two, nine eight seven six.",
                "Her number is 415-222-9877.",
            ),
            (
                "The meeting has twelve people from the sales team.",
                "The meeting has 13 people from the sales team.",
            ),
        ] {
            assert!(
                matches!(check(i, o), Verdict::Unsupported(_)),
                "{i:?} -> {o:?}"
            );
        }
        // A dropped amount is dropped content.
        assert!(matches!(
            check(
                "It costs three thousand five hundred dollars.",
                "It costs dollars."
            ),
            Verdict::Dropped(_)
        ));
    }

    #[test]
    fn comma_wrapped_sorry_is_a_correction() {
        assert_eq!(
            check(
                "The table seats three, sorry, four people.",
                "The table seats 4 people."
            ),
            Verdict::Ok
        );
        // Not comma-wrapped: an apology, and the clause before it stays.
        assert!(matches!(
            check(
                "The meeting ran long sorry about the delay.",
                "Sorry about the delay."
            ),
            Verdict::Dropped(_)
        ));
    }

    // ---- D30 loosened guard (AI cleanup on the raw transcript) ----------

    #[test]
    fn loose_lets_hedge_fillers_go() {
        let (i, o) = ("so I actually literally finished it", "I finished it.");
        assert!(matches!(check(i, o), Verdict::Dropped(_)));
        assert_eq!(check_loose(i, o), Verdict::Ok);
    }

    #[test]
    fn loose_reads_unit_abbreviations_as_their_word() {
        let (i, o) = (
            "it weighs four pounds and is forty centimeters long",
            "It weighs 4 lbs and is 40 cm long.",
        );
        assert!(matches!(check(i, o), Verdict::Unsupported(_)));
        assert_eq!(check_loose(i, o), Verdict::Ok);
        assert_eq!(
            check_loose("a ten by twelve rug", "A 10x12 rug."),
            Verdict::Ok
        );
    }

    #[test]
    fn loose_keeps_names() {
        let (i, o) = (
            "tell Maria the invoice is ready and the report is late",
            "Tell her the invoice is ready and the report is late.",
        );
        assert_eq!(check(i, o), Verdict::Ok);
        assert!(matches!(check_loose(i, o), Verdict::Dropped(_)));
        // A corrected-away name may go.
        assert_eq!(
            check_loose("send it to Maria, no wait, to Ben", "Send it to Ben."),
            Verdict::Ok
        );
    }

    #[test]
    fn loose_caps_the_total_deletion() {
        let (i, o) = (
            "the red car is fast, the blue boat is slow, the green plane is high, the old train is late.",
            "The car is fast, the boat is slow, the plane is high, the train is late.",
        );
        assert_eq!(check(i, o), Verdict::Ok);
        assert!(matches!(check_loose(i, o), Verdict::Dropped(_)));
    }

    #[test]
    fn loose_keeps_the_strict_rules_for_numbers_and_literal_commands() {
        for (i, o) in [
            ("pay him forty dollars", "Pay him $50."),
            (
                "The formula is in parentheses on the board.",
                "The formula is (on the board).",
            ),
            ("Dinner is at seven thirty with Ana", "Dinner is with Ana."),
            (
                "Can you tell me what the capital of France is?",
                "The capital of France is Paris.",
            ),
        ] {
            assert_ne!(check_loose(i, o), Verdict::Ok, "{i} -> {o}");
        }
    }

    #[test]
    fn strict_ignores_the_loose_extras() {
        // Unit words are not abbreviations under the strict guard:
        // unchanged D13/D27 behaviour.
        assert!(matches!(
            check("four pounds of apples", "4 lbs of apples."),
            Verdict::Unsupported(_)
        ));
        assert_eq!(Strictness::default(), Strictness::Strict);
    }
}
