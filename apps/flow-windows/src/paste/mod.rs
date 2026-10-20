//! Text insertion (spec §4.6): clipboard + synthetic Ctrl+V with full
//! clipboard save/restore, KEYEVENTF_UNICODE typing as fallback.
//!
//! Decisions are pure (strategy choice, restore guard, which formats can be
//! snapshotted) and tested here; `win` does the Win32 calls.

#[cfg(windows)]
pub mod integrity;
#[cfg(windows)]
pub mod win;

use crate::prefs::PasteMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    Clipboard { shift_insert: bool },
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteOptions {
    pub method: PasteMethod,
    pub restore_clipboard: bool,
    /// `None` = per-app default ([`restore_delay_ms`]).
    pub restore_delay_ms: Option<u64>,
    pub type_fallback_max_chars: usize,
}

impl PasteOptions {
    pub fn from_prefs(p: &crate::prefs::Prefs) -> Self {
        PasteOptions {
            method: p.paste_method,
            restore_clipboard: p.restore_clipboard,
            restore_delay_ms: p.paste_restore_delay_ms,
            type_fallback_max_chars: p.type_fallback_max_chars,
        }
    }
}

/// Wait between the paste chord and restoring the user's clipboard, keyed
/// on the foreground exe (Wispr helper 0x1403188a0, 02 §REA note 12): slow
/// readers (Office, WhatsApp) and remote viewers fetch the clipboard late.
pub fn restore_delay_ms(exe: &str) -> u64 {
    const REMOTE_VIEWERS: &[&str] = &[
        "vncviewer",
        "tvnviewer",
        "mstsc",
        "msrdc",
        "wfica32",
        "vmware-view",
        "rdcman",
        "anydesk",
        "teamviewer",
    ];
    const SLOW_READERS: &[&str] = &["outlook", "olk", "winword", "whatsapp"];
    let lower = exe.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    if REMOTE_VIEWERS.contains(&stem) {
        5000
    } else if SLOW_READERS.contains(&stem) {
        3000
    } else {
        500
    }
}

/// Preferred strategy before touching the clipboard.
pub fn initial_strategy(method: PasteMethod) -> Strategy {
    match method {
        PasteMethod::CtrlV => Strategy::Clipboard {
            shift_insert: false,
        },
        PasteMethod::ShiftInsert => Strategy::Clipboard { shift_insert: true },
        PasteMethod::Type => Strategy::Type,
    }
}

/// Clipboard couldn't be opened (another app holds it): type short texts,
/// give up on long ones (typing 5 kB char-by-char is slow and fragile).
pub fn fallback_after_clipboard_failure(text: &str, max_chars: usize) -> Option<Strategy> {
    (text.chars().count() <= max_chars).then_some(Strategy::Type)
}

/// Restore the user's clipboard only if nobody wrote to it since we did
/// (`GetClipboardSequenceNumber` unchanged) — otherwise we'd clobber a copy
/// the user or target app made in the meantime.
pub fn should_restore(enabled: bool, seq_after_our_set: u32, seq_now: u32) -> bool {
    enabled && seq_after_our_set == seq_now
}

/// Formats whose clipboard handle is an HGLOBAL we can copy byte-wise.
/// GDI-handle formats (bitmaps, metafiles, palettes) and owner-display
/// formats are skipped; Windows re-synthesises CF_BITMAP from CF_DIB.
pub fn is_restorable_format(fmt: u32) -> bool {
    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    const CF_OWNERDISPLAY: u32 = 0x80;
    const CF_DSPBITMAP: u32 = 0x82;
    const CF_DSPMETAFILEPICT: u32 = 0x83;
    const CF_DSPENHMETAFILE: u32 = 0x8E;
    !matches!(
        fmt,
        CF_BITMAP
            | CF_METAFILEPICT
            | CF_PALETTE
            | CF_ENHMETAFILE
            | CF_OWNERDISPLAY
            | CF_DSPBITMAP
            | CF_DSPMETAFILEPICT
            | CF_DSPENHMETAFILE
    )
}

/// CF_UNICODETEXT payload: UTF-16LE, NUL-terminated, CRLF line endings.
pub fn unicode_text_bytes(text: &str) -> Vec<u8> {
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    crlf.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|u| u.to_le_bytes())
        .collect()
}

/// Paste-layer markers flow-core emits for Cursor/Windsurf "@file"
/// tagging (`postprocess::cursor_tag`): `{{BREAK}}` = split the insertion
/// (let the mention popup open), `{{TAB}}` = press Tab (accept it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment<'a> {
    Text(&'a str),
    Tab,
    Break,
}

pub const BREAK_MARKER: &str = "{{BREAK}}";
pub const TAB_MARKER: &str = "{{TAB}}";
/// Pause at a `{{BREAK}}` so the target app reacts to the `@` first.
pub const BREAK_PAUSE_MS: u64 = 150;

pub fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let b = rest.find(BREAK_MARKER);
        let t = rest.find(TAB_MARKER);
        let (at, seg, len) = match (b, t) {
            (Some(b), Some(t)) if t < b => (t, Segment::Tab, TAB_MARKER.len()),
            (Some(b), _) => (b, Segment::Break, BREAK_MARKER.len()),
            (None, Some(t)) => (t, Segment::Tab, TAB_MARKER.len()),
            (None, None) => {
                out.push(Segment::Text(rest));
                break;
            }
        };
        if at > 0 {
            out.push(Segment::Text(&rest[..at]));
        }
        out.push(seg);
        rest = &rest[at + len..];
    }
    out
}

/// `text` without paste markers: what the user sees after the paste, for
/// "copy last transcript" and learn-from-edits.
pub fn plain_text(text: &str) -> String {
    segments(text)
        .into_iter()
        .map(|s| match s {
            Segment::Text(t) => t,
            Segment::Tab => "\t",
            Segment::Break => "",
        })
        .collect()
}

/// Plain text (the common case) goes through the normal clipboard paste;
/// marker-carrying text is typed segment by segment (no clipboard, so the
/// user's clipboard needs no multi-step restore).
pub fn has_markers(text: &str) -> bool {
    text.contains(BREAK_MARKER) || text.contains(TAB_MARKER)
}

/// Mandatory integrity RIDs (`SECURITY_MANDATORY_*_RID`).
pub mod rid {
    pub const LOW: u32 = 0x1000;
    pub const MEDIUM: u32 = 0x2000;
    pub const MEDIUM_PLUS: u32 = 0x2100;
    pub const HIGH: u32 = 0x3000;
    pub const SYSTEM: u32 = 0x4000;
}

/// UIPI: input injected by a process is silently dropped by windows of a
/// higher integrity level — and `SendInput` still reports success, so the
/// only way to notice is to compare levels *before* sending. `None` (level
/// unreadable) = assume OK and let the paste try.
pub fn uipi_blocks(own: Option<u32>, target: Option<u32>) -> bool {
    matches!((own, target), (Some(o), Some(t)) if t > o)
}

pub const ELEVATED_TARGET_MSG: &str = "The app you're dictating into runs as administrator, so \
     Windows blocks pasting from Flow Clone. The text is on your clipboard: press Ctrl+V.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_markers() {
        assert_eq!(
            plain_text("see @{{BREAK}}main.rs{{TAB}} now"),
            "see @main.rs\t now"
        );
        assert_eq!(plain_text("plain"), "plain");
    }

    #[test]
    fn segments_split_cursor_markers() {
        use Segment::*;
        assert_eq!(segments("plain text"), vec![Text("plain text")]);
        assert!(!has_markers("plain text"));
        assert_eq!(
            segments("open @{{BREAK}}main.rs{{TAB}} now"),
            vec![Text("open @"), Break, Text("main.rs"), Tab, Text(" now")]
        );
        assert_eq!(segments("{{TAB}}{{BREAK}}"), vec![Tab, Break]);
        assert_eq!(segments(""), Vec::<Segment>::new());
        assert!(has_markers("a{{TAB}}"));
    }

    #[test]
    fn uipi_blocks_only_higher_integrity_targets() {
        assert!(uipi_blocks(Some(rid::MEDIUM), Some(rid::HIGH)), "admin cmd");
        assert!(uipi_blocks(Some(rid::MEDIUM), Some(rid::SYSTEM)));
        assert!(!uipi_blocks(Some(rid::MEDIUM), Some(rid::MEDIUM)));
        assert!(
            !uipi_blocks(Some(rid::MEDIUM), Some(rid::LOW)),
            "sandboxed browser"
        );
        assert!(
            !uipi_blocks(Some(rid::HIGH), Some(rid::HIGH)),
            "we run elevated too"
        );
        assert!(!uipi_blocks(Some(rid::MEDIUM), None), "unreadable = try");
        assert!(!uipi_blocks(None, Some(rid::HIGH)));
    }

    #[test]
    fn restore_delay_per_app() {
        assert_eq!(restore_delay_ms("notepad.exe"), 500);
        assert_eq!(restore_delay_ms("OUTLOOK.EXE"), 3000);
        assert_eq!(restore_delay_ms("olk.exe"), 3000);
        assert_eq!(restore_delay_ms("WINWORD.EXE"), 3000);
        assert_eq!(restore_delay_ms("WhatsApp.exe"), 3000);
        assert_eq!(restore_delay_ms("mstsc.exe"), 5000);
        assert_eq!(restore_delay_ms("AnyDesk.exe"), 5000);
        assert_eq!(restore_delay_ms("TeamViewer"), 5000);
        assert_eq!(restore_delay_ms(""), 500);
    }

    #[test]
    fn strategy_from_method() {
        assert_eq!(
            initial_strategy(PasteMethod::CtrlV),
            Strategy::Clipboard {
                shift_insert: false
            }
        );
        assert_eq!(
            initial_strategy(PasteMethod::ShiftInsert),
            Strategy::Clipboard { shift_insert: true }
        );
        assert_eq!(initial_strategy(PasteMethod::Type), Strategy::Type);
    }

    #[test]
    fn fallback_types_only_short_text() {
        assert_eq!(
            fallback_after_clipboard_failure("héllo", 5),
            Some(Strategy::Type)
        );
        assert_eq!(fallback_after_clipboard_failure("hello!", 5), None);
    }

    #[test]
    fn restore_guard() {
        assert!(should_restore(true, 41, 41));
        assert!(!should_restore(true, 41, 42), "someone copied meanwhile");
        assert!(!should_restore(false, 41, 41));
    }

    #[test]
    fn restorable_formats() {
        assert!(is_restorable_format(13), "CF_UNICODETEXT");
        assert!(is_restorable_format(8), "CF_DIB");
        assert!(is_restorable_format(15), "CF_HDROP");
        assert!(is_restorable_format(0xC123), "registered");
        for f in [2u32, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E] {
            assert!(!is_restorable_format(f), "{f:#x}");
        }
    }

    #[test]
    fn unicode_bytes_are_utf16le_nul_terminated_crlf() {
        assert_eq!(unicode_text_bytes("a"), vec![b'a', 0, 0, 0]);
        let b = unicode_text_bytes("a\nb");
        let units: Vec<u16> = b
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(units, vec![b'a' as u16, 13, 10, b'b' as u16, 0]);
        let b = unicode_text_bytes("x\r\ny");
        assert_eq!(b.len(), 5 * 2, "existing CRLF not doubled");
    }
}
