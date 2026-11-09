//! Email signature (spec §7).

use crate::config::FieldContext;

const GREETINGS: [&str; 4] = ["hi", "hello", "hey", "dear"];

/// `text` with `"\n\n" + sig` appended when it reads like an email body
/// and the field does not already hold the signature.
pub(super) fn append(text: String, sig: &str, ctx: &FieldContext) -> String {
    if !reads_like_email(&text) || already_signed(sig, ctx, &text) {
        return text;
    }
    let mut out = text.strip_suffix(' ').unwrap_or(&text).to_string();
    out.push_str("\n\n");
    out.push_str(sig);
    out
}

fn reads_like_email(text: &str) -> bool {
    let lower = text.trim_start().to_lowercase();
    let greeting = GREETINGS.iter().any(|g| {
        lower
            .strip_prefix(g)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| matches!(c, ' ' | ',' | '!'))
    });
    greeting
        || text
            .split_whitespace()
            .filter(|w| w.chars().any(char::is_alphanumeric))
            .count()
            >= 10
}

fn already_signed(sig: &str, ctx: &FieldContext, text: &str) -> bool {
    let needle = normalise(sig.trim_start_matches(['-', '—', ' ']));
    [
        ctx.before.as_str(),
        ctx.selected.as_str(),
        ctx.after.as_str(),
        text,
    ]
    .iter()
    .any(|h| normalise(h).contains(&needle))
}

fn normalise(s: &str) -> String {
    s.to_lowercase()
        .replace("&nbsp;", " ")
        .replace('\u{a0}', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIG: &str = "— Ann";

    fn sign(text: &str) -> String {
        append(text.into(), SIG, &FieldContext::default())
    }

    #[test]
    fn greeting_or_ten_words() {
        assert_eq!(sign("Hi Tom, thanks. "), "Hi Tom, thanks.\n\n— Ann");
        assert_eq!(sign("hello, all set."), "hello, all set.\n\n— Ann");
        assert_eq!(sign("Hey! Done."), "Hey! Done.\n\n— Ann");
        assert_eq!(sign("Thanks for the update. "), "Thanks for the update. ");
        // Only one trailing space is removed.
        assert_eq!(sign("Hi Tom.  "), "Hi Tom. \n\n— Ann");
        assert_eq!(sign(""), "");
        assert_eq!(sign("Hiya Tom."), "Hiya Tom.");
        let ten = "one two three four five six seven eight nine ten.";
        assert_eq!(sign(ten), format!("{ten}\n\n— Ann"));
        let nine = "one two three four five six seven eight nine.";
        assert_eq!(sign(nine), nine);
        // Words without a letter or digit do not count.
        assert_eq!(
            sign("one two three four five six seven eight nine -"),
            "one two three four five six seven eight nine -"
        );
    }

    #[test]
    fn not_twice() {
        let ctx = FieldContext {
            after: "\n\n-- ann".into(),
            ..Default::default()
        };
        assert_eq!(
            append("Hi Tom, thanks.".into(), SIG, &ctx),
            "Hi Tom, thanks."
        );
        let ctx = FieldContext {
            before: "Sent&nbsp;by voice".into(),
            ..Default::default()
        };
        assert_eq!(append("Hi Tom.".into(), "— Sent by voice", &ctx), "Hi Tom.");
        assert_eq!(sign("Hi Tom, from Ann."), "Hi Tom, from Ann.");
    }
}
