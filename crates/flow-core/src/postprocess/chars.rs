//! Character classes shared by the post-processing steps (spec §5
//! "Definitions").

use unicode_normalization::char::is_combining_mark;

/// A letter, digit or combining mark, in any script: the characters a
/// dictionary key's whole-word match must not touch.
pub(super) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
}

/// `.`, `!`, `?`, `…` and the CJK sentence marks: the ending a selection
/// is read by (§5.6).
pub(super) fn is_sentence_end(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？')
}

/// `"`, `'`, `’`, `”`.
pub(super) fn is_closing_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '’' | '”')
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

/// An `Extended_Pictographic` character, a variation selector U+FE0F, a
/// skin-tone modifier or a regional indicator.
pub(super) fn is_emoji(c: char) -> bool {
    let u = c as u32;
    u == 0xFE0F || (0x1F3FB..=0x1F3FF).contains(&u) || (0x1F1E6..=0x1F1FF).contains(&u) || {
        EXT_PICT
            .binary_search_by(|&(lo, hi)| {
                if hi < u {
                    std::cmp::Ordering::Less
                } else if lo > u {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

/// The `Extended_Pictographic` ranges, sorted.
const EXT_PICT: &[(u32, u32)] = &[
    (0x00A9, 0x00A9),
    (0x00AE, 0x00AE),
    (0x203C, 0x203C),
    (0x2049, 0x2049),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x2199),
    (0x21A9, 0x21AA),
    (0x231A, 0x231B),
    (0x2328, 0x2328),
    (0x2388, 0x2388),
    (0x23CF, 0x23CF),
    (0x23E9, 0x23F3),
    (0x23F8, 0x23FA),
    (0x24C2, 0x24C2),
    (0x25AA, 0x25AB),
    (0x25B6, 0x25B6),
    (0x25C0, 0x25C0),
    (0x25FB, 0x25FE),
    (0x2600, 0x2605),
    (0x2607, 0x2612),
    (0x2614, 0x2685),
    (0x2690, 0x2705),
    (0x2708, 0x2712),
    (0x2714, 0x2714),
    (0x2716, 0x2716),
    (0x271D, 0x271D),
    (0x2721, 0x2721),
    (0x2728, 0x2728),
    (0x2733, 0x2734),
    (0x2744, 0x2744),
    (0x2747, 0x2747),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2763, 0x2767),
    (0x2795, 0x2797),
    (0x27A1, 0x27A1),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2934, 0x2935),
    (0x2B05, 0x2B07),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x3030, 0x3030),
    (0x303D, 0x303D),
    (0x3297, 0x3297),
    (0x3299, 0x3299),
    (0x1F000, 0x1F0FF),
    (0x1F10D, 0x1F10F),
    (0x1F12F, 0x1F12F),
    (0x1F16C, 0x1F171),
    (0x1F17E, 0x1F17F),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1AD, 0x1F1E5),
    (0x1F201, 0x1F20F),
    (0x1F21A, 0x1F21A),
    (0x1F22F, 0x1F22F),
    (0x1F232, 0x1F23A),
    (0x1F23C, 0x1F23F),
    (0x1F249, 0x1F3FA),
    (0x1F400, 0x1F53D),
    (0x1F546, 0x1F64F),
    (0x1F680, 0x1F6FF),
    (0x1F774, 0x1F77F),
    (0x1F7D5, 0x1F7FF),
    (0x1F80C, 0x1F80F),
    (0x1F848, 0x1F84F),
    (0x1F85A, 0x1F85F),
    (0x1F888, 0x1F88F),
    (0x1F8AE, 0x1F8FF),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1FAFF),
    (0x1FC00, 0x1FFFD),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_class() {
        for c in [
            '😀',
            '©',
            '™',
            '★',
            '✅',
            '❤',
            '\u{FE0F}',
            '\u{1F3FD}',
            '\u{1F1EC}',
        ] {
            assert!(is_emoji(c), "{c:?}");
        }
        for c in ['°', 'a', '.', '…', '—'] {
            assert!(!is_emoji(c), "{c:?}");
        }
    }
}
