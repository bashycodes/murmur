//! Win32 clipboard paste (see module docs in `paste`).
//!
//! 1. snapshot every restorable clipboard format
//! 2. set CF_UNICODETEXT + history/monitor opt-out formats
//! 3. release physically-held modifiers (mask Win/Alt) and SendInput Ctrl+V
//! 4. wait, then restore the snapshot iff the sequence number is unchanged

use super::{
    fallback_after_clipboard_failure, initial_strategy, is_restorable_format, should_restore,
    unicode_text_bytes, PasteOptions, Strategy,
};
use crate::hotkeys::Key;
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
    fn open() -> Option<ClipGuard> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(ClipGuard);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        None
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

fn held_modifiers() -> Vec<Key> {
    [Key::Ctrl, Key::Win, Key::Alt, Key::Shift]
        .into_iter()
        .filter(|k| {
            input::vks_for(*k)
                .into_iter()
                .any(|vk| unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 })
        })
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
    let mut s = input::release_modifiers(&held_modifiers());
    s.extend(input::unicode_text(text));
    send_all(&s)
}

/// Insert `text` into the focused control. Blocking (sleeps for the restore
/// delay) — call from a worker thread.
pub fn paste(text: &str, opts: &PasteOptions) -> Result<(), String> {
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
    let mut strokes = input::release_modifiers(&held_modifiers());
    strokes.extend(input::paste_chord(shift_insert));
    let sent = send_all(&strokes);
    std::thread::sleep(Duration::from_millis(opts.restore_delay_ms));
    let now_seq = unsafe { GetClipboardSequenceNumber() };
    if should_restore(opts.restore_clipboard, our_seq, now_seq) {
        if let Some(_g) = ClipGuard::open() {
            unsafe { restore(&snap) };
        }
    }
    sent
}
