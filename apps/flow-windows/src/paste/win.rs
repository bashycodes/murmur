//! Win32 clipboard paste (see module docs in `paste`).
//!
//! 1. snapshot every restorable clipboard format
//! 2. set CF_UNICODETEXT + history/monitor opt-out formats
//! 3. masked key-up for held Alt/Win, then [Ctrl↓,V↓] … 50 ms … [V↑,Ctrl↑]
//! 4. wait 500/3000/5000 ms (per app), restore iff sequence number unchanged
//!
//! Every clipboard operation (paste, copy) holds [`CLIPBOARD_OP`] for its
//! whole snapshot → set → chord → restore cycle (review S15): two pastes
//! that overlap (paste-last pressed twice, a dictation finishing while a
//! History re-paste runs) would otherwise snapshot each other's text as
//! "the user's clipboard" and lose the real one.

use super::{
    fallback_after_clipboard_failure, initial_strategy, integrity, is_restorable_format,
    restore_delay_ms, segments, should_restore, uipi_blocks, unicode_text_bytes, PasteOptions,
    Segment, Strategy, BREAK_PAUSE_MS, ELEVATED_TARGET_MSG,
};
use crate::input::{self, Stroke};
use std::time::Duration;
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

/// Serialises clipboard operations process-wide (S15).
static CLIPBOARD_OP: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn clipboard_op() -> std::sync::MutexGuard<'static, ()> {
    CLIPBOARD_OP
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

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
    set_privacy_formats();
    true
}

/// Keep dictated text out of Win+V history, the cloud clipboard and
/// clipboard monitors (the opt-outs Wispr's helper sets, 02 §3; S14).
/// Call with the clipboard open, after `EmptyClipboard`.
unsafe fn set_privacy_formats() {
    let zero = 0u32.to_le_bytes();
    for name in super::PRIVACY_FORMATS {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let f = RegisterClipboardFormatW(windows::core::PCWSTR(wide.as_ptr()));
        if f != 0 {
            set_bytes(f, &zero);
        }
    }
}

unsafe fn restore(snap: &Snapshot) {
    if EmptyClipboard().is_err() {
        return;
    }
    for (fmt, bytes) in snap {
        set_bytes(*fmt, bytes);
    }
}

fn physically_down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 }
}

fn ctrl_held() -> bool {
    physically_down(0xA2) || physically_down(0xA3)
}

/// Wait (bounded) until the user has let go of every chord modifier, so a
/// shortcut-triggered paste (paste-last: Shift+Alt+Z) is not read as
/// Ctrl+Shift+V (review C13).
pub fn wait_modifiers_released(timeout: Duration) {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < timeout && input::CHORD_MODIFIER_VKS.into_iter().any(physically_down) {
        std::thread::sleep(Duration::from_millis(15));
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
    let mut s = input::release_before_paste(&held_alt_win_vks(), ctrl_held());
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

/// "Copy last transcript" / History → Copy: the text stays on the
/// clipboard for the user's own Ctrl+V, but like a paste it is kept out of
/// Win+V history, the cloud clipboard and clipboard monitors (S14): a
/// transcript must not sync to Microsoft or linger in history.
pub fn copy_text(text: &str) -> Result<(), String> {
    let _op = clipboard_op();
    let _g = ClipGuard::open().ok_or("clipboard is busy")?;
    unsafe {
        if !set_text_excluded_from_history(text) {
            return Err("could not set the clipboard".into());
        }
    }
    Ok(())
}

/// Read CF_UNICODETEXT (tests and diagnostics).
pub fn read_text() -> Option<String> {
    let _g = ClipGuard::open()?;
    unsafe {
        let h = GetClipboardData(CF_UNICODETEXT).ok()?;
        let g = HGLOBAL(h.0);
        let p = GlobalLock(g) as *const u16;
        if p.is_null() {
            return None;
        }
        let n = GlobalSize(g) / 2;
        let units = std::slice::from_raw_parts(p, n);
        let end = units.iter().position(|u| *u == 0).unwrap_or(n);
        let s = String::from_utf16_lossy(&units[..end]);
        let _ = GlobalUnlock(g);
        Some(s)
    }
}

/// Whether the clipboard currently carries the registered format `name`.
pub fn has_format(name: &str) -> bool {
    let Some(_g) = ClipGuard::open() else {
        return false;
    };
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let f = RegisterClipboardFormatW(windows::core::PCWSTR(wide.as_ptr()));
        let mut fmt = 0u32;
        loop {
            fmt = EnumClipboardFormats(fmt);
            if fmt == 0 {
                return false;
            }
            if fmt == f {
                return true;
            }
        }
    }
}

/// Insert `text` into the focused control. Blocking (sleeps for the restore
/// delay) — call from a worker thread. Serialised with every other
/// clipboard operation of this process (S15).
pub fn paste(text: &str, opts: &PasteOptions) -> Result<(), String> {
    let _op = clipboard_op();
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
    let snap = {
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
            snap
        }
    };
    // Read after CloseClipboard (review C14): the number our own write
    // ends at, so any later change is someone else's.
    let our_seq = unsafe { GetClipboardSequenceNumber() };
    let delay = opts.restore_delay_ms.unwrap_or_else(|| {
        let (_, exe, _) = crate::context::uia::foreground_app();
        restore_delay_ms(&crate::context::exe_name(&exe))
    });
    let release = input::release_before_paste(&held_alt_win_vks(), ctrl_held());
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
            // Re-check with the clipboard held: a copy that landed while
            // we waited for OpenClipboard is the user's and stays (C14).
            if should_restore(true, our_seq, unsafe { GetClipboardSequenceNumber() }) {
                unsafe { restore(&snap) };
            }
        }
    }
    sent
}

/// Real-clipboard tests (Windows CI). They skip when the session has no
/// usable clipboard (service desktops).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::PasteMethod;

    /// The clipboard is process-global: one test at a time.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn usable() -> bool {
        if copy_text("probe").is_err() || read_text().as_deref() != Some("probe") {
            eprintln!("no usable clipboard in this session; skipping");
            return false;
        }
        true
    }

    fn set_plain(text: &str) {
        let _g = ClipGuard::open().expect("clipboard");
        unsafe {
            assert!(EmptyClipboard().is_ok());
            assert!(set_bytes(CF_UNICODETEXT, &unicode_text_bytes(text)));
        }
    }

    fn opts() -> PasteOptions {
        PasteOptions {
            method: PasteMethod::CtrlV,
            restore_clipboard: true,
            restore_delay_ms: Some(80),
            type_fallback_max_chars: 0,
        }
    }

    #[test]
    fn copy_is_kept_out_of_history_and_cloud_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        copy_text("secret transcript").unwrap();
        assert_eq!(read_text().as_deref(), Some("secret transcript"));
        for f in crate::paste::PRIVACY_FORMATS {
            assert!(has_format(f), "{f} missing on a copy (S14)");
        }
    }

    #[test]
    fn a_paste_restores_the_users_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let _ = paste("dictated", &opts());
        assert_eq!(read_text().as_deref(), Some("the user's clipboard"));
    }

    #[test]
    fn overlapping_pastes_never_lose_the_users_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let threads: Vec<_> = ["first", "second", "third"]
            .into_iter()
            .map(|t| std::thread::spawn(move || paste(t, &opts())))
            .collect();
        for t in threads {
            let _ = t.join();
        }
        assert_eq!(
            read_text().as_deref(),
            Some("the user's clipboard"),
            "S15: a paste must not snapshot another paste's text"
        );
    }
}
