//! The spoken "press enter" command (spec §8).

use super::chars::is_word_char;

/// `(text, press_enter)`: `text` without a trailing "press enter", and
/// whether the dictation (cleaned or raw) ended with the command.
pub(super) fn take(text: &str, raw_asr: &str) -> (String, bool) {
    if let Some(rest) = strip_command(text) {
        return (rest, true);
    }
    (text.to_string(), strip_command(raw_asr).is_some())
}

/// `text` without its final "press enter" (with an optional `.`, `!` or
/// `?`), which moves to the text before it when that ends in a letter or
/// digit. `None` when `text` does not end with the command.
fn strip_command(text: &str) -> Option<String> {
    let mut s = text.trim_end();
    let mut mark = None;
    if let Some(c) = s.chars().last().filter(|c| matches!(c, '.' | '!' | '?')) {
        mark = Some(c);
        s = &s[..s.len() - 1];
    }
    let s = strip_suffix_ci(s, "enter")?;
    let gap = s.trim_end();
    if gap.len() == s.len() {
        return None;
    }
    let prefix = strip_suffix_ci(gap, "press")?;
    if prefix.chars().last().is_some_and(is_word_char) {
        return None;
    }
    let prefix = prefix.trim_end();
    let mut out = prefix.to_string();
    if let Some(m) = mark {
        if prefix.chars().last().is_some_and(char::is_alphanumeric) {
            out.push(m);
        }
    }
    Some(out)
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = s.len().checked_sub(suffix.len())?;
    (s.is_char_boundary(cut) && s[cut..].eq_ignore_ascii_case(suffix)).then(|| &s[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(t: &str) -> Option<String> {
        strip_command(t)
    }

    #[test]
    fn removes_the_command_and_moves_its_mark() {
        assert_eq!(strip("Send it press enter.").as_deref(), Some("Send it."));
        assert_eq!(strip("Send it press enter!").as_deref(), Some("Send it!"));
        assert_eq!(strip("Send it press enter?").as_deref(), Some("Send it?"));
        assert_eq!(strip("Send it. Press enter.").as_deref(), Some("Send it."));
        assert_eq!(strip("Send it, press enter").as_deref(), Some("Send it,"));
        assert_eq!(strip("Send it.\nPress enter.").as_deref(), Some("Send it."));
        assert_eq!(strip("Send it PRESS ENTER ").as_deref(), Some("Send it"));
        assert_eq!(strip("press enter").as_deref(), Some(""));
        assert_eq!(strip("Press  enter.").as_deref(), Some(""));
    }

    #[test]
    fn other_phrases_are_not_the_command() {
        for t in [
            "Press enter now.",
            "Send it press the enter.",
            "hit enter",
            "press return",
            "Impress enter.",
            "Send it pressenter.",
            "Send it press center.",
        ] {
            assert_eq!(strip(t), None, "{t}");
        }
    }

    #[test]
    fn raw_only_command_keeps_the_text() {
        assert_eq!(
            take("Send it.", "send it press enter"),
            ("Send it.".to_string(), true)
        );
        assert_eq!(take("Send it.", "send it"), ("Send it.".to_string(), false));
    }
}
