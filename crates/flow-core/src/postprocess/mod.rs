//! Post-processing: the last deterministic step of a commit, after AI
//! cleanup and before the paste plan (spec `docs/specs/text-postprocess.md`).
//!
//! [`apply_with`] takes the cleaned text, the raw transcript, the session
//! config and the focused field, and returns the text to insert plus
//! whether the shell presses Enter afterwards. Steps, in order:
//! whole-utterance snippet (Windows), spoken "press enter" (Windows),
//! all-lowercase, dictionary replacements, fitting to the cursor, style
//! punctuation, trailing space, email signature (Windows).

mod apps;
mod chars;
mod cursor;
mod enter;
mod replace;
mod signature;
pub mod spoken;
mod style;
pub mod terminal;

pub use apps::effective_category;
pub use replace::DictMatcher;

use crate::config::{AppCategory, FieldContext, Platform, SessionConfig};
use crate::dictionary::DictEntry;
use cursor::{Field, Fitted, Proper};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostOutput {
    pub text: String,
    pub press_enter: bool,
    /// Kept for source compatibility; always false.
    pub showed_prompt: bool,
}

/// [`apply_with`] with a matcher compiled from `cfg.dictionary`.
pub fn apply(text: &str, raw_asr: &str, cfg: &SessionConfig, ctx: &FieldContext) -> PostOutput {
    let m = DictMatcher::new(&cfg.dictionary.entries, &[]);
    apply_with(text, raw_asr, cfg, ctx, &m)
}

/// Post-process the cleaned `text` of a dictation into `ctx`'s field.
pub fn apply_with(
    text: &str,
    raw_asr: &str,
    cfg: &SessionConfig,
    ctx: &FieldContext,
    m: &DictMatcher,
) -> PostOutput {
    let windows = cfg.platform == Platform::Windows;
    let category = effective_category(cfg.platform, ctx);
    let style = cfg.styles.for_category(category);

    if windows {
        if let Some(body) = m.whole_snippet(raw_asr) {
            return PostOutput {
                text: body,
                ..Default::default()
            };
        }
    }

    let (mut text, press_enter) = if windows && cfg.press_enter_enabled {
        enter::take(text, raw_asr)
    } else {
        (text.to_string(), false)
    };
    if cfg.all_lowercase {
        text = text.to_lowercase();
    }
    text = m.find_and_replace(&text);

    let mut room_after = false;
    if apps::manages_own_spacing(cfg.platform, ctx) {
        text = style::punctuate(text.trim(), style, category);
    } else {
        let names: Vec<String> = cfg
            .user_names
            .iter()
            .chain(&ctx.screen_words)
            .cloned()
            .collect();
        let proper = Proper {
            names: &names,
            dict: &cfg.dictionary.entries,
        };
        // An unreadable field is treated as an empty one.
        let field = if ctx.context_known {
            Field {
                before: &ctx.before,
                selected: &ctx.selected,
                after: &ctx.after,
            }
        } else {
            Field::default()
        };
        match cursor::fit(&text, field, proper) {
            Fitted::Selection(s) => text = s,
            Fitted::Cursor {
                text: t,
                room_after: r,
            } => {
                text = style::punctuate(&t, style, category);
                // On Android an unreadable field may already end in a space.
                room_after = r && (ctx.context_known || windows);
            }
        }
    }

    if room_after && !press_enter && text.chars().last().is_some_and(|c| !chars::is_cjk(c)) {
        text.push(' ');
    }

    if windows && category == AppCategory::Email && ctx.context_known {
        if let Some(sig) = cfg
            .email_signature
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            text = signature::append(text, sig, ctx);
        }
    }

    PostOutput {
        text,
        press_enter,
        showed_prompt: false,
    }
}

/// True when the first word of `text` (surrounding punctuation removed) is
/// "I" or an "I'" contraction, an entry (or the first word of an entry) of
/// `proper`, or the first word of a dictionary entry's word or replacement.
pub fn starts_with_proper_noun(text: &str, proper: &[String], dict: &[DictEntry]) -> bool {
    let Some(word) = text.split_whitespace().next().map(trim_punct) else {
        return false;
    };
    if word.is_empty() {
        return false;
    }
    if word == "I" || word.starts_with("I'") || word.starts_with("I’") {
        return true;
    }
    let first_is = |s: &str| s.split_whitespace().next().map(trim_punct) == Some(word);
    proper.iter().any(|p| p.trim() == word || first_is(p))
        || dict
            .iter()
            .any(|e| first_is(&e.word) || e.replacement.as_deref().is_some_and(first_is))
}

fn trim_punct(w: &str) -> &str {
    w.trim_matches(|c: char| !c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Style;
    use crate::dictionary::Dictionary;

    const PARCEL: &str = "The parcel arrived early.";

    fn win() -> SessionConfig {
        SessionConfig::default()
    }

    fn android() -> SessionConfig {
        SessionConfig {
            platform: Platform::Android,
            ..Default::default()
        }
    }

    fn with_dict(mut cfg: SessionConfig, entries: Vec<DictEntry>) -> SessionConfig {
        cfg.dictionary = Dictionary { entries };
        cfg
    }

    fn field(before: &str, after: &str) -> FieldContext {
        FieldContext {
            before: before.into(),
            after: after.into(),
            ..Default::default()
        }
    }

    fn run(text: &str, cfg: &SessionConfig, ctx: &FieldContext) -> PostOutput {
        apply(text, text, cfg, ctx)
    }

    fn out(text: &str, cfg: &SessionConfig, ctx: &FieldContext) -> String {
        run(text, cfg, ctx).text
    }

    fn app(id: &str) -> FieldContext {
        FieldContext {
            app_id: id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_field_gets_a_trailing_space() {
        let o = run(PARCEL, &win(), &FieldContext::default());
        assert_eq!(o.text, "The parcel arrived early. ");
        assert!(!o.press_enter);
        assert!(!o.showed_prompt);
    }

    #[test]
    fn cursor_context_examples() {
        let w = win();
        assert_eq!(
            out(PARCEL, &w, &field("", "and then")),
            "The parcel arrived early "
        );
        assert_eq!(
            out(PARCEL, &w, &field("", " and then")),
            "The parcel arrived early"
        );
        assert_eq!(
            out(PARCEL, &w, &field("", "Then we left.")),
            "The parcel arrived early. "
        );
        assert_eq!(
            out(PARCEL, &w, &field("", "42")),
            "The parcel arrived early. "
        );
        assert_eq!(out(PARCEL, &w, &field("", ".")), "The parcel arrived early");
        assert_eq!(
            out(PARCEL, &w, &field("", ")")),
            "The parcel arrived early."
        );
        assert_eq!(
            out(PARCEL, &w, &field("", " ")),
            "The parcel arrived early."
        );
        assert_eq!(
            out(PARCEL, &w, &field("I think", "")),
            " the parcel arrived early. "
        );
        assert_eq!(
            out(PARCEL, &w, &field("Done.", "")),
            " The parcel arrived early. "
        );
        assert_eq!(
            out("世界很好。", &w, &FieldContext::default()),
            "世界很好。"
        );
        // Change: no space before a newline.
        assert_eq!(
            out(PARCEL, &w, &field("", "\nNext line")),
            "The parcel arrived early."
        );
    }

    #[test]
    fn mid_sentence_case_uses_names_and_screen_words() {
        let mut cfg = win();
        cfg.user_names = vec!["Mara".into()];
        let ctx = FieldContext {
            before: "and ".into(),
            screen_words: vec!["Quillon".into()],
            ..Default::default()
        };
        assert_eq!(out("Mara called.", &cfg, &ctx), "Mara called. ");
        assert_eq!(
            out("Mara called.", &win(), &field("and ", "")),
            "mara called. "
        );
        assert_eq!(out("Quillon is down.", &cfg, &ctx), "Quillon is down. ");
        let cfg = with_dict(win(), vec![DictEntry::word("Kubernetes")]);
        assert_eq!(
            out("Kubernetes failed.", &cfg, &field("and ", "")),
            "Kubernetes failed. "
        );
        // Change: mixed-case and all-caps words keep their case.
        assert_eq!(
            out("NASA called.", &win(), &field("and ", "")),
            "NASA called. "
        );
        assert_eq!(
            out("IPhone works.", &win(), &field("and ", "")),
            "IPhone works. "
        );
    }

    #[test]
    fn unknown_context() {
        let ctx = FieldContext {
            context_known: false,
            before: "stale".into(),
            ..Default::default()
        };
        assert_eq!(out(PARCEL, &win(), &ctx), "The parcel arrived early. ");
        assert_eq!(out(PARCEL, &android(), &ctx), "The parcel arrived early.");
    }

    #[test]
    fn selection_takes_its_shape_and_skips_style() {
        let mut cfg = win();
        cfg.styles.other = Style::Casual;
        let ctx = FieldContext {
            before: "x".into(),
            selected: "Bad words.".into(),
            after: "y".into(),
            ..Default::default()
        };
        assert_eq!(out("New words.", &cfg, &ctx), "New words.");
        let ctx = FieldContext {
            selected: " Bad words. ".into(),
            ..Default::default()
        };
        assert_eq!(out("New words.", &cfg, &ctx), " New words. ");
    }

    #[test]
    fn all_lowercase() {
        let mut cfg = with_dict(win(), vec![DictEntry::replacement("lumo", "Lumio")]);
        cfg.all_lowercase = true;
        assert_eq!(
            out("I told Mara about lumo.", &cfg, &FieldContext::default()),
            "i told mara about Lumio. "
        );
        let ctx = FieldContext {
            selected: "Old".into(),
            ..Default::default()
        };
        assert_eq!(out("New words.", &cfg, &ctx), "New words");
    }

    #[test]
    fn dictionary_runs_before_fitting() {
        let cfg = with_dict(
            win(),
            vec![DictEntry::replacement("cubernetes", "Kubernetes")],
        );
        assert_eq!(
            out("deploy the cubernetes job.", &cfg, &FieldContext::default()),
            "deploy the Kubernetes job. "
        );
        let cfg = with_dict(win(), vec![DictEntry::word("Kubernetes")]);
        assert_eq!(
            out("deploy the kubernetes job.", &cfg, &FieldContext::default()),
            "deploy the kubernetes job. "
        );
    }

    #[test]
    fn whole_snippet_on_windows_only() {
        let snip = vec![DictEntry::snippet("my address", "12 Elm Road")];
        let cfg = with_dict(win(), snip.clone());
        for ctx in [FieldContext::default(), field("and", "x")] {
            let o = apply("My address.", "my address", &cfg, &ctx);
            assert_eq!(o.text, "12 Elm Road");
            assert!(!o.press_enter);
        }
        let cfg = with_dict(win(), vec![DictEntry::snippet("sign off", "Best,\nMara")]);
        assert_eq!(
            apply("Sign off.", "sign off", &cfg, &FieldContext::default()).text,
            "Best,\nMara"
        );
        let cfg = with_dict(android(), snip);
        assert_eq!(
            apply("My address.", "my address", &cfg, &FieldContext::default()).text,
            "12 Elm Road "
        );
    }

    #[test]
    fn style_in_context() {
        let mut cfg = win();
        cfg.styles.personal = Style::Casual;
        cfg.styles.other = Style::Casual;
        for id in ["WhatsApp.exe", "notepad.exe"] {
            assert_eq!(
                out(
                    "See you soon. Bring the map. Call Mara first.",
                    &cfg,
                    &app(id)
                ),
                "See you soon. Bring the map. Call Mara first "
            );
        }
        let wa = app("WhatsApp.exe");
        assert_eq!(
            out("See you soon. Bring the map.", &win(), &wa),
            "See you soon. Bring the map "
        );
        assert_eq!(
            out("See you soon. Bring the map. Call Mara.", &win(), &wa),
            "See you soon. Bring the map. Call Mara. "
        );
        assert_eq!(
            out("Are you there? Call me.", &win(), &wa),
            "Are you there? Call me "
        );
        assert_eq!(
            out("See you soon.", &win(), &app("notepad.exe")),
            "See you soon. "
        );
        assert_eq!(out("Wait...", &cfg, &wa), "Wait... ");
    }

    #[test]
    fn the_shell_category_decides_chat() {
        // Change: an unlisted app in a personal-chat bucket loses the period.
        let ctx = FieldContext {
            app_id: "zentrix.exe".into(),
            app_category: AppCategory::PersonalChat,
            ..Default::default()
        };
        assert_eq!(out("See you soon.", &win(), &ctx), "See you soon ");
    }

    #[test]
    fn own_spacing_apps_insert_the_trimmed_text() {
        let mut cfg = win();
        cfg.styles.other = Style::Casual;
        let ctx = FieldContext {
            app_id: "Notion.exe".into(),
            before: "and".into(),
            after: "then".into(),
            ..Default::default()
        };
        assert_eq!(out("  The parcel arrived early.  ", &win(), &ctx), PARCEL);
        assert_eq!(out(PARCEL, &cfg, &ctx), "The parcel arrived early");
        let ctx = FieldContext {
            app_id: "chrome.exe".into(),
            url_host: "www.notion.so".into(),
            before: "and".into(),
            ..Default::default()
        };
        assert_eq!(out(PARCEL, &win(), &ctx), PARCEL);
        let ctx = FieldContext {
            app_id: "dev.flowclone.hybrid".into(),
            before: "and".into(),
            ..Default::default()
        };
        assert_eq!(out(PARCEL, &android(), &ctx), PARCEL);
    }

    #[test]
    fn press_enter() {
        let o = run("Send it press enter.", &win(), &FieldContext::default());
        assert_eq!((o.text.as_str(), o.press_enter), ("Send it.", true));
        let o = run("press enter", &win(), &FieldContext::default());
        assert_eq!((o.text.as_str(), o.press_enter), ("", true));
        let o = run("Send it press enter.", &win(), &field("", " more"));
        assert_eq!((o.text.as_str(), o.press_enter), ("Send it", true));
        let mut casual = win();
        casual.styles.other = Style::Casual;
        let o = run("Send it press enter.", &casual, &FieldContext::default());
        assert_eq!((o.text.as_str(), o.press_enter), ("Send it", true));
        let o = run("Send it.\nPress enter.", &win(), &FieldContext::default());
        assert_eq!((o.text.as_str(), o.press_enter), ("Send it.", true));
        // Change: the raw-only case adds no trailing space.
        let o = apply(
            "Send it.",
            "send it press enter",
            &win(),
            &FieldContext::default(),
        );
        assert_eq!((o.text.as_str(), o.press_enter), ("Send it.", true));
        let o = run("Press enter now.", &win(), &FieldContext::default());
        assert_eq!(
            (o.text.as_str(), o.press_enter),
            ("Press enter now. ", false)
        );
    }

    #[test]
    fn press_enter_off_or_android() {
        let mut off = win();
        off.press_enter_enabled = false;
        for cfg in [off, android()] {
            let o = run("Send it press enter.", &cfg, &FieldContext::default());
            assert_eq!(
                (o.text.as_str(), o.press_enter),
                ("Send it press enter. ", false)
            );
            let o = apply(
                "Send it.",
                "send it press enter",
                &cfg,
                &FieldContext::default(),
            );
            assert!(!o.press_enter);
        }
    }

    #[test]
    fn email_signature() {
        let mut cfg = win();
        cfg.email_signature = Some("— Ann".into());
        let outlook = app("olk.exe");
        assert_eq!(
            out("Hi Tom, thanks.", &cfg, &outlook),
            "Hi Tom, thanks.\n\n— Ann"
        );
        assert_eq!(out("Thanks for that.", &cfg, &outlook), "Thanks for that. ");
        let ten = "We can meet on the third floor after lunch today.";
        assert_eq!(out(ten, &cfg, &outlook), format!("{ten}\n\n— Ann"));
        let nine = "We can meet on the third floor after lunch.";
        assert_eq!(out(nine, &cfg, &outlook), format!("{nine} "));
        // A shell-set Email category counts.
        let ctx = FieldContext {
            app_id: "zentrix.exe".into(),
            app_category: AppCategory::Email,
            ..Default::default()
        };
        assert_eq!(
            out("Hi Tom, thanks.", &cfg, &ctx),
            "Hi Tom, thanks.\n\n— Ann"
        );
        // Not outside email, not on Android, not with an unknown field.
        assert_eq!(
            out("Hi Tom, thanks.", &cfg, &app("winword.exe")),
            "Hi Tom, thanks. "
        );
        let mut a = cfg.clone();
        a.platform = Platform::Android;
        let gmail = app("com.google.android.gm");
        assert_eq!(out("Hi Tom, thanks.", &a, &gmail), "Hi Tom, thanks. ");
        let unknown = FieldContext {
            context_known: false,
            ..outlook.clone()
        };
        assert_eq!(out("Hi Tom, thanks.", &cfg, &unknown), "Hi Tom, thanks. ");
        cfg.email_signature = Some(String::new());
        assert_eq!(out("Hi Tom, thanks.", &cfg, &outlook), "Hi Tom, thanks. ");
    }

    #[test]
    fn proper_noun_rules() {
        let names = vec!["Mara Quill".to_string(), "Lumio".to_string()];
        let dict = vec![
            DictEntry::word("Zentrix Hub"),
            DictEntry::replacement("cubernetes", "Kubernetes Engine"),
        ];
        let p = |t: &str| starts_with_proper_noun(t, &names, &dict);
        assert!(p("I left."));
        assert!(p("I'm here."));
        assert!(p("I’ll go."));
        assert!(p("\"Mara, hi.\""));
        assert!(p("Lumio rocks."));
        assert!(p("Zentrix is up."));
        assert!(p("Kubernetes failed."));
        assert!(p("cubernetes failed."));
        assert!(!p("Quill came."));
        assert!(!p("mara came."));
        assert!(!p("Its fine."));
        assert!(!p(""));
        assert!(!p("... "));
    }
}
