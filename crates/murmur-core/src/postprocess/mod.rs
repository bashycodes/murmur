//! Post-processing: the last deterministic step of a commit, after AI
//! cleanup and before the paste plan (spec `docs/specs/text-postprocess.md`).
//!
//! [`apply_with`] takes the cleaned text, the raw transcript, the session
//! config and the focused field, and returns the text to insert plus
//! whether the shell presses Enter afterwards. Steps, in order:
//! whole-utterance snippet (Windows), spoken "press enter" (Windows),
//! all-lowercase, dictionary replacements, fitting to the cursor (or the
//! skips for apps that manage their own text), style punctuation,
//! trailing space, email signature (Windows).

pub mod apps;
mod chars;
mod cursor;
mod enter;
mod replace;
mod signature;
pub mod spoken;
mod style;
pub mod terminal;

pub use replace::DictMatcher;

use crate::align::trim_punct;
use crate::config::{AppCategory, FieldContext, Platform, SessionConfig, Style};
use crate::dictionary::DictEntry;
use apps::{AppInfo, AppType};
use cursor::{Field, Proper};

/// The category post-processing and AI cleanup use: the shell's own
/// category when it sent one, else the app groups' (§9).
pub fn effective_category(platform: Platform, ctx: &FieldContext) -> AppCategory {
    if ctx.app_category != AppCategory::Other {
        return ctx.app_category;
    }
    classify(platform, ctx).category()
}

fn classify(platform: Platform, ctx: &FieldContext) -> AppInfo {
    apps::classify(platform, &ctx.app_id, &ctx.url_host)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostOutput {
    pub text: String,
    pub press_enter: bool,
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
    if text.is_empty() {
        return PostOutput::default();
    }
    let run = Run {
        cfg,
        ctx,
        windows: cfg.platform == Platform::Windows,
        style: cfg
            .styles
            .for_category(effective_category(cfg.platform, ctx)),
        listed: classify(cfg.platform, ctx),
    };
    let snippet = if run.windows {
        m.whole_snippet(raw_asr)
    } else {
        None
    };
    let mut out = match snippet {
        Some(body) => PostOutput {
            text: body,
            press_enter: false,
        },
        None => run.body(text, raw_asr, m),
    };
    if run.windows && run.listed.app_type == AppType::Email && ctx.context_known {
        if let Some(sig) = cfg.email_signature.as_deref().filter(|s| !s.is_empty()) {
            out.text = signature::append(out.text, sig, ctx);
        }
    }
    out
}

/// What one dictation's post-processing reads, decided once.
struct Run<'a> {
    cfg: &'a SessionConfig,
    ctx: &'a FieldContext,
    windows: bool,
    style: Style,
    /// The app groups' view of the app, ignoring `ctx.app_category`.
    listed: AppInfo,
}

impl Run<'_> {
    /// Steps 3–8: press Enter, all-lowercase, dictionary, cursor fitting
    /// or a §5.7 skip, style punctuation, trailing space.
    fn body(&self, text: &str, raw_asr: &str, m: &DictMatcher) -> PostOutput {
        let cfg = self.cfg;
        let (mut text, press_enter) = if self.windows && cfg.press_enter_enabled {
            enter::take(text, raw_asr)
        } else {
            (text.to_string(), false)
        };
        if cfg.all_lowercase {
            text = text.to_lowercase();
        }
        text = m.find_and_replace(&text);
        let text = self.fit(text.trim(), press_enter);
        PostOutput { text, press_enter }
    }

    /// Steps 6–8 on the trimmed text `t`.
    fn fit(&self, t: &str, press_enter: bool) -> String {
        let ctx = self.ctx;
        if t.is_empty() {
            return String::new();
        }
        let selected = !ctx.selected.is_empty();
        // This app's own windows: inserted as is, whatever the context;
        // over a selection the text keeps its period too.
        if self.windows && self.listed.is_murmur_app {
            return if selected {
                t.to_string()
            } else {
                self.punctuate(t)
            };
        }
        let names: Vec<String> = self
            .cfg
            .user_names
            .iter()
            .chain(&ctx.screen_words)
            .cloned()
            .collect();
        let proper = Proper {
            names: &names,
            dict: &self.cfg.dictionary.entries,
        };
        if selected {
            return cursor::over_selection(t, &ctx.selected, proper);
        }
        if self.listed.is_notion && notion_skip(ctx) {
            return self.punctuate(t);
        }
        let field = Field {
            before: &ctx.before,
            after: &ctx.after,
        };
        let fitted = cursor::at_cursor(t, field, proper);
        let mut out = self.punctuate(&fitted.text);
        // On Android an unreadable field may already end in a space.
        if fitted.room_after
            && !press_enter
            && (ctx.context_known || self.windows)
            && out.chars().last().is_some_and(|c| !chars::is_cjk(c))
        {
            out.push(' ');
        }
        out
    }

    fn punctuate(&self, t: &str) -> String {
        style::punctuate(t, self.style, self.listed.is_messaging, self.ctx)
    }
}

/// §5.7 in Notion: a short or "..."-ended `before` with nothing after the
/// cursor (the selection is empty; the caller checked).
fn notion_skip(ctx: &FieldContext) -> bool {
    !ctx.before.is_empty()
        && ctx.after.is_empty()
        && (ctx.before.ends_with("...") || ctx.before.split(' ').count() <= 2)
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

    fn notion(before: &str, after: &str) -> FieldContext {
        FieldContext {
            app_id: "chrome.exe".into(),
            url_host: "notion.so".into(),
            ..field(before, after)
        }
    }

    #[test]
    fn empty_field_gets_a_trailing_space() {
        let o = run(PARCEL, &win(), &FieldContext::default());
        assert_eq!(o.text, "The parcel arrived early. ");
        assert!(!o.press_enter);
    }

    #[test]
    fn empty_text_stays_empty_even_for_a_snippet() {
        let cfg = with_dict(win(), vec![DictEntry::snippet("my address", "12 Elm Road")]);
        let o = apply("", "my address", &cfg, &FieldContext::default());
        assert_eq!(o, PostOutput::default());
    }

    #[test]
    fn whitespace_only_text_becomes_empty() {
        // Change: no stray space.
        for ctx in [FieldContext::default(), field("x ", "")] {
            assert_eq!(out("   ", &win(), &ctx), "");
        }
    }

    #[test]
    fn cursor_context_examples() {
        let w = win();
        let early = "The parcel arrived early";
        assert_eq!(out(PARCEL, &w, &field("", "and then")), format!("{early} "));
        assert_eq!(out(PARCEL, &w, &field("", " and then")), early);
        assert_eq!(
            out(PARCEL, &w, &field("", "Then we left.")),
            "The parcel arrived early. "
        );
        assert_eq!(out(PARCEL, &w, &field("", "42")), format!("{PARCEL} "));
        for a in [".", ",", ";", "?", "— and"] {
            assert_eq!(out(PARCEL, &w, &field("", a)), early, "{a:?}");
        }
        for a in [")", "\"", "—", "😀", "…", "$5", "\tand", " "] {
            assert_eq!(out(PARCEL, &w, &field("", a)), PARCEL, "{a:?}");
        }
        assert_eq!(out(PARCEL, &w, &field("", "- And")), format!("{PARCEL} "));
        assert_eq!(out(PARCEL, &w, &field("", "- and")), format!("{early} "));
        assert_eq!(
            out(PARCEL, &w, &field("I think", "")),
            " the parcel arrived early. "
        );
        assert_eq!(
            out(PARCEL, &w, &field("Done.", "")),
            " The parcel arrived early. "
        );
        assert_eq!(out("世界很好。", &w, &field("", "再见")), "世界很好。");
        assert_eq!(out("好的。", &w, &field("", "x")), "好的。");
        assert_eq!(
            out("世界很好。", &w, &FieldContext::default()),
            "世界很好。"
        );
        // Change: no space before a newline.
        assert_eq!(out(PARCEL, &w, &field("", "\nNext line")), PARCEL);
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
        // Change: all-caps words keep their case; mixed case lowers the
        // first letter only.
        assert_eq!(
            out("NASA called.", &win(), &field("and ", "")),
            "NASA called. "
        );
        assert_eq!(
            out("IPhone works.", &win(), &field("and ", "")),
            "iPhone works. "
        );
    }

    #[test]
    fn unknown_context_still_uses_the_field() {
        let unknown = |before: &str, app_id: &str| FieldContext {
            context_known: false,
            before: before.into(),
            app_id: app_id.into(),
            ..Default::default()
        };
        assert_eq!(
            out(PARCEL, &win(), &unknown("", "")),
            "The parcel arrived early. "
        );
        assert_eq!(
            out(PARCEL, &android(), &unknown("", "")),
            "The parcel arrived early."
        );
        assert_eq!(
            out(PARCEL, &android(), &unknown("ok ", "")),
            "the parcel arrived early."
        );
        assert_eq!(
            out(
                "See you soon.",
                &android(),
                &unknown("Ok. ", "com.whatsapp")
            ),
            "See you soon."
        );
        assert_eq!(
            out("See you soon.", &android(), &unknown("", "com.whatsapp")),
            "See you soon"
        );
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
        let ctx = FieldContext {
            selected: " bad words ".into(),
            ..Default::default()
        };
        assert_eq!(out("New words.", &cfg, &ctx), " new words ");
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
        let eleven = "Go. ".repeat(10) + "Go.";
        assert_eq!(
            out(&eleven, &cfg, &app("notepad.exe")),
            format!("{eleven} ")
        );
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
        let mut formal = win();
        formal.styles.personal = Style::Formal;
        assert_eq!(
            out("See you soon. Bring the map.", &formal, &wa),
            "See you soon. Bring the map. "
        );
    }

    #[test]
    fn the_listed_category_decides_chat() {
        let bucket = |id: &str, c: AppCategory| FieldContext {
            app_id: id.into(),
            app_category: c,
            ..Default::default()
        };
        assert_eq!(
            out(
                "See you soon.",
                &win(),
                &bucket("notepad.exe", AppCategory::PersonalChat)
            ),
            "See you soon. "
        );
        assert_eq!(
            out(
                "See you soon.",
                &win(),
                &bucket("WhatsApp.exe", AppCategory::Email)
            ),
            "See you soon "
        );
        // Fix: Windows ids match without `.exe`.
        assert_eq!(
            out("See you soon.", &win(), &app("Slack.exe")),
            "See you soon "
        );
    }

    #[test]
    fn notion_skip() {
        let w = win();
        for b in ["so ", "so then", "Done.", "we went to the shop..."] {
            assert_eq!(out(PARCEL, &w, &notion(b, "")), PARCEL, "{b:?}");
            let page = FieldContext {
                url_host: "notion.so/page".into(),
                ..notion(b, "")
            };
            assert_eq!(out(PARCEL, &w, &page), PARCEL, "{b:?}");
            let pkg = FieldContext {
                app_id: "notion.id".into(),
                ..field(b, "")
            };
            assert_eq!(out(PARCEL, &android(), &pkg), PARCEL, "{b:?}");
        }
        assert_eq!(
            out(PARCEL, &w, &notion("so then ", "")),
            "the parcel arrived early. "
        );
        assert_eq!(
            out(PARCEL, &w, &notion("we went to the shop... ", "")),
            "The parcel arrived early. "
        );
        assert_eq!(
            out(PARCEL, &w, &notion("so ", "x")),
            "the parcel arrived early "
        );
        assert_eq!(
            out(PARCEL, &w, &notion("", "")),
            "The parcel arrived early. "
        );
        for host in ["www.notion.so", "Notion.so", "team.notion.site"] {
            let other = FieldContext {
                url_host: host.into(),
                ..notion("so ", "")
            };
            assert_eq!(
                out(PARCEL, &w, &other),
                "the parcel arrived early. ",
                "{host}"
            );
        }
        // The text is trimmed; style punctuation still applies.
        assert_eq!(
            out("  The parcel arrived early.  ", &w, &notion("so ", "")),
            PARCEL
        );
        let mut casual = win();
        casual.styles.other = Style::Casual;
        assert_eq!(
            out(PARCEL, &casual, &notion("so ", "")),
            "The parcel arrived early"
        );
        // The Windows desktop app gets normal fitting.
        let desktop = FieldContext {
            app_id: "Notion.exe".into(),
            ..field("so ", "")
        };
        assert_eq!(out(PARCEL, &w, &desktop), "the parcel arrived early. ");
    }

    #[test]
    fn own_app_windows_insert_the_text_as_is() {
        let contexts = |id: &str| {
            [
                FieldContext {
                    app_id: id.into(),
                    ..field("and", "then")
                },
                FieldContext {
                    app_id: id.into(),
                    ..field("so ", "")
                },
                FieldContext {
                    app_id: id.into(),
                    selected: "bad".into(),
                    ..Default::default()
                },
                app(id),
            ]
        };
        for id in ["murmur.exe", "murmur-windows.exe"] {
            for ctx in contexts(id) {
                assert_eq!(out(PARCEL, &win(), &ctx), PARCEL, "{ctx:?}");
            }
            let mut casual = win();
            casual.styles.other = Style::Casual;
            assert_eq!(out(PARCEL, &casual, &app(id)), "The parcel arrived early");
            // Over a selection the text keeps its period.
            let over = FieldContext {
                app_id: id.into(),
                selected: "abc".into(),
                ..Default::default()
            };
            assert_eq!(out(PARCEL, &casual, &over), PARCEL);
            // A sentence mark on the cursor line keeps it too.
            let marked = FieldContext {
                app_id: id.into(),
                ..field("Ok. ", "")
            };
            assert_eq!(out(PARCEL, &casual, &marked), PARCEL);
            let o = run("Send it press enter.", &win(), &app(id));
            assert_eq!((o.text.as_str(), o.press_enter), ("Send it.", true));
        }
        // Android's own package is not special.
        let ctx = FieldContext {
            app_id: "dev.murmur".into(),
            ..field("and ", "")
        };
        assert_eq!(out(PARCEL, &android(), &ctx), "the parcel arrived early. ");
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
        let outlook = app("outlook.exe");
        assert_eq!(
            out("Hi Tom, thanks.", &cfg, &outlook),
            "Hi Tom, thanks.\n\n— Ann"
        );
        assert_eq!(
            out("Thanks for the update.", &cfg, &outlook),
            "Thanks for the update. "
        );
        let ten = "We can meet on the third floor after lunch today.";
        assert_eq!(out(ten, &cfg, &outlook), format!("{ten}\n\n— Ann"));
        let nine = "We can meet on the third floor after lunch.";
        assert_eq!(out(nine, &cfg, &outlook), format!("{nine} "));
        assert_eq!(out("", &cfg, &outlook), "");
        // The shell's bucket does not count; the listed category does.
        let bucket = |id: &str, c: AppCategory| FieldContext {
            app_id: id.into(),
            app_category: c,
            ..Default::default()
        };
        assert_eq!(
            out(
                "Hi Tom, thanks.",
                &cfg,
                &bucket("notepad.exe", AppCategory::Email)
            ),
            "Hi Tom, thanks. "
        );
        assert_eq!(
            out(
                "Hi Tom, thanks.",
                &cfg,
                &bucket("outlook.exe", AppCategory::WorkChat)
            ),
            "Hi Tom, thanks.\n\n— Ann"
        );
        // Fix: the Windows id matches in any case, without `.exe`.
        assert_eq!(
            out("Hi Tom, thanks.", &cfg, &app("Outlook.EXE")),
            "Hi Tom, thanks.\n\n— Ann"
        );
        // Not on Android, not with an unknown field, not with an empty
        // signature.
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
    fn snippet_body_gets_the_signature() {
        let mut cfg = with_dict(
            win(),
            vec![
                DictEntry::snippet("reply ok", "Hi Tom, all good here."),
                DictEntry::snippet("my address", "12 Elm Road"),
            ],
        );
        cfg.email_signature = Some("— Ann".into());
        let outlook = app("outlook.exe");
        assert_eq!(
            apply("Reply ok.", "reply ok", &cfg, &outlook).text,
            "Hi Tom, all good here.\n\n— Ann"
        );
        assert_eq!(
            apply("My address.", "my address", &cfg, &outlook).text,
            "12 Elm Road"
        );
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
