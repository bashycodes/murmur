//! Style punctuation (spec §6).

use crate::config::{AppCategory, Style};

/// `text` with its final period dropped when the style asks for it.
pub(super) fn punctuate(text: &str, style: Style, category: AppCategory) -> String {
    let drop = match style {
        Style::Formal => false,
        Style::Casual | Style::VeryCasual => true,
        Style::Default | Style::Excited => category.is_chat() && sentence_count(text) <= 2,
    };
    match text.strip_suffix('.') {
        Some(rest) if drop && !rest.ends_with('.') => rest.to_string(),
        _ => text.to_string(),
    }
}

/// Non-empty sentences, split at `.`, `!`, `?` and newlines.
fn sentence_count(text: &str) -> usize {
    text.split(['.', '!', '?', '\n'])
        .filter(|s| !s.trim().is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use AppCategory::*;
    use Style::*;

    const ALL: [AppCategory; 7] = [
        Email,
        WorkChat,
        PersonalChat,
        AiPrompt,
        Terminal,
        Document,
        Other,
    ];

    #[test]
    fn counts_sentences() {
        assert_eq!(sentence_count("Are you there? Call me."), 2);
        assert_eq!(sentence_count("See you soon.\nBring the map."), 2);
        assert_eq!(sentence_count("See you soon. Bring the map. Call Mara."), 3);
        assert_eq!(sentence_count("Wait..."), 1);
    }

    #[test]
    fn casual_drops_the_final_period_everywhere() {
        for c in ALL {
            for s in [Casual, VeryCasual] {
                assert_eq!(
                    punctuate("See you soon. Bring the map. Call Mara first.", s, c),
                    "See you soon. Bring the map. Call Mara first"
                );
            }
        }
    }

    #[test]
    fn default_drops_it_in_short_chat_messages() {
        for s in [Default, Excited] {
            assert_eq!(
                punctuate("See you soon. Bring the map.", s, PersonalChat),
                "See you soon. Bring the map"
            );
            assert_eq!(
                punctuate("See you soon. Bring the map. Call Mara.", s, PersonalChat),
                "See you soon. Bring the map. Call Mara."
            );
            assert_eq!(
                punctuate("Are you there? Call me.", s, WorkChat),
                "Are you there? Call me"
            );
            assert_eq!(punctuate("See you soon.", s, Email), "See you soon.");
            assert_eq!(punctuate("See you soon.", s, Other), "See you soon.");
        }
    }

    #[test]
    fn formal_and_other_marks_stay() {
        for c in ALL {
            assert_eq!(punctuate("See you soon.", Formal, c), "See you soon.");
            for s in [Default, Formal, Casual, VeryCasual, Excited] {
                assert_eq!(punctuate("Are you coming?", s, c), "Are you coming?");
                assert_eq!(punctuate("That is great!", s, c), "That is great!");
                assert_eq!(punctuate("Well…", s, c), "Well…");
                assert_eq!(punctuate("Well...", s, c), "Well...");
            }
        }
    }

    #[test]
    fn a_url_loses_only_the_sentence_period() {
        assert_eq!(
            punctuate("Visit example.com.", Casual, Other),
            "Visit example.com"
        );
    }

    #[test]
    fn leading_space_is_kept() {
        assert_eq!(punctuate(" see you soon.", Casual, Other), " see you soon");
    }
}
