//! Structured paste plan (CR-2): the commit's text split at the in-band
//! `{{BREAK}}` / `{{TAB}}` markers, plus a trailing Enter for spoken
//! "press enter". Post-processing no longer produces the markers (D82);
//! they act only when the text carries them, e.g. a snippet body.
//!
//! `final_text` keeps the markers for shells built before this existed.
//! A shell that pastes `segments` must not also act on `press_enter`.

use serde::{Deserialize, Serialize};

pub const BREAK_MARKER: &str = "{{BREAK}}";
pub const TAB_MARKER: &str = "{{TAB}}";

/// A key the shell sends between text segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PasteKey {
    /// Press Enter (spoken "press enter"); always last.
    Enter,
    /// Press Tab (accept the IDE @-mention popup).
    Tab,
    /// Paste boundary: finish inserting, pause briefly so the app reacts
    /// (opens the @-mention popup), then continue. No key is sent.
    Break,
}

/// JSON: `{"text": "…"}` or `{"key": "Enter" | "Tab" | "Break"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PasteSegment {
    Text { text: String },
    Key { key: PasteKey },
}

/// Split `text` at the markers; empty text runs are dropped.
pub fn segments(text: &str, press_enter: bool) -> Vec<PasteSegment> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        let b = rest
            .find(BREAK_MARKER)
            .map(|i| (i, PasteKey::Break, BREAK_MARKER));
        let t = rest
            .find(TAB_MARKER)
            .map(|i| (i, PasteKey::Tab, TAB_MARKER));
        let next = match (b, t) {
            (Some(b), Some(t)) => Some(if t.0 < b.0 { t } else { b }),
            (x, None) | (None, x) => x,
        };
        let Some((i, k, m)) = next else { break };
        if i > 0 {
            out.push(PasteSegment::Text {
                text: rest[..i].to_string(),
            });
        }
        out.push(PasteSegment::Key { key: k });
        rest = &rest[i + m.len()..];
    }
    if !rest.is_empty() {
        out.push(PasteSegment::Text {
            text: rest.to_string(),
        });
    }
    if press_enter {
        out.push(PasteSegment::Key {
            key: PasteKey::Enter,
        });
    }
    out
}

/// `text` without paste markers: what was actually inserted (history,
/// "paste last").
pub fn plain_text(text: &str) -> String {
    text.replace(BREAK_MARKER, "").replace(TAB_MARKER, "")
}
