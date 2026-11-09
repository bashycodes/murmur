//! Style punctuation (spec §6).

use crate::config::{FieldContext, Style};

/// `text` with its final `.` dropped when the style asks for it. `chat`:
/// the listed category is a chat app.
pub(super) fn punctuate(text: &str, style: Style, chat: bool, ctx: &FieldContext) -> String {
    let drop = !suppressed(ctx)
        && match style {
            Style::Formal => false,
            Style::Casual => sentence_count(text) <= 10,
            Style::VeryCasual => true,
            Style::Default | Style::Excited => chat && sentence_count(text) <= 2,
        };
    match text.strip_suffix('.') {
        Some(rest) if drop && !rest.ends_with('.') => rest.to_string(),
        _ => text.to_string(),
    }
}

/// Pieces between `.`, `!` and `?` that are not empty after trimming.
fn sentence_count(text: &str) -> usize {
    text.split(['.', '!', '?'])
        .filter(|s| !s.trim().is_empty())
        .count()
}

/// The current line of `before` or `after`, or the selection, holds a
/// sentence mark: the field already reads as full sentences.
fn suppressed(ctx: &FieldContext) -> bool {
    let line_before = ctx.before.rsplit('\n').next().unwrap_or("");
    let line_after = ctx.after.split('\n').next().unwrap_or("");
    [line_before, line_after, ctx.selected.as_str()]
        .iter()
        .any(|s| s.contains(['.', '!', '?']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Style::*;

    const ALL: [Style; 5] = [Default, Formal, Casual, VeryCasual, Excited];

    fn p(text: &str, style: Style, chat: bool) -> String {
        punctuate(text, style, chat, &FieldContext::default())
    }

    fn around(before: &str, after: &str) -> FieldContext {
        FieldContext {
            before: before.into(),
            after: after.into(),
            ..FieldContext::default()
        }
    }

    #[test]
    fn counts_sentences() {
        assert_eq!(sentence_count("Are you there? Call me."), 2);
        assert_eq!(sentence_count("See you soon.\nBring the map."), 2);
        assert_eq!(sentence_count("See you soon. Bring the map. Call Mara."), 3);
        assert_eq!(sentence_count("Wait... then go."), 2);
        assert_eq!(sentence_count(&"a line\n".repeat(10)), 1);
    }

    #[test]
    fn casual_drops_up_to_ten_sentences() {
        for chat in [false, true] {
            assert_eq!(
                p(
                    "See you soon. Bring the map. Call Mara first.",
                    Casual,
                    chat
                ),
                "See you soon. Bring the map. Call Mara first"
            );
            let ten = "Go. ".repeat(9) + "Go.";
            assert_eq!(p(&ten, Casual, chat), &ten[..ten.len() - 1]);
            let eleven = "Go. ".repeat(10) + "Go.";
            assert_eq!(p(&eleven, Casual, chat), eleven);
        }
    }

    #[test]
    fn very_casual_always_drops() {
        let eleven = "Go. ".repeat(10) + "Go.";
        assert_eq!(p(&eleven, VeryCasual, false), &eleven[..eleven.len() - 1]);
    }

    #[test]
    fn default_drops_it_in_short_chat_messages() {
        for s in [Default, Excited] {
            assert_eq!(
                p("See you soon. Bring the map.", s, true),
                "See you soon. Bring the map"
            );
            assert_eq!(
                p("See you soon. Bring the map. Call Mara.", s, true),
                "See you soon. Bring the map. Call Mara."
            );
            assert_eq!(
                p("Are you there? Call me.", s, true),
                "Are you there? Call me"
            );
            assert_eq!(p("See you soon.", s, false), "See you soon.");
        }
    }

    #[test]
    fn formal_and_other_marks_stay() {
        for chat in [false, true] {
            assert_eq!(p("See you soon.", Formal, chat), "See you soon.");
            for s in ALL {
                assert_eq!(p("Are you coming?", s, chat), "Are you coming?");
                assert_eq!(p("That is great!", s, chat), "That is great!");
                assert_eq!(p("Well…", s, chat), "Well…");
                assert_eq!(p("Well...", s, chat), "Well...");
            }
        }
    }

    #[test]
    fn a_url_loses_only_the_sentence_period() {
        assert_eq!(p("Visit example.com.", Casual, false), "Visit example.com");
    }

    #[test]
    fn leading_space_is_kept() {
        assert_eq!(p(" see you soon.", Casual, false), " see you soon");
    }

    #[test]
    fn suppressed_by_a_mark_on_the_current_line_or_selection() {
        for ctx in [
            around("Ok. ", ""),
            around("Ok.\u{a0}", ""),
            around("Ok.\t", ""),
            around("", " Ok."),
            around("first\nOk. ", ""),
            FieldContext {
                selected: "Old.".into(),
                ..FieldContext::default()
            },
        ] {
            assert_eq!(
                punctuate("See you soon.", VeryCasual, true, &ctx),
                "See you soon.",
                "{ctx:?}"
            );
        }
        for ctx in [
            around("wait… ", ""),
            around("好。", ""),
            around("Note: ", ""),
            around("Ok.\r\n", ""),
            around("", "\nOk."),
        ] {
            assert_eq!(
                punctuate("See you soon.", VeryCasual, true, &ctx),
                "See you soon",
                "{ctx:?}"
            );
        }
    }
}
