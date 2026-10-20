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
fn norm(w: &str) -> String {
    let w = w.replace('\u{2019}', "'").to_lowercase();
    let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    let w = w.trim_matches('\'');
    let mut w = w.to_string();
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
        out.extend(
            l.split(['.', '?', '!', ';'])
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string),
        );
    }
    out
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

/// Input clauses (one content word is enough to count), each flagged `true` when it may be deleted (the clause
/// just before a backtracking phrase, or the phrase itself).
fn clauses(s: &str) -> Vec<(String, bool)> {
    // Tokenise keeping clause punctuation as separators.
    let mut parts: Vec<Vec<String>> = vec![Vec::new()];
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
            parts.push(Vec::new());
            i += spoken;
            continue;
        }
        if !w.is_empty() {
            parts.last_mut().unwrap().push(w);
        }
        if raw.ends_with([',', '.', '?', '!', ';', ':']) {
            parts.push(Vec::new());
        }
        i += 1;
    }
    // Split further at triggers; mark the clause before a trigger.
    let mut out: Vec<(Vec<String>, bool)> = Vec::new();
    for p in parts.into_iter().filter(|p| !p.is_empty()) {
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
    // The input as spoken and as the deterministic pass writes it (digits,
    // "$40", "3:30 PM"), plus digit aliases of number words.
    let mut input_words = words(input);
    input_words.extend(words(&crate::postprocess::spoken::spoken_cleanup(input)));
    input_words.extend(number_aliases(&input_words.clone()));
    let output_words = words(output);
    let output_words: Vec<String> = output_words
        .iter()
        .cloned()
        .chain(number_aliases(&output_words))
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
    let cleaned = crate::postprocess::spoken::spoken_cleanup(input);
    for (clause, deletable) in clauses(&cleaned) {
        if deletable {
            continue;
        }
        let c: Vec<String> = clause
            .split(' ')
            .map(str::to_string)
            .filter(|w| is_content(w))
            .collect();
        if c.is_empty() {
            continue;
        }
        let kept: Vec<bool> = c
            .iter()
            .map(|w| {
                present(w, &output_words)
                    || number_aliases(std::slice::from_ref(w))
                        .iter()
                        .any(|a| present(a, &output_words))
            })
            .collect();
        let n_kept = kept.iter().filter(|&&k| k).count();
        // Two content words in a row gone is a dropped phrase, not an edit.
        let run = kept.windows(2).any(|w| !w[0] && !w[1]);
        if (n_kept as f64 / c.len() as f64) < MIN_COVERAGE || run {
            return Verdict::Dropped(clause);
        }
    }
    Verdict::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
