//! Win32 clipboard paste (see module docs in `paste`).
//!
//! 1. snapshot every restorable clipboard format
//! 2. set CF_UNICODETEXT + history/monitor opt-out formats
//! 3. masked key-up for held Alt/Win, then [Ctrl↓,V↓] … 50 ms … [V↑,Ctrl↑]
//! 4. wait 500/3000/5000 ms (per app), restore iff sequence number unchanged

use super::{
    fallback_after_clipboard_failure, initial_strategy, integrity, is_restorable_format,
    restore_delay_ms, segments, should_restore, uipi_blocks, unicode_text_bytes, PasteOptions,
    Segment, Strategy, BREAK_PAUSE_MS, ELEVATED_TARGET_MSG,
};
use crate::input::{self, Stroke};
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

const CF_UNICODETEXT: u32 = 13;

struct ClipGuard;

impl ClipGuard {
    /// OpenClipboard with retries: another process (clipboard managers, RDP)
    /// commonly holds it for a few ms.
    /// Backoff 5, 10, 20, 40, 80, then the rest of a 250 ms budget (Wispr).
    fn open() -> Option<ClipGuard> {
        let mut waited = 0u64;
        let mut step = 5u64;
        loop {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(ClipGuard);
            }
            if waited >= 250 {
                return None;
            }
            let d = step.min(250 - waited);
            std::thread::sleep(Duration::from_millis(d));
            waited += d;
            step *= 2;
        }
    }
}

impl Drop for ClipGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

type Snapshot = Vec<(u32, Vec<u8>)>;

unsafe fn snapshot() -> Snapshot {
    let mut out = Vec::new();
    let mut fmt = 0u32;
    loop {
        fmt = EnumClipboardFormats(fmt);
        if fmt == 0 {
            break;
        }
        if !is_restorable_format(fmt) {
            continue;
        }
        let Ok(h) = GetClipboardData(fmt) else {
            continue;
        };
        let g = HGLOBAL(h.0);
        let size = GlobalSize(g);
        let p = GlobalLock(g) as *const u8;
        if p.is_null() {
            continue;
        }
        let bytes = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(g);
        out.push((fmt, bytes));
    }
    out
}

unsafe fn set_bytes(fmt: u32, bytes: &[u8]) -> bool {
    let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else {
        return false;
    };
    let p = GlobalLock(g) as *mut u8;
    if p.is_null() {
        let _ = GlobalFree(g);
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    let _ = GlobalUnlock(g);
    if SetClipboardData(fmt, HANDLE(g.0)).is_err() {
        // Ownership only transfers on success.
        let _ = GlobalFree(g);
        return false;
    }
    true
}

unsafe fn set_text_excluded_from_history(text: &str) -> bool {
    if EmptyClipboard().is_err() {
        return false;
    }
    if !set_bytes(CF_UNICODETEXT, &unicode_text_bytes(text)) {
        return false;
    }
    // Keep dictated text out of Win+V history, cloud clipboard and clipboard
    // monitors (same opt-outs Wispr's helper sets, 02 §3).
    let zero = 0u32.to_le_bytes();
    let f = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
    if f != 0 {
        set_bytes(f, &zero);
    }
    let f = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
    if f != 0 {
        set_bytes(f, &zero);
    }
    let f = RegisterClipboardFormatW(w!("CanUploadToCloudClipboard"));
    if f != 0 {
        set_bytes(f, &zero);
    }
    let f = RegisterClipboardFormatW(w!("Clipboard Viewer Ignore"));
    if f != 0 {
        set_bytes(f, &zero);
    }
    true
}

unsafe fn restore(snap: &Snapshot) {
    if EmptyClipboard().is_err() {
        return;
    }
    for (fmt, bytes) in snap {
        set_bytes(*fmt, bytes);
    }
}

/// Concrete VKs (L/R) currently physically held among Alt/Win.
fn held_alt_win_vks() -> Vec<u16> {
    input::RELEASE_BEFORE_PASTE_VKS
        .into_iter()
        .filter(|vk| unsafe { (GetAsyncKeyState(*vk as i32) as u16) & 0x8000 != 0 })
        .collect()
}

fn send_all(strokes: &[Stroke]) -> Result<(), String> {
    let n = input::win::send(strokes);
    if n == strokes.len() {
        Ok(())
    } else {
        Err(format!(
            "SendInput inserted {n}/{} events (target app elevated? UIPI blocks input)",
            strokes.len()
        ))
    }
}

fn type_text(text: &str) -> Result<(), String> {
    let mut s = input::release_before_paste(&held_alt_win_vks());
    s.extend(input::unicode_text(text));
    send_all(&s)
}

/// Cursor/Windsurf "@file" output: type text, Tab to accept the mention,
/// pause at breaks so the popup opens first.
fn type_segments(text: &str) -> Result<(), String> {
    for seg in segments(text) {
        match seg {
            Segment::Text(t) => type_text(t)?,
            Segment::Tab => send_all(&[Stroke::down(input::vk::TAB), Stroke::up(input::vk::TAB)])?,
            Segment::Break => std::thread::sleep(Duration::from_millis(BREAK_PAUSE_MS)),
        }
    }
    Ok(())
}

/// `[downs]` → hold 50 ms → `[ups]`; a short up batch is retried once so a
/// modifier is never left stuck down (Wispr 0x1403231ca).
fn send_chord(shift_insert: bool) -> Result<(), String> {
    let (downs, ups) = input::paste_chord(shift_insert);
    let sent = send_all(&downs);
    std::thread::sleep(Duration::from_millis(input::PASTE_HOLD_MS));
    if input::win::send(&ups) != ups.len() {
        input::win::send(&ups);
    }
    sent
}

/// Insert `text` into the focused control. Blocking (sleeps for the restore
/// delay) — call from a worker thread.
/// "Copy last transcript": an ordinary clipboard copy (unlike a paste it
/// is meant to stay, so it is not excluded from Win+V history).
pub fn copy_text(text: &str) -> Result<(), String> {
    let _g = ClipGuard::open().ok_or("clipboard is busy")?;
    unsafe {
        if EmptyClipboard().is_err() || !set_bytes(CF_UNICODETEXT, &unicode_text_bytes(text)) {
            return Err("could not set the clipboard".into());
        }
    }
    Ok(())
}

pub fn paste(text: &str, opts: &PasteOptions) -> Result<(), String> {
    // UIPI drops our input silently (SendInput still "succeeds"), so check
    // first and leave the text on the clipboard for a manual Ctrl+V.
    if uipi_blocks(integrity::own(), integrity::foreground()) {
        if let Some(_g) = ClipGuard::open() {
            unsafe { set_text_excluded_from_history(text) };
        }
        return Err(ELEVATED_TARGET_MSG.into());
    }
    if super::has_markers(text) {
        return type_segments(text);
    }
    let shift_insert = match initial_strategy(opts.method) {
        Strategy::Type => return type_text(text),
        Strategy::Clipboard { shift_insert } => shift_insert,
    };
    let (snap, our_seq) = {
        let Some(_g) = ClipGuard::open() else {
            return match fallback_after_clipboard_failure(text, opts.type_fallback_max_chars) {
                Some(_) => type_text(text),
                None => Err("clipboard busy and text too long to type".into()),
            };
        };
        unsafe {
            let snap = snapshot();
            if !set_text_excluded_from_history(text) {
                return Err("failed to write clipboard".into());
            }
            (snap, GetClipboardSequenceNumber())
        }
    };
    let delay = opts.restore_delay_ms.unwrap_or_else(|| {
        let (_, exe, _) = crate::context::uia::foreground_app();
        restore_delay_ms(&crate::context::exe_name(&exe))
    });
    let release = input::release_before_paste(&held_alt_win_vks());
    let mut sent = send_all(&release);
    let chord = send_chord(shift_insert);
    if sent.is_ok() {
        sent = chord;
    }
    std::thread::sleep(Duration::from_millis(delay));
    // Unlike Wispr's immediate path we also check the sequence number, so a
    // copy made by the user/target meanwhile is never clobbered.
    let now_seq = unsafe { GetClipboardSequenceNumber() };
    if should_restore(opts.restore_clipboard, our_seq, now_seq) {
        if let Some(_g) = ClipGuard::open() {
            unsafe { restore(&snap) };
        }
    }
    sent
}
