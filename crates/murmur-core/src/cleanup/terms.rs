//! D64: which dictionary terms a cleanup prompt names.
//!
//! The prompt's DICTIONARY line sits in the per-dictation user message (the
//! cached system prefix never changes), so every term costs prefill tokens
//! on each commit. Only terms the transcript plausibly misspells are named:
//! a window of the transcript's words that, with case, spaces and
//! punctuation ignored, is the term itself in another spelling ("vora" /
//! "Vo-Ra"), a near spelling (edit similarity >= 0.75, "Marbelford"), the
//! same consonant skeleton ("sip-sip" / "ZipZip") or, for a long word, the
//! same opening ("hyperbolic..."), always with the term's opening sound. A
//! term already spelled exactly is left out, and so is a dictation with no
//! near term: its prompt is the one without a dictionary. The model decides whether a named term is meant; nothing
//! here rewrites text.

use std::collections::{HashMap, HashSet};

/// Most dictionary terms one prompt names (closest first).
pub const MAX_PROMPT_TERMS: usize = 20;

/// A transcript word: byte span and its lowercase letters and digits.
struct Word {
    start: usize,
    end: usize,
    norm: String,
}

fn words(s: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut cur: Option<Word> = None;
    for (i, c) in s.char_indices() {
        // An apostrophe inside a word belongs to it ("what's").
        if c.is_alphanumeric() || (c == '\'' && cur.is_some()) {
            let w = cur.get_or_insert(Word {
                start: i,
                end: i,
                norm: String::new(),
            });
            w.end = i + c.len_utf8();
            if c.is_alphanumeric() {
                w.norm.extend(c.to_lowercase());
            }
        } else if let Some(w) = cur.take() {
            out.push(w);
        }
    }
    out.extend(cur);
    out
}

/// The digits of a spoken number word: (value, a tens word that a unit
/// may follow).
fn number_word(w: &str) -> Option<(u32, bool)> {
    const UNITS: &str = "zero one two three four five six seven eight nine ten eleven twelve \
                         thirteen fourteen fifteen sixteen seventeen eighteen nineteen";
    const TENS: &str = "twenty thirty forty fifty sixty seventy eighty ninety";
    if let Some(i) = UNITS.split(' ').position(|u| u == w) {
        return Some((i as u32, false));
    }
    TENS.split(' ')
        .position(|t| t == w)
        .map(|i| (20 + 10 * i as u32, true))
}

/// `ws` with every run of spoken number words as one word of their digits
/// ("x eighty six" -> "x", "86"; "k eight s" -> "k", "8", "s"; adjacent
/// numbers join as a code is read: "three twelve" -> "312"). `None` when
/// the text has no number word. D67: an entry with digits ("Go2", "K8s")
/// is then found when its number was said as words.
fn with_spoken_digits(ws: &[Word]) -> Option<Vec<Word>> {
    let mut out: Vec<Word> = Vec::new();
    let mut any = false;
    let mut i = 0;
    while i < ws.len() {
        let Some((v, tens)) = number_word(&ws[i].norm) else {
            out.push(Word {
                start: ws[i].start,
                end: ws[i].end,
                norm: ws[i].norm.clone(),
            });
            i += 1;
            continue;
        };
        any = true;
        let mut digits = String::new();
        let start = ws[i].start;
        let mut end = ws[i].end;
        let mut cur = Some((v, tens));
        while let Some((mut v, tens)) = cur {
            end = ws[i].end;
            i += 1;
            if tens {
                if let Some((u @ 1..=9, false)) = ws.get(i).and_then(|w| number_word(&w.norm)) {
                    v += u;
                    end = ws[i].end;
                    i += 1;
                }
            }
            digits.push_str(&v.to_string());
            cur = ws.get(i).and_then(|w| number_word(&w.norm));
        }
        out.push(Word {
            start,
            end,
            norm: digits,
        });
    }
    any.then_some(out)
}

/// First letter, then the consonants with h/w/y dropped, c/q as k, z as s,
/// doubles merged: "quillbay" and "kwillbay" -> "klb".
fn skeleton(norm: &str) -> String {
    let mut out = String::new();
    for (i, c) in norm.chars().enumerate() {
        let c = match c {
            'c' | 'q' => 'k',
            'z' => 's',
            c => c,
        };
        if i > 0 && ("aeiouhwy".contains(c) || c.is_ascii_digit()) {
            continue;
        }
        if out.ends_with(c) {
            continue;
        }
        out.push(c);
    }
    out
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut d: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut p = d[0];
        d[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let t = d[j + 1];
            d[j + 1] = (d[j + 1] + 1).min(d[j] + 1).min(p + usize::from(x != y));
            p = t;
        }
    }
    d[b.len()]
}

/// Terms of at most this many letters match only the same letters.
const EXACT_ONLY_LEN: usize = 3;
/// Edit similarity that names a term on its own ("Marbelford").
const NEAR_SIMILARITY: f32 = 0.75;
/// Lower similarity that names a term when the consonant skeletons agree
/// ("sip-sip" / "ZipZip"), for skeletons of at least
/// [`MIN_SKELETON`] letters.
const SKELETON_SIMILARITY: f32 = 0.6;
const MIN_SKELETON: usize = 3;
/// A term this long is named from an opening of [`LONG_OPENING`] letters
/// ("hyperbolic drama" / "Hyperbolicorama"), scored [`LONG_SCORE`].
const LONG_TERM: usize = 12;
const LONG_OPENING: usize = 7;
const LONG_SCORE: f32 = 0.7;

/// A dictionary term, normalised once.
struct Term {
    chars: Vec<char>,
    skel: String,
}

impl Term {
    /// `None` for a term with fewer than 2 letters or digits ("C++", "C#"),
    /// which would otherwise match every standalone letter.
    fn new(norm: &str) -> Option<Self> {
        let chars: Vec<char> = norm.chars().collect();
        (chars.len() >= 2).then(|| Term {
            skel: skeleton(norm),
            chars,
        })
    }
}

/// A run of transcript words joined, normalised once for every term.
struct Window {
    chars: Vec<char>,
    skel: String,
}

impl Window {
    fn new(joined: &str) -> Self {
        Window {
            chars: joined.chars().collect(),
            skel: skeleton(joined),
        }
    }
}

/// How close `win` is to `term`; `None` when not close enough to name.
fn closeness(term: &Term, win: &Window) -> Option<f32> {
    let (t, w) = (&term.chars, &win.chars);
    if w == t {
        return Some(1.0);
    }
    // Short terms: only the same letters count. Otherwise the opening
    // sound must agree: "loom" is not a misheard "eLoom", nor "or a" a
    // misheard "Vo-Ra".
    if t.len() <= EXACT_ONLY_LEN
        || w.len() <= 2
        || win.skel.chars().next() != term.skel.chars().next()
    {
        return None;
    }
    // The skeleton ignores digits, so a term with digits needs the same
    // digits: "velk 50" is not a misheard "Velk42".
    let digits = |s: &[char]| s.iter().filter(|c| c.is_ascii_digit()).collect::<String>();
    if t.iter().any(char::is_ascii_digit) && digits(t) != digits(w) {
        return None;
    }
    let long = t.len().max(w.len());
    if long - t.len().min(w.len()) <= long / 3 + 1 {
        let sim = 1.0 - levenshtein(t, w) as f32 / long as f32;
        if sim >= NEAR_SIMILARITY
            || (sim >= SKELETON_SIMILARITY
                && win.skel.len() >= MIN_SKELETON
                && win.skel == term.skel)
        {
            return Some(sim);
        }
    }
    // A long word cut short or misheard after a clear opening.
    let prefix = t.iter().zip(w).take_while(|(a, b)| a == b).count();
    (t.len() >= LONG_TERM && prefix >= LONG_OPENING).then_some(LONG_SCORE)
}

/// The terms of `terms` worth naming in a prompt for `transcript`, closest
/// first (ties keep dictionary order), at most `cap`. Duplicates (ignoring
/// case) are named once.
pub fn select_terms<'a>(terms: &[&'a str], transcript: &str, cap: usize) -> Vec<&'a str> {
    let plain = words(transcript);
    let digits = with_spoken_digits(&plain);
    // Joined windows of k words, built once per word list and k for every
    // term.
    let mut windows: HashMap<(bool, usize), Vec<Window>> = HashMap::new();
    let mut picked: Vec<(f32, usize, &'a str)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (idx, &term) in terms.iter().enumerate() {
        let tw = words(term);
        let norm: String = tw.iter().map(|w| w.norm.as_str()).collect();
        let has_digit = norm.chars().any(|c| c.is_ascii_digit());
        let Some(t) = Term::new(&norm) else { continue };
        if !seen.insert(norm) {
            continue;
        }
        let mut best: Option<f32> = None;
        // An entry with digits is also matched against the text with its
        // spoken numbers as digits.
        let lists = [
            (false, Some(&plain)),
            (true, digits.as_ref().filter(|_| has_digit)),
        ];
        for (as_digits, ws) in lists {
            let Some(ws) = ws else { continue };
            // Words already spelled as the entry take part in no window.
            let mut exact = vec![false; ws.len()];
            for (i, win) in ws.windows(tw.len()).enumerate() {
                if &transcript[win[0].start..win[tw.len() - 1].end] == term {
                    exact[i..i + tw.len()].iter_mut().for_each(|e| *e = true);
                }
            }
            for k in tw.len().saturating_sub(1).max(1)..=tw.len() + 1 {
                let wins = windows.entry((as_digits, k)).or_insert_with(|| {
                    ws.windows(k)
                        .map(|win| {
                            Window::new(&win.iter().map(|w| w.norm.as_str()).collect::<String>())
                        })
                        .collect()
                });
                for (i, win) in wins.iter().enumerate() {
                    if exact[i..i + k].contains(&true) {
                        continue;
                    }
                    if let Some(c) = closeness(&t, win) {
                        best = Some(best.map_or(c, |b| b.max(c)));
                    }
                }
            }
        }
        if let Some(c) = best {
            picked.push((c, idx, term));
        }
    }
    picked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    picked.truncate(cap);
    picked.into_iter().map(|(_, _, t)| t).collect()
}

/// Is `phrase`, its words joined, one of `terms` in another spelling or a
/// near one? The rules ask before collapsing a repeated word, so that
/// "bop bop" for an entry BopBop reaches the cleanup intact.
pub fn names_a_term(terms: &[&str], phrase: &str) -> bool {
    let joined: String = words(phrase).iter().map(|w| w.norm.as_str()).collect();
    let win = Window::new(&joined);
    terms.iter().any(|term| {
        let norm: String = words(term).iter().map(|w| w.norm.as_str()).collect();
        Term::new(&norm).is_some_and(|t| closeness(&t, &win).is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel<'a>(terms: &[&'a str], t: &str) -> Vec<&'a str> {
        select_terms(terms, t, MAX_PROMPT_TERMS)
    }

    #[test]
    fn names_misspelled_and_respaced_terms() {
        let terms = [
            "Quillmate",
            "Marbleford",
            "Vo-Ra",
            "ZipZip",
            "Kwill Bay",
            "Zentrova",
        ];
        assert_eq!(
            sel(&terms, "I opened quill mate on the train."),
            ["Quillmate"]
        );
        assert_eq!(
            sel(&terms, "Meet me in Marbelford at noon."),
            ["Marbleford"]
        );
        assert_eq!(sel(&terms, "The vora dropped again."), ["Vo-Ra"]);
        assert_eq!(sel(&terms, "Add it to sip-sip for Monday."), ["ZipZip"]);
        assert_eq!(
            sel(&terms, "I dictated this with quill bay."),
            ["Kwill Bay"]
        );
        assert_eq!(sel(&terms, "The zentroba invoice is late."), ["Zentrova"]);
    }

    #[test]
    fn a_long_word_is_named_from_its_opening() {
        let terms = ["Hyperbolicorama"];
        assert_eq!(
            sel(&terms, "That was hyperbolic drama, honestly."),
            ["Hyperbolicorama"]
        );
    }

    #[test]
    fn exact_spellings_and_unrelated_words_name_nothing() {
        let terms = ["Quillmate", "ZipZip", "Kwill Bay", "Gradle", "VEX"];
        assert!(sel(&terms, "Quillmate and ZipZip both synced.").is_empty());
        assert!(sel(&terms, "Kwill Bay typed this.").is_empty());
        assert!(sel(&terms, "We walked to the market and bought gravel.").is_empty());
        // Three letters: only the same letters (another case) count.
        assert!(sel(&terms, "The vet was very kind.").is_empty());
        assert_eq!(sel(&terms, "Open vex and check."), ["VEX"]);
        assert!(sel(&terms, "").is_empty());
        // A near spelling with another opening sound is another word.
        let t = "Her loom was busy, or a friend's was.";
        assert!(sel(&["eLoom", "Vo-Ra"], t).is_empty());
    }

    #[test]
    fn a_wrong_spelling_next_to_a_right_one_is_still_named() {
        assert_eq!(
            sel(&["ZipZip"], "ZipZip, I mean zip zip, is open."),
            ["ZipZip"]
        );
    }

    #[test]
    fn closest_first_capped_and_deduplicated() {
        let terms = ["Brightloom", "Quillmate", "quillmate", "Marbleford"];
        let t = "Use brightlume, quillmate and marbleford.";
        assert_eq!(sel(&terms, t), ["Quillmate", "Marbleford", "Brightloom"]);
        assert_eq!(select_terms(&terms, t, 2), ["Quillmate", "Marbleford"]);
        assert!(select_terms(&terms, t, 0).is_empty());
    }

    #[test]
    fn a_term_of_one_letter_and_symbols_matches_nothing() {
        assert!(sel(&["C++", "C#"], "Plan C is to write it in c.").is_empty());
        assert_eq!(sel(&["C++", "Go2"], "We wrote it in go 2."), ["Go2"]);
    }

    #[test]
    fn an_entry_with_digits_is_named_when_its_number_is_said_as_words() {
        // Made-up entries (D67).
        assert_eq!(sel(&["Zorb9"], "Open zorb nine and check."), ["Zorb9"]);
        assert_eq!(
            sel(&["Velk42"], "The velk forty two build is out."),
            ["Velk42"]
        );
        assert_eq!(
            sel(&["Pelto3"], "Ship it on pelto three tonight."),
            ["Pelto3"]
        );
        // Another number, or the words alone, are another word.
        assert!(sel(&["Velk42"], "The velk fifty build is out.").is_empty());
        assert!(sel(&["Zorb9"], "Two zorbs and nine cats.").is_empty());
        // An entry without digits never matches number words.
        assert!(sel(&["Tenfold"], "Ten folders arrived.").is_empty());
    }

    #[test]
    fn an_entry_with_digits_needs_exactly_its_digits() {
        // Made-up entries (D67). Other digits name nothing.
        assert!(sel(&["Velk42"], "The velk forty three build is out.").is_empty());
        assert!(sel(&["Velk42"], "The velk four build is out.").is_empty());
        // Merged number words ("three twelve" -> 312) match only an entry
        // with exactly those digits.
        assert_eq!(
            sel(&["Rho312"], "Flash rho three twelve first."),
            ["Rho312"]
        );
        assert!(sel(&["Rho31"], "Flash rho three twelve first.").is_empty());
        assert!(sel(&["Rho312"], "Flash rho three eleven first.").is_empty());
        assert!(sel(&["Rho3"], "Flash rho three twelve first.").is_empty());
    }

    #[test]
    fn an_entry_that_is_a_word_plus_a_number_is_named_in_prose_too() {
        // Known limit: "win ten dollars" holds the entry Win10's word and
        // number, so the entry is named, as it already was when the speech
        // model wrote "win 10 dollars" (D64). Naming only lists it on the
        // DICTIONARY line; the prompt keeps a word used in its ordinary
        // sense, so the model decides.
        assert_eq!(sel(&["Win10"], "You could win ten dollars."), ["Win10"]);
        assert_eq!(sel(&["Win10"], "You could win 10 dollars."), ["Win10"]);
        // Without the entry's word nothing is named.
        assert!(sel(&["Win10"], "We got ten dollars back.").is_empty());
    }

    #[test]
    fn spoken_numbers_become_one_digit_word() {
        let norms = |t: &str| -> Vec<String> {
            with_spoken_digits(&words(t))
                .map(|ws| ws.into_iter().map(|w| w.norm).collect())
                .unwrap_or_default()
        };
        assert_eq!(norms("x eighty six"), ["x", "86"]);
        assert_eq!(norms("three twelve now"), ["312", "now"]);
        assert_eq!(norms("forty"), ["40"]);
        assert_eq!(norms("twenty ten"), ["2010"]);
        assert!(norms("no numbers here").is_empty());
    }

    #[test]
    fn a_repeated_word_names_a_term_only_when_joined_it_is_one() {
        assert!(names_a_term(&["BopBop"], "bop bop"));
        assert!(names_a_term(&["Bop Bop"], "bop, bop"));
        assert!(!names_a_term(&["BopBop"], "the the"));
        assert!(!names_a_term(&["Quillmate"], "bop bop"));
        assert!(!names_a_term(&[], "bop bop"));
    }

    #[test]
    fn skeleton_merges_vowels_and_soft_letters() {
        assert_eq!(skeleton("quillbay"), skeleton("kwillbay"));
        assert_eq!(skeleton("sipsip"), skeleton("zipzip"));
        assert_ne!(skeleton("kilobyte"), skeleton("kwillbay"));
    }
}
