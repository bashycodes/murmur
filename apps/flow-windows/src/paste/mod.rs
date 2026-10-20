//! Text insertion (spec §4.6): clipboard + synthetic Ctrl+V with full
//! clipboard save/restore, KEYEVENTF_UNICODE typing as fallback.
//!
//! Decisions are pure (strategy choice, restore guard, which formats can be
//! snapshotted) and tested here; `win` does the Win32 calls.

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
    pub restore_delay_ms: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

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
