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

/// D65: what `answer`, a cleanup of `source` (in `locked`, non-Latin),
/// adds that `source` does not have: a letter of another non-Latin script
/// (Arabic, Cyrillic, Greek, CJK, …), a CJK or full-width mark, or U+FFFD;
/// for a Hebrew source also a Latin word (two letters or more, any case)
/// that is not in `source` or `known` (the prompt's dictionary and screen
/// words) and does not spell a Hebrew word of `source` ("בסלאק" →
/// "ב-Slack", [`spells`]). [`drifted_from`] misses these: the answer stays
/// mostly `locked` (Qwen2.5-14B, 2026-11-12: Chinese, Cyrillic, Arabic
/// and Turkish words inside Hebrew lines). `None`: nothing added.
pub fn added_foreign(locked: Script, source: &str, answer: &str, known: &[&str]) -> Option<String> {
    let class = |c: char| -> Option<Option<Script>> {
        let cp = c as u32;
        if c == '\u{FFFD}' || (0x3000..=0x303F).contains(&cp) || (0xFF00..=0xFFEF).contains(&cp) {
            return Some(None);
        }
        if !c.is_alphabetic() {
            return None;
        }
        match Script::of(c) {
            Some(s) if s == locked || s == Script::Latin => None,
            s => Some(s),
        }
    };
    let had: Vec<Option<Script>> = source.chars().filter_map(class).collect();
    if let Some(c) = answer
        .chars()
        .find(|c| class(*c).is_some_and(|k| !had.contains(&k)))
    {
        return Some(format!("answer adds {c:?}"));
    }
    if locked != Script::Hebrew {
        return None;
    }
    let mut kept: Vec<String> = latin_words(source).collect();
    kept.extend(known.iter().flat_map(|k| latin_words(k)));
    let hebrew: Vec<&str> = source
        .split(|c: char| Script::of(c) != Some(Script::Hebrew) && c != '\'' && c != '\u{05F3}')
        .filter(|w| !w.is_empty())
        .collect();
    latin_words(answer)
        .find(|w| !kept.contains(w) && !hebrew.iter().any(|h| spells(w, h)))
        .map(|w| format!("answer adds {w:?}"))
}

/// Lowercase runs of two or more Latin letters.
fn latin_words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| Script::of(c) != Some(Script::Latin))
        .filter(|w| w.chars().count() >= 2)
        .map(str::to_lowercase)
}

/// Consonant classes of a transliteration: a Hebrew letter and the Latin
/// spellings it is written with. Vowel letters (א ה ו י ע, a e i o u y h
/// w) are not compared.
fn hebrew_class(c: char) -> Option<u8> {
    Some(match c {
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
        _ => return None,
    })
}

/// The consonant classes of a lowercase Latin word, each with the Hebrew
/// classes it may be written as; `None` for a letter outside ASCII.
fn latin_classes(w: &str) -> Option<Vec<&'static [u8]>> {
    let b = w.as_bytes();
    if !w.is_ascii() {
        return None;
    }
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
            (b'a' | b'e' | b'i' | b'o' | b'u' | b'y' | b'h' | b'w', _) => (b"", 1),
            (b'b' | b'v', _) => (b"b", 1),
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

/// Does Latin word `w` (lowercase) spell Hebrew word `h`, perhaps after
/// one or two one-letter prefixes (ב ה ו כ ל מ ש: "בסלאק" → "Slack",
/// "לדרייב" → "Drive")? Compared by consonants, two at least.
fn spells(w: &str, h: &str) -> bool {
    let Some(lat) = latin_classes(w).filter(|l| l.len() >= 2) else {
        return false;
    };
    let skeleton = |s: &str| {
        let mut v: Vec<u8> = Vec::new();
        for k in s.chars().filter_map(hebrew_class) {
            if v.last() != Some(&k) {
                v.push(k);
            }
        }
        v
    };
    let mut rest = h;
    for _ in 0..3 {
        let he = skeleton(rest);
        if he.len() == lat.len() && he.iter().zip(&lat).all(|(k, alt)| alt.contains(k)) {
            return true;
        }
        match rest.chars().next() {
            Some(p @ ('ב' | 'ה' | 'ו' | 'כ' | 'ל' | 'מ' | 'ש')) => {
                rest = &rest[p.len_utf8()..]
            }
            _ => break,
        }
    }
    false
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
    }

    #[test]
    fn lock_lines_name_the_language() {
        assert!(lock_line(Script::Hebrew).contains("in Hebrew"));
        assert!(retry_line(Script::Hebrew).contains("Hebrew only"));
    }
}
