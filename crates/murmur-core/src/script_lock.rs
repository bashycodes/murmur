//! Language lock for edits of existing text (Command Mode, transforms):
//! the answer keeps the writing system of the text it replaces unless the
//! instruction asks for another language. Dictation cleanup checks its
//! answer with it too (D65): a cleaned dictation keeps its transcript's
//! script.
//!
//! Small on-device models (Qwen2.5-1.5B) drift to English when the
//! instruction ("make this more formal") and the few-shot example are
//! English but the selected text is Hebrew (2026-11-01 owner report). The
//! script of the text is a reliable, model-free signal: a Hebrew selection
//! rewritten into Latin letters changed language. For a Latin-script
//! selection the language can't be told from the letters (English vs
//! Spanish), so nothing is locked there.

/// A writing system the lock can tell apart by Unicode block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Latin,
    Hebrew,
    Arabic,
    Cyrillic,
    Greek,
}

impl Script {
    fn of(c: char) -> Option<Script> {
        if !c.is_alphabetic() {
            return None;
        }
        Some(match c as u32 {
            0x0590..=0x05FF | 0xFB1D..=0xFB4F => Script::Hebrew,
            0x0600..=0x06FF
            | 0x0750..=0x077F
            | 0x08A0..=0x08FF
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF => Script::Arabic,
            0x0400..=0x052F => Script::Cyrillic,
            0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
            0x0041..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
            _ => return None,
        })
    }

    /// How the prompt names the language to keep.
    pub fn language(self) -> &'static str {
        match self {
            Script::Latin => "the language of the selected text",
            Script::Hebrew => "Hebrew",
            Script::Arabic => "the same Arabic-script language as the text",
            Script::Cyrillic => "the same Cyrillic-script language as the text",
            Script::Greek => "Greek",
        }
    }
}

/// Fewest letters for a script to be judged at all.
const MIN_LETTERS: usize = 3;
/// Share of the letters a script needs to be the text's script.
const DOMINANT: f32 = 0.6;
/// An answer with less than this share of the source's letters drifted.
const KEPT: f32 = 0.5;

fn letters(text: &str) -> impl Iterator<Item = Script> + '_ {
    text.chars().filter_map(Script::of)
}

/// Share of `text`'s letters (of the scripts above) written in `s`;
/// `None` with fewer than [`MIN_LETTERS`] letters.
pub fn share(text: &str, s: Script) -> Option<f32> {
    let (mut n, mut hit) = (0usize, 0usize);
    for l in letters(text) {
        n += 1;
        hit += (l == s) as usize;
    }
    (n >= MIN_LETTERS).then(|| hit as f32 / n as f32)
}

/// The script most of `text`'s letters are in, if one clearly dominates.
pub fn dominant(text: &str) -> Option<Script> {
    let all = [
        Script::Latin,
        Script::Hebrew,
        Script::Arabic,
        Script::Cyrillic,
        Script::Greek,
    ];
    all.into_iter()
        .find(|s| share(text, *s).is_some_and(|x| x >= DOMINANT))
}

/// Words that ask for a change of language ("translate", "in English",
/// "באנגלית"). A mention of the source's own language ("keep it in
/// Hebrew") is not a change.
const TRANSLATE: [&str; 8] = [
    "translat",
    "transliterat",
    "תרגם",
    "תתרגם",
    "לתרגם",
    "תרגום",
    "ترجم",
    "перевед",
];

/// (lowercase names of a language, the script it is written in).
const LANGUAGES: [(&[&str], Option<Script>); 16] = [
    (
        &["english", "אנגלית", "انجليزي", "الإنجليزية", "англий"],
        None,
    ),
    (&["hebrew", "עברית", "עיברית"], Some(Script::Hebrew)),
    (
        &["arabic", "ערבית", "عربي", "العربية"],
        Some(Script::Arabic),
    ),
    (&["persian", "farsi", "פרסית"], Some(Script::Arabic)),
    (&["urdu"], Some(Script::Arabic)),
    (&["russian", "רוסית", "русск"], Some(Script::Cyrillic)),
    (&["ukrainian", "אוקראינית"], Some(Script::Cyrillic)),
    (&["greek", "יוונית"], Some(Script::Greek)),
    (&["yiddish", "יידיש"], Some(Script::Hebrew)),
    (&["spanish", "ספרדית"], None),
    (&["french", "צרפתית"], None),
    (&["german", "גרמנית"], None),
    (&["italian", "איטלקית"], None),
    (&["portuguese", "פורטוגזית"], None),
    (
        &[
            "chinese",
            "japanese",
            "korean",
            "סינית",
            "יפנית",
            "קוריאנית",
        ],
        None,
    ),
    (&["latin letters", "latin script", "אותיות לטיניות"], None),
];

/// Does `instruction` ask for the text in another language than its
/// `source` script (translate, "in English", "write it in Arabic")?
pub fn asks_language_change(instruction: &str, source: Script) -> bool {
    let i = instruction.to_lowercase();
    if TRANSLATE.iter().any(|w| i.contains(w)) {
        return true;
    }
    LANGUAGES
        .iter()
        .any(|(names, script)| names.iter().any(|n| i.contains(n)) && *script != Some(source))
}

/// The script an edit of `text` must keep under `instruction`: a non-Latin
/// script that dominates `text`, unless the instruction asks for another
/// language.
pub fn lock_for(text: &str, instruction: &str) -> Option<Script> {
    dominant(text).filter(|s| *s != Script::Latin && !asks_language_change(instruction, *s))
}

/// Did `answer` leave `locked`? An answer without letters (digits, an
/// emoji) never did.
pub fn drifted(locked: Script, answer: &str) -> bool {
    share(answer, locked).is_some_and(|x| x < KEPT)
}

/// D65: did `answer`, a cleanup of `source`, leave `locked`? Its share of
/// `locked` letters is under [`KEPT`] times the source's share. Relative,
/// because a correct cleanup of a mixed dictation writes transliterated
/// names in Latin letters ("בסלאק" → "ב-Slack") and can fall under half
/// Hebrew while the source was barely over [`DOMINANT`]; a translation
/// keeps almost none (0 % on every translated probe answer). For a source
/// all in `locked` this is [`drifted`]. No letters on either side: no
/// verdict.
pub fn drifted_from(locked: Script, source: &str, answer: &str) -> bool {
    match (share(source, locked), share(answer, locked)) {
        (Some(src), Some(ans)) => ans < KEPT * src,
        _ => false,
    }
}

/// What [`added_foreign`] counts as another writing system in an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Foreign {
    /// A letter of a non-Latin script the lock knows, other than the locked one.
    Script(Script),
    /// Any other letter (CJK, …), a CJK or full-width mark, or U+FFFD.
    Other,
}

impl Foreign {
    fn of(c: char, locked: Script) -> Option<Foreign> {
        let cp = c as u32;
        if c == '\u{FFFD}' || (0x3000..=0x303F).contains(&cp) || (0xFF00..=0xFFEF).contains(&cp) {
            return Some(Foreign::Other);
        }
        if !c.is_alphabetic() {
            return None;
        }
        match Script::of(c) {
            Some(s) if s == locked || s == Script::Latin => None,
            Some(s) => Some(Foreign::Script(s)),
            None => Some(Foreign::Other),
        }
    }
}

/// D65: what `answer`, a cleanup of `source` (in `locked`, non-Latin),
/// adds that `source` does not have: a letter of another non-Latin script
/// (Arabic, Cyrillic, Greek, CJK, …), a CJK or full-width mark, or U+FFFD;
/// for a Hebrew source also a Latin word (two letters or more, any case)
/// that is not in `source` or `known` (the whole dictionary vocabulary and
/// the screen words, whether or not the prompt showed them) and is not
/// written in `source` in Hebrew letters: a word or two adjacent words
/// that [`spells`] it ("בסלאק" → "ב-Slack", "צ'אט ג'יפיטי" → "ChatGPT"),
/// or, for an all-caps word of 2–5 letters, its letters' English names
/// ("איי פי איי" → "API", "אוקיי" → "OK", [`spells_letters`]).
/// [`drifted_from`] misses these: the answer stays mostly `locked`
/// (Qwen2.5-14B, 2026-11-12: Chinese, Cyrillic, Arabic and Turkish words
/// inside Hebrew lines). `None`: nothing added. Dictation cleanup only:
/// Command Mode and transforms check [`drifted`] and not this yet.
pub fn added_foreign(locked: Script, source: &str, answer: &str, known: &[&str]) -> Option<String> {
    let had: Vec<Foreign> = source
        .chars()
        .filter_map(|c| Foreign::of(c, locked))
        .collect();
    if let Some(c) = answer
        .chars()
        .find(|c| Foreign::of(*c, locked).is_some_and(|k| !had.contains(&k)))
    {
        return Some(format!("answer adds {c:?}"));
    }
    if locked != Script::Hebrew {
        return None;
    }
    let mut kept: Vec<String> = latin_words(source).map(str::to_lowercase).collect();
    kept.extend(
        known
            .iter()
            .flat_map(|k| latin_words(k))
            .map(str::to_lowercase),
    );
    let hebrew: Vec<String> = source
        .split(|c: char| Script::of(c) != Some(Script::Hebrew) && c != '\'' && c != '\u{05F3}')
        .map(plain_hebrew)
        .filter(|w| !w.is_empty())
        .collect();
    let pairs: Vec<String> = hebrew.windows(2).map(|p| p.concat()).collect();
    latin_words(answer)
        .find(|w| {
            let lw = w.to_lowercase();
            !kept.contains(&lw)
                && !hebrew.iter().chain(&pairs).any(|h| spells(&lw, h))
                && !spells_letters(w, &hebrew)
        })
        .map(|w| format!("answer adds {w:?}"))
}

/// Runs of two or more Latin letters, as written.
fn latin_words(text: &str) -> impl Iterator<Item = &str> + '_ {
    text.split(|c: char| Script::of(c) != Some(Script::Latin))
        .filter(|w| w.chars().count() >= 2)
}

/// A Hebrew word compared by its letters: no geresh, no points, final
/// forms as the regular letter.
fn plain_hebrew(w: &str) -> String {
    w.chars()
        .filter_map(|c| match c {
            'ך' => Some('כ'),
            'ם' => Some('מ'),
            'ן' => Some('נ'),
            'ף' => Some('פ'),
            'ץ' => Some('צ'),
            'א'..='ת' => Some(c),
            _ => None,
        })
        .collect()
}

/// `h`, then `h` without one and two leading one-letter prefixes
/// (ב ה ו כ ל מ ש) while it has them.
fn unprefixed(h: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(h);
    let mut n = 0;
    std::iter::from_fn(move || {
        let cur = rest?;
        n += 1;
        rest = match cur.chars().next() {
            Some(p @ ('ב' | 'ה' | 'ו' | 'כ' | 'ל' | 'מ' | 'ש')) if n <= 2 => {
                Some(&cur[p.len_utf8()..])
            }
            _ => None,
        };
        Some(cur)
    })
}

/// How an English letter's name is written in Hebrew letters (as
/// [`plain_hebrew`] leaves it: no geresh, no final forms).
fn letter_names(c: char) -> &'static [&'static str] {
    match c {
        'A' | 'I' => &["איי", "אי"],
        'B' => &["בי", "ביי"],
        'C' => &["סי"],
        'D' => &["די"],
        'E' => &["אי", "איי"],
        'F' => &["אפ", "עפ"],
        'G' => &["גי"],
        'H' => &["אייצ", "הייצ", "אייטש"],
        'J' => &["גיי", "גי"],
        'K' => &["קיי", "קי"],
        'L' => &["אל", "אאל"],
        'M' => &["אמ", "אאמ"],
        'N' => &["אנ", "אאנ"],
        'O' => &["או", "אוו"],
        'P' => &["פי"],
        'Q' => &["קיו"],
        'R' => &["אר", "ער", "אאר"],
        'S' => &["אס", "עס", "אאס"],
        'T' => &["טי"],
        'U' => &["יו"],
        'V' => &["וי", "ווי"],
        'W' => &["דאבליו", "דבליו"],
        'X' => &["אקס"],
        'Y' => &["וואי", "וויי"],
        'Z' => &["זי", "זד"],
        _ => &[],
    }
}

/// Is all of `h` the names of `letters`, in order?
fn names_spell(h: &str, letters: &[char]) -> bool {
    match letters.split_first() {
        None => h.is_empty(),
        Some((c, rest)) => letter_names(*c)
            .iter()
            .any(|n| h.strip_prefix(n).is_some_and(|t| names_spell(t, rest))),
    }
}

/// Is `w`, an all-caps word of 2–5 letters, said letter by letter in
/// `hebrew` (the source's words, [`plain_hebrew`]): up to one adjacent
/// word per letter, joined ("איי פי איי" → "API", "אוקיי" → "OK"), the
/// first one perhaps with a prefix ("בפי אר" → "ב-PR")?
fn spells_letters(w: &str, hebrew: &[String]) -> bool {
    let letters: Vec<char> = w.chars().collect();
    if !(2..=5).contains(&letters.len()) || !letters.iter().all(char::is_ascii_uppercase) {
        return false;
    }
    (0..hebrew.len()).any(|i| {
        (1..=letters.len().min(hebrew.len() - i)).any(|k| {
            let tail = hebrew[i + 1..i + k].concat();
            unprefixed(&hebrew[i]).any(|head| names_spell(&format!("{head}{tail}"), &letters))
        })
    })
}

/// Consonant classes of a transliteration: a Hebrew letter and the Latin
/// spellings it is written with. Vowel letters (א ה י ע and a single ו)
/// are not compared; a doubled וו is v or w ("ווייז" → "Waze").
fn hebrew_skeleton(h: &str) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::new();
    let mut chars = h.chars().peekable();
    while let Some(c) = chars.next() {
        let k = match c {
            'ו' if chars.peek() == Some(&'ו') => {
                chars.next();
                b'w'
            }
            'ב' => b'b',
            'ג' => b'g',
            'ד' => b'd',
            'ז' => b'z',
            'ח' => b'h',
            'ט' | 'ת' => b't',
            'כ' | 'ך' | 'ק' => b'k',
            'ל' => b'l',
            'מ' | 'ם' => b'm',
            'נ' | 'ן' => b'n',
            'ס' | 'ש' => b's',
            'פ' | 'ף' => b'p',
            'צ' | 'ץ' => b'c',
            'ר' => b'r',
            _ => continue,
        };
        if v.last() != Some(&k) {
            v.push(k);
        }
    }
    v
}

/// The consonant classes of a lowercase Latin word, each with the Hebrew
/// classes it may be written as; `None` for a letter outside ASCII. Not
/// compared: a e i o u y h, and a w before no vowel.
fn latin_classes(w: &str) -> Option<Vec<&'static [u8]>> {
    let b = w.as_bytes();
    if !w.is_ascii() {
        return None;
    }
    let vowel = |c: Option<u8>| matches!(c, Some(b'a' | b'e' | b'i' | b'o' | b'u' | b'y'));
    let mut out: Vec<&'static [u8]> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let next = b.get(i + 1).copied();
        let (k, step): (&'static [u8], usize) = match (b[i], next) {
            (b'c', Some(b'k')) => (b"k", 2),
            (b'c', Some(b'h')) => (b"khc", 2),
            (b'k', Some(b'h')) => (b"hk", 2),
            (b's', Some(b'h')) => (b"s", 2),
            (b'p', Some(b'h')) => (b"p", 2),
            (b't', Some(b'h')) => (b"t", 2),
            (b'c', Some(b'e' | b'i' | b'y')) => (b"s", 1),
            (b'w', Some(b'h')) => (b"w", 2),
            (b'w', n) if vowel(n) => (b"w", 1),
            (b'a' | b'e' | b'i' | b'o' | b'u' | b'y' | b'h' | b'w', _) => (b"", 1),
            (b'b', _) => (b"b", 1),
            (b'v', _) => (b"bw", 1),
            (b'c' | b'k' | b'q', _) => (b"k", 1),
            (b'f' | b'p', _) => (b"p", 1),
            (b'j', _) => (b"gz", 1),
            (b's', _) => (b"sz", 1),
            (b'z', _) => (b"zc", 1),
            (b'x', _) => {
                out.extend([b"k" as &[u8], b"s"]);
                i += 1;
                continue;
            }
            (c, _) => (
                match c {
                    b'd' => b"d",
                    b'g' => b"g",
                    b'l' => b"l",
                    b'm' => b"m",
                    b'n' => b"n",
                    b'r' => b"r",
                    b't' => b"t",
                    _ => return None,
                },
                1,
            ),
        };
        if !k.is_empty() && out.last() != Some(&k) {
            out.push(k);
        }
        i += step;
    }
    Some(out)
}

/// Does Latin word `w` (lowercase) spell Hebrew word `h` ([`plain_hebrew`]),
/// perhaps after one or two one-letter prefixes ("בסלאק" → "Slack",
/// "לדרייב" → "Drive")? Compared by consonants, two at least. Not matched:
/// a v or w written with a single ו ("ויקיפדיה" → "Wikipedia").
fn spells(w: &str, h: &str) -> bool {
    let Some(lat) = latin_classes(w).filter(|l| l.len() >= 2) else {
        return false;
    };
    unprefixed(h).any(|rest| {
        let he = hebrew_skeleton(rest);
        he.len() == lat.len() && he.iter().zip(&lat).all(|(k, alt)| alt.contains(k))
    })
}

/// The prompt line that locks the language (appended to the user turn,
/// where small models weigh it most).
pub fn lock_line(s: Script) -> String {
    format!(
        "Write the result in {}, the language of the selected text, even though the instruction \
         is in another language. Do not translate it unless the instruction explicitly asks to.",
        s.language()
    )
}

/// The stronger line for the one retry after a drifted answer.
pub fn retry_line(s: Script) -> String {
    format!(
        "Important: your answer must be in {} only, like the selected text. Do not answer in \
         English and do not translate.",
        s.language()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HE: &str = "הפגישה נדחתה ליום שלישי";

    #[test]
    fn dominant_script_by_letters() {
        assert_eq!(dominant(HE), Some(Script::Hebrew));
        assert_eq!(
            dominant("The meeting moved to Tuesday."),
            Some(Script::Latin)
        );
        assert_eq!(dominant("Встреча перенесена"), Some(Script::Cyrillic));
        assert_eq!(dominant("تم تأجيل الاجتماع"), Some(Script::Arabic));
        // A Hebrew sentence with one English product name is still Hebrew.
        assert_eq!(
            dominant("נדבר מחר בזום על הדוח של Q3"),
            Some(Script::Hebrew)
        );
        // Too few letters, or an even mix: no verdict.
        assert_eq!(dominant("12:30"), None);
        assert_eq!(dominant("ok"), None);
        assert_eq!(dominant("שלום hello"), None);
    }

    #[test]
    fn only_explicit_language_changes_unlock() {
        for i in [
            "make it more formal",
            "shorten this",
            "fix the grammar",
            "keep it in Hebrew but make it polite",
            "תנסח את זה יותר רשמי",
            "turn this into a list",
        ] {
            assert!(!asks_language_change(i, Script::Hebrew), "{i}");
            assert_eq!(lock_for(HE, i), Some(Script::Hebrew), "{i}");
        }
        for i in [
            "translate this to English",
            "Translate to Spanish",
            "write it in English",
            "rewrite this in Russian",
            "תתרגם לאנגלית",
            "תכתוב את זה באנגלית",
            "transliterate it",
        ] {
            assert!(asks_language_change(i, Script::Hebrew), "{i}");
            assert_eq!(lock_for(HE, i), None, "{i}");
        }
    }

    #[test]
    fn latin_text_is_never_locked() {
        assert_eq!(lock_for("hey, can't make it", "make it formal"), None);
        assert_eq!(lock_for("12 3", "make it formal"), None);
    }

    #[test]
    fn drift_is_a_change_of_script() {
        assert!(drifted(
            Script::Hebrew,
            "The meeting was postponed to Tuesday."
        ));
        assert!(!drifted(Script::Hebrew, "הפגישה נדחתה ליום שלישי הקרוב."));
        assert!(!drifted(Script::Hebrew, "הדוח של Q3 יוצג ב-Zoom מחר."));
        assert!(!drifted(Script::Hebrew, "3/11"), "no letters, no verdict");
    }

    #[test]
    fn drift_from_a_source_is_relative_to_its_share() {
        let mixed = "תעלה את ה release notes לדרייב ותשלח בסלאק";
        let latinised = "תעלה release notes לדרייב ותשלח ב-Slack.";
        assert!(share(latinised, Script::Hebrew).unwrap() < KEPT);
        assert!(!drifted_from(Script::Hebrew, mixed, latinised));
        assert!(drifted_from(
            Script::Hebrew,
            mixed,
            "Upload the release notes to Drive and send on Slack."
        ));
        assert!(!drifted_from(Script::Hebrew, mixed, "3/11"));
    }

    // Made-up lines. Each junk answer stays mostly Hebrew, so
    // `drifted_from` passes it.
    #[test]
    fn an_answer_that_adds_another_script_or_a_new_latin_word_is_caught() {
        let src = "תזכיר לי לעבור בסופר אחרי העבודה ולקנות עגבניות";
        let added = |a: &str| added_foreign(Script::Hebrew, src, a, &[]);
        for junk in [
            "תזכיר לי לעבור בסופר אחרי העבודה ולקנות помидоры.",
            "תזכיר לי לעבור בסופר بعد העבודה ולקנות עגבניות.",
            "תזכיר לי לעבור בסופר אחרי העבודה ולקנות 番茄。",
            "תזכיר לי לעבור בסופר אחרי העבודה ולקנות עגבני\u{FFFD}ת.",
            "תזכיר לי לעבור בסופר after העבודה ולקנות עגבניות.",
            "תזכיר לי לעבור בסופר אחרי העבודה ולקנות domatesler.",
            "תזכיר לי לעבור בסופר ağaç העבודה ולקנות עגבניות.",
        ] {
            assert!(!drifted_from(Script::Hebrew, src, junk), "{junk}");
            assert!(added(junk).is_some(), "{junk}");
        }
        for clean in [
            "תזכיר לי לעבור בסופר אחרי העבודה ולקנות עגבניות.",
            "תזכיר לי לעבור בסופר אחרי העבודה, ולקנות 2 קילו עגבניות!",
            "3/11",
        ] {
            assert_eq!(added(clean), None, "{clean}");
        }
    }

    #[test]
    fn latin_words_of_the_dictation_or_its_dictionary_are_kept() {
        let src = "תשלח את ה pdf לדנה בזום לפני ה standup";
        let a = "תשלח את ה-PDF לדנה ב-Zoom לפני ה-standup.";
        assert_eq!(added_foreign(Script::Hebrew, src, a, &[]), None);
        // Transliterated in the dictation, written in Latin letters.
        let src = "תעלה את הקובץ לדרייב ותשלח בסלאק לצוות";
        let a = "תעלה את הקובץ ל-Drive ותשלח ב-Slack לצוות.";
        assert_eq!(added_foreign(Script::Hebrew, src, a, &[]), None);
        // A dictionary or screen word the prompt showed the model.
        let src = "שלחתי את זה לצוות של זפיר";
        let a = "שלחתי את זה לצוות של Zephyr.";
        assert_eq!(added_foreign(Script::Hebrew, src, a, &["Zephyr"]), None);
        assert!(added_foreign(Script::Hebrew, src, "שלחתי את זה לצוות של Orbit.", &[]).is_some());
        // A word of another script the dictation already had.
        let src = "הוא אמר спасибо ויצא מהחדר";
        let a = "הוא אמר спасибо ויצא מהחדר.";
        assert_eq!(added_foreign(Script::Hebrew, src, a, &[]), None);
    }

    #[test]
    fn spelling_compares_consonants_after_prefixes() {
        assert!(spells("slack", "בסלאק"));
        assert!(spells("drive", "לדרייב"));
        assert!(spells("zoom", "ובזום"));
        assert!(spells("chrome", "כרום"));
        assert!(!spells("after", "אחרי"));
        assert!(!spells("ok", "אוקיי"), "one consonant: no verdict");
        assert!(!spells("ağaç", "אגץ"));
        // A doubled וו is v or w; a single ו is a vowel.
        assert!(spells("waze", "ווייז"));
        assert!(spells("vodafone", "בוודאפון"));
        assert!(spells("twitter", "טוויטר"));
        assert!(!spells("wikipedia", "ויקיפדיה"), "single ו: not matched");
    }

    // Made-up lines: an acronym said by its letters' English names, as
    // separate words or one word, is the dictation's own word.
    #[test]
    fn an_acronym_said_letter_by_letter_is_kept() {
        for (src, answer) in [
            (
                "תבדוק את ה איי פי איי של השירות",
                "תבדוק את ה-API של השירות.",
            ),
            ("תפתח פי אר על התיקון הזה", "תפתח PR על התיקון הזה."),
            ("שלחו לי אס אם אס עם הקוד", "שלחו לי SMS עם הקוד."),
            ("הוא עובד על כלי איי איי חדש", "הוא עובד על כלי AI חדש."),
            (
                "הסי אי או של החברה דיבר אתמול",
                "ה-CEO של החברה דיבר אתמול.",
            ),
            ("אוקיי אז נדבר מחר", "OK, אז נדבר מחר."),
            ("נשלח לך בפי די אף", "נשלח לך ב-PDF."),
        ] {
            assert_eq!(
                added_foreign(Script::Hebrew, src, answer, &[]),
                None,
                "{answer}"
            );
        }
        let src = "תבדוק את הממשק של השירות";
        for junk in ["תבדוק את ה-API של השירות.", "תבדוק את Api של השירות."]
        {
            assert!(
                added_foreign(Script::Hebrew, src, junk, &[]).is_some(),
                "{junk}"
            );
        }
        // Lowercase junk is never an acronym, whatever the dictation says.
        let src = "שילמתי תשע עשרה מאות שקל אוקיי";
        let junk = "שילמתי nineteen מאות שקל, אוקיי.";
        assert!(added_foreign(Script::Hebrew, src, junk, &[]).is_some());
        let junk = "שילמתי geçerer מאות שקל, אוקיי.";
        assert!(added_foreign(Script::Hebrew, src, junk, &[]).is_some());
    }

    #[test]
    fn a_name_said_as_two_words_is_kept() {
        let src = "תשאל את צ'אט ג'יפיטי מה הוא חושב";
        let a = "תשאל את ChatGPT מה הוא חושב.";
        assert_eq!(added_foreign(Script::Hebrew, src, a, &[]), None);
        let src = "תפתח את ווייז ותבדוק אם יש פקקים";
        let a = "תפתח את Waze ותבדוק אם יש פקקים.";
        assert_eq!(added_foreign(Script::Hebrew, src, a, &[]), None);
    }

    #[test]
    fn lock_lines_name_the_language() {
        assert!(lock_line(Script::Hebrew).contains("in Hebrew"));
        assert!(retry_line(Script::Hebrew).contains("Hebrew only"));
    }
}
