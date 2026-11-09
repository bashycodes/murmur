//! Character classes shared by the post-processing steps.

use unicode_normalization::char::is_combining_mark;

/// A letter, digit or combining mark, in any script: the characters a
/// dictionary key's whole-word match must not touch.
pub(super) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
}

/// `.`, `!`, `?`, `…` and the CJK sentence marks.
pub(super) fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？')
}

/// Han, Hiragana, Katakana, Hangul, CJK punctuation and full-width forms.
pub(super) fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x1100..=0x11FF       // Hangul Jamo
            | 0x2E80..=0x2FDF // CJK radicals
            | 0x3000..=0x303F // CJK symbols and punctuation
            | 0x3040..=0x309F // Hiragana
            | 0x30A0..=0x30FF // Katakana
            | 0x3130..=0x318F // Hangul compatibility Jamo
            | 0x31F0..=0x31FF // Katakana phonetic extensions
            | 0x3400..=0x4DBF // Han extension A
            | 0x4E00..=0x9FFF // Han
            | 0xA960..=0xA97F // Hangul Jamo extended A
            | 0xAC00..=0xD7AF // Hangul syllables
            | 0xD7B0..=0xD7FF // Hangul Jamo extended B
            | 0xF900..=0xFAFF // Han compatibility
            | 0xFF00..=0xFFEF // half-width and full-width forms
            | 0x20000..=0x323AF // Han supplementary planes
    )
}

/// Marks after which a dictation starts without a space.
pub(super) fn is_opening_mark(c: char) -> bool {
    matches!(
        c,
        '(' | '[' | '{' | '"' | '\'' | '“' | '‘' | '«' | '¿' | '¡'
    )
}

/// `text` without its final run of sentence-end characters (and the
/// whitespace that run leaves at the end).
pub(super) fn strip_sentence_end(text: &str) -> &str {
    text.trim_end_matches(is_sentence_end).trim_end()
}
