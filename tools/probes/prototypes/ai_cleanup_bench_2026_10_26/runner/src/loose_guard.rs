//! Loosened LLM-output guard (2026-10-26 AI-cleanup bench prototype).
//!
//! A copy of `flow_core::guard` (support + coverage, D13/D27) with the owner's
//! 2026-10-26 loosening: AI cleanup may make light Wispr-style edits, so the
//! guard must allow them while still rejecting added substantive words,
//! changed numbers/names and big deletions. Every change is marked `LOOSE:`;
//! the rest is the product guard verbatim, so `diff` against
//! crates/flow-core/src/guard.rs shows the whole proposal.
//!
//! * LOOSE 1: more fillers are free to drop ("actually", "basically",
//!   "literally", "totally", "kinda", "sorta", "honestly").
//! * LOOSE 2: unit abbreviations count as their word ("lbs" = pound, "cm" =
//!   centimeter, "kg", "km", "ft", "oz", "hr", "min", "sec", "x" = by).
//! * LOOSE 3 (tried, reverted): tolerating two content words in a row gone
//!   let the 1.5B delete a whole aside ("The deadline dash which nobody likes
//!   dash is Friday" -> "The deadline is Friday": "likes" is a stopword, so
//!   only "which nobody" counted). Short trims ("and it weighs" -> "and
//!   weighs") lose stopwords only and already pass, so the run rule stays.
//! * LOOSE 4 (tightening, for the owner's "changed names" limit): a word the
//!   input capitalises mid-sentence (a name) must survive.
//! * LOOSE 5 (tightening, "big deletions"): over the whole input at most 20 %
//!   of the non-deletable content words (and at least 2 allowed) may be gone.
//!
//! Numbers are unchanged: any output number the input does not back, or any
//! lost amount/time/count, is still a reject.

pub use flow_core::guard::Verdict;

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
    // LOOSE 1: hedge fillers a light edit may drop.
    "actually", "basically", "literally", "totally", "kinda", "sorta", "honestly",
];
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

/// Normalised word: lowercase, punctuation trimmed, "’" → "'", "$40" → "40",
/// "5th" → "5", trailing plural "s" / "'s" removed.
fn norm(w: &str) -> String {
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
     // LOOSE 2: a unit abbreviation is its word ("4 lbs" for "four pounds").
    const UNITS: &[(&str, &str)] = &[
        ("lb", "pound"), ("lbs", "pound"), ("cm", "centimeter"), ("mm", "millimeter"),
        ("km", "kilometer"), ("kg", "kilogram"), ("ft", "foot"), ("feet", "foot"),
        ("oz", "ounce"), ("hr", "hour"), ("hrs", "hour"), ("min", "minute"), ("mins", "minute"),
        ("sec", "second"), ("secs", "second"), ("x", "by"), ("centimetre", "centimeter"),
        ("kilometre", "kilometer"), ("metre", "meter"),
    ];
    if let Some((_, full)) = UNITS.iter().find(|(a, _)| *a == w) {
        return full.to_string();
    }
    w
}

/// Words of `s`, split also at ':' '/' '-' and between digits and letters,
/// so "3:30pm" → 3, 30, pm. All-zero numbers ("4:00") carry nothing.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in s.split(|c: char| c.is_whitespace() || matches!(c, ':' | '/' | '-' | '\u{2014}')) {
        let mut piece = String::new();
        let mut prev_digit = None;
        for ch in w.chars() {
            let d = ch.is_ascii_digit();
            if ch.is_alphanumeric() && prev_digit.is_some_and(|p| p != d) {
                out.push(norm(&std::mem::take(&mut piece)));
            }
            if ch.is_alphanumeric() {
                prev_digit = Some(d);
            }
            piece.push(ch);
        }
        out.push(norm(&piece));
    }
    out.retain(|w| !w.is_empty() && !w.chars().all(|c| c == '0'));
    out
}

fn is_content(w: &str) -> bool {
    !STOP.contains(&w) && !FILLER.contains(&w) && !SPOKEN.contains(&w)
}

fn content(s: &str) -> Vec<String> {
    words(s).into_iter().filter(|w| is_content(w)).collect()
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
fn clauses(s: &str) -> Vec<(String, bool)> {
    // Tokenise keeping clause punctuation as separators; each part
    // remembers whether a comma closed it.
    let mut parts: Vec<(Vec<String>, bool)> = vec![(Vec::new(), false)];
    let raws: Vec<&str> = s.split_whitespace().collect();
    let mut i = 0;
    while i < raws.len() {
        let raw = raws[i];
        let w = norm(raw);
        let next = raws.get(i + 1).map(|n| norm(n)).unwrap_or_default();
        // Spoken punctuation words are clause boundaries too.
        let spoken = match w.as_str() {
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
            let sorry = after_comma && comma && p.len() == 1 && p[0] == "sorry";
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

/// Check `output` against `input` (the text the LLM was given).
pub fn check(input: &str, output: &str) -> Verdict {
    let (input, output) = (&strip_commands(input), &strip_commands(output));
    let cleaned = flow_core::postprocess::spoken::spoken_cleanup(input);
    // The input as spoken and as the deterministic pass writes it (digits,
    // "$40", "3:30 PM"), plus digit forms of number words.
    let mut input_words = words(input);
    let cleaned_words = words(&cleaned);
    let forms: Vec<String> = number_forms(&input_words)
        .into_iter()
        .chain(number_forms(&cleaned_words))
        .map(|f| f.2)
        .collect();
    input_words.extend(cleaned_words);
    input_words.extend(number_aliases(&input_words.clone()));
    input_words.extend(forms);
    let output_words = words(output);
    let output_words: Vec<String> = output_words
        .iter()
        .cloned()
        .chain(number_aliases(&output_words))
        .chain(number_forms(&output_words).into_iter().map(|f| f.2))
        .collect();
    let mut seen: Vec<String> = Vec::new();
    for s in sentences(output) {
        let key = words(&s).join(" ");
        if seen.contains(&key) && !words(input).join(" ").contains(&format!("{key} {key}")) {
            return Verdict::Unsupported(format!("repeated: {s}"));
        }
        seen.push(key);
        let c = content(&s);
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
    for (clause, deletable) in clauses(&cleaned) {
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
            .filter(|(w, _)| is_content(w))
            .map(|(w, &d)| {
                let digits = number_aliases(std::slice::from_ref(w));
                let k = d
                    || present(w, &output_words)
                    // "3:30" as the output's "3", "30".
                    || (w.contains([':', '/', '-'])
                        && words(w).iter().all(|p| present(p, &output_words)))
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
        // Two content words in a row gone is a dropped phrase, not an edit
        // (LOOSE 3 reverted, see the module doc).
        let run = kept.windows(2).any(|w| !w[0] && !w[1]);
        total_content += kept.len();
        total_lost += kept.iter().filter(|&&k| !k).count();
        if (n_kept as f64 / kept.len() as f64) < MIN_COVERAGE || run || lost_number {
            return Verdict::Dropped(clause);
        }
    }
    // LOOSE 5: a cap on the whole deletion, so many small trims cannot add
    // up to a summary.
    if total_lost > 2usize.max(total_content / 5) {
        return Verdict::Dropped(format!("{total_lost} of {total_content} content words gone"));
    }
    // LOOSE 4: names survive.
    for name in names(&cleaned) {
        if !present(&name, &output_words) && !present(&name, &retracted) {
            return Verdict::Dropped(format!("name {name}"));
        }
    }
    Verdict::Ok
}

/// LOOSE 4: content words the input capitalises other than at a sentence
/// start or as "I" (normalised).
fn names(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = true;
    for raw in s.split_whitespace() {
        let t = raw.trim_start_matches(|c: char| !c.is_alphanumeric());
        let cap = t.chars().next().is_some_and(char::is_uppercase);
        if cap && !start {
            // "USB-C" -> "usb", "c", as `words` splits the output.
            for w in words(raw) {
                if !w.starts_with("i'") && w != "i" && is_content(&w) {
                    out.push(w);
                }
            }
        }
        start = raw.ends_with(['.', '?', '!', '\n']);
    }
    out
}
