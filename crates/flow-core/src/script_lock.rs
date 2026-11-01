//! Language lock for edits of existing text (Command Mode, transforms):
//! the answer keeps the writing system of the text it replaces unless the
//! instruction asks for another language.
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
    fn lock_lines_name_the_language() {
        assert!(lock_line(Script::Hebrew).contains("in Hebrew"));
        assert!(retry_line(Script::Hebrew).contains("Hebrew only"));
    }
}
