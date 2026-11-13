//! Win32 clipboard paste (see module docs in `paste`).
//!
//! 1. snapshot every restorable clipboard format
//! 2. set history/monitor opt-out formats, and CF_UNICODETEXT delay-rendered
//!    by a message-only window of this thread (D93)
//! 3. masked key-up for held Alt/Win, then [Ctrl↓,V↓] … 50 ms … [V↑,Ctrl↑]
//! 4. wait 500/3000/5000 ms (per app), pumping messages: the target's read
//!    of the text arrives as WM_RENDERFORMAT, which confirms the paste
//! 5. nobody read it: the chord once more (same window only), then
//!    [`NOT_CONFIRMED_MSG`] with the text left on the clipboard
//! 6. restore iff our window still owns the clipboard
//!
//! Every clipboard operation (paste, copy) holds [`CLIPBOARD_OP`] for its
//! whole snapshot → set → chord → restore cycle (review S15): two pastes
//! that overlap (paste-last pressed twice, a dictation finishing while a
//! History re-paste runs) would otherwise snapshot each other's text as
//! "the user's clipboard" and lose the real one.

use super::{
    after_chord, fallback_after_clipboard_failure, initial_strategy, integrity,
    is_restorable_format, restore_delay_ms, segments, selection_from_copy, should_restore,
    should_restore_owned, uipi_blocks, unicode_text_bytes, AfterChord, PasteOptions, Segment,
    Strategy, BREAK_PAUSE_MS, COPY_WAIT_MS, ELEVATED_TARGET_MSG, NOT_CONFIRMED_MSG,
};
use crate::input::{self, Stroke};
use std::cell::RefCell;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, BOOL, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    MsgWaitForMultipleObjects, PeekMessageW, RegisterClassExW, HWND_MESSAGE, MSG, PM_REMOVE,
    QS_ALLINPUT, WINDOW_EX_STYLE, WINDOW_STYLE, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WNDCLASSEXW,
};

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
    /// Backoff 5, 10, 20, 40, 80, then the rest of a 250 ms budget.
    fn open() -> Option<ClipGuard> {
        Self::open_as(None)
    }

    /// Same, opened by `owner`: `EmptyClipboard` then makes it the
    /// clipboard owner (it gets the delayed-rendering messages).
    fn open_as(owner: Option<HWND>) -> Option<ClipGuard> {
        let mut waited = 0u64;
        let mut step = 5u64;
        loop {
            if unsafe { OpenClipboard(owner.unwrap_or_default()) }.is_ok() {
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
/// clipboard monitors (the standard opt-out formats; S14).
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
/// modifier is never left stuck down. The hold pumps messages: the target
/// may ask for the text at once (D93).
fn send_chord(shift_insert: bool) -> Result<(), String> {
    let (downs, ups) = input::paste_chord(shift_insert);
    let sent = send_all(&downs);
    pump_for(Duration::from_millis(input::PASTE_HOLD_MS));
    if input::win::send(&ups) != ups.len() {
        input::win::send(&ups);
    }
    sent
}

/// Command Mode, when UI Automation can't read the selection: Ctrl+C in
/// the focused app, read what it put on the clipboard, then put the
/// user's clipboard back (if nobody else wrote to it since). `Ok(None)` =
/// the clipboard did not change within [`COPY_WAIT_MS`] (nothing
/// selected). Call after the shortcut keys are up; serialised with every
/// paste (S15) so a paste never snapshots our copy.
pub fn copy_selection() -> Result<Option<String>, String> {
    let _op = clipboard_op();
    if uipi_blocks(integrity::own(), integrity::foreground()) {
        return Err(ELEVATED_TARGET_MSG.into());
    }
    let snap = {
        let _g = ClipGuard::open().ok_or("clipboard is busy")?;
        unsafe { snapshot() }
    };
    let before = unsafe { GetClipboardSequenceNumber() };
    send_all(&input::release_before_paste(
        &held_alt_win_vks(),
        ctrl_held(),
    ))?;
    let (downs, ups) = input::copy_chord();
    let sent = send_all(&downs);
    std::thread::sleep(Duration::from_millis(input::PASTE_HOLD_MS));
    if input::win::send(&ups) != ups.len() {
        input::win::send(&ups);
    }
    sent?;
    let t0 = std::time::Instant::now();
    let mut changed = false;
    while t0.elapsed() < Duration::from_millis(COPY_WAIT_MS) {
        if unsafe { GetClipboardSequenceNumber() } != before {
            changed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !changed {
        return Ok(None);
    }
    // Apps write several formats one SetClipboardData at a time (each
    // bumps the number): let the copy settle before reading it.
    std::thread::sleep(Duration::from_millis(40));
    let copied_seq = unsafe { GetClipboardSequenceNumber() };
    let text = read_text();
    if let Some(_g) = ClipGuard::open() {
        if should_restore(true, copied_seq, unsafe { GetClipboardSequenceNumber() }) {
            unsafe { restore(&snap) };
        }
    }
    selection_from_copy(true, text)
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

/// The text a paste on this thread delay-renders (D93), and whether an
/// app asked for it.
struct Pending {
    bytes: Vec<u8>,
    read: bool,
}

thread_local! {
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
}

const OWNER_CLASS: PCWSTR = w!("MurmurPasteClipboardOwner");

/// Put the pending text on the clipboard (the caller holds it open).
/// `read` = an app asked for it.
fn render_pending(read: bool) {
    // No borrow is held across the Win32 call: a message sent to this
    // thread meanwhile may come back here.
    let Some(bytes) = PENDING.with(|p| p.try_borrow().ok()?.as_ref().map(|p| p.bytes.clone()))
    else {
        return;
    };
    let rendered = unsafe { set_bytes(CF_UNICODETEXT, &bytes) };
    PENDING.with(|p| {
        if let Some(p) = p
            .try_borrow_mut()
            .ok()
            .as_deref_mut()
            .and_then(Option::as_mut)
        {
            p.read |= read && rendered;
        }
    });
}

unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // An app's GetClipboardData (or a synthesised CF_TEXT): the app
        // holds the clipboard open, so only SetClipboardData here.
        WM_RENDERFORMAT if wp.0 == CF_UNICODETEXT as usize => {
            render_pending(true);
            LRESULT(0)
        }
        // The window is going away with the text unrendered: leave the
        // text on the clipboard.
        WM_RENDERALLFORMATS => {
            if OpenClipboard(hwnd).is_ok() {
                if GetClipboardOwner().ok() == Some(hwnd) {
                    render_pending(false);
                }
                let _ = CloseClipboard();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// D93: a message-only window that owns the clipboard while a paste runs.
/// It lives on, and is pumped by, the paste's thread.
struct OwnerWindow(HWND);

impl OwnerWindow {
    fn new(text: &str) -> Option<OwnerWindow> {
        static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None).ok()?.into();
            let registered = *REGISTERED.get_or_init(|| {
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(owner_proc),
                    hInstance: instance,
                    lpszClassName: OWNER_CLASS,
                    ..Default::default()
                };
                RegisterClassExW(&wc) != 0
            });
            if !registered {
                return None;
            }
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                OWNER_CLASS,
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                None,
                instance,
                None,
            )
            .ok()?;
            PENDING.with(|p| {
                *p.borrow_mut() = Some(Pending {
                    bytes: unicode_text_bytes(text),
                    read: false,
                })
            });
            Some(OwnerWindow(hwnd))
        }
    }

    fn read(&self) -> bool {
        PENDING.with(|p| p.borrow().as_ref().is_some_and(|p| p.read))
    }

    fn owns_clipboard(&self) -> bool {
        unsafe { GetClipboardOwner() }.ok() == Some(self.0)
    }
}

impl Drop for OwnerWindow {
    fn drop(&mut self) {
        // A window that still owns unrendered text gets WM_RENDERALLFORMATS
        // while it is destroyed, which needs PENDING: clear it after.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
        PENDING.with(|p| *p.borrow_mut() = None);
    }
}

/// Sleep `d` while dispatching this thread's messages (WM_RENDERFORMAT).
fn pump_for(d: Duration) {
    let end = Instant::now() + d;
    loop {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&msg);
            }
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        let ms = u32::try_from(left.as_millis()).unwrap_or(u32::MAX).max(1);
        unsafe {
            MsgWaitForMultipleObjects(None, BOOL(0), ms, QS_ALLINPUT);
        }
    }
}

/// Insert `text` into the focused control. Blocking (waits the restore
/// delay, twice when the paste is retried) — call from a worker thread.
/// Serialised with every other clipboard operation of this process (S15).
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
    // No window (should not happen): the text is set for real and the
    // paste can't be confirmed, as before D93.
    let owner = OwnerWindow::new(text);
    let snap = {
        let Some(_g) = ClipGuard::open_as(owner.as_ref().map(|o| o.0)) else {
            return match fallback_after_clipboard_failure(text, opts.type_fallback_max_chars) {
                Some(_) => type_text(text),
                None => Err("clipboard busy and text too long to type".into()),
            };
        };
        unsafe {
            let snap = snapshot();
            let set = match owner {
                Some(_) => offer_delayed(),
                None => set_text_excluded_from_history(text),
            };
            if !set {
                return Err("failed to write clipboard".into());
            }
            snap
        }
    };
    // Read after CloseClipboard (review C14): the number our own write
    // ends at, so any later change is someone else's (no-window case).
    let our_seq = unsafe { GetClipboardSequenceNumber() };
    let delay = Duration::from_millis(opts.restore_delay_ms.unwrap_or_else(|| {
        let (_, exe, _) = crate::context::uia::foreground_app();
        restore_delay_ms(&crate::context::exe_name(&exe))
    }));
    let window = unsafe { GetForegroundWindow() };
    let mut attempt = 0;
    let (sent, outcome) = loop {
        attempt += 1;
        let release = input::release_before_paste(&held_alt_win_vks(), ctrl_held());
        let mut sent = send_all(&release);
        let chord = send_chord(shift_insert);
        if sent.is_ok() {
            sent = chord;
        }
        // The target gets the per-app delay to read the text; its read
        // comes to us (messages are pumped meanwhile).
        pump_for(delay);
        let read = owner.as_ref().is_none_or(OwnerWindow::read);
        // A retry only into the same window, and only with our text still
        // on the clipboard (never someone's copy made since).
        let can_retry = sent.is_ok()
            && unsafe { GetForegroundWindow() } == window
            && owner.as_ref().is_some_and(OwnerWindow::owns_clipboard);
        match after_chord(read, attempt, can_retry) {
            AfterChord::Retry => continue,
            done => break (sent, done),
        }
    };
    if let (AfterChord::NotConfirmed, Some(o)) = (outcome, &owner) {
        // Leave the text for a manual Ctrl+V, rendered (the window goes).
        if let Some(_g) = ClipGuard::open_as(Some(o.0)) {
            if o.owns_clipboard() {
                render_pending(false);
            }
        }
        sent?;
        return Err(NOT_CONFIRMED_MSG.into());
    }
    if let Some(_g) = ClipGuard::open() {
        // Checked with the clipboard held: a copy the user/target made
        // since is theirs and stays (C14).
        let ours = match &owner {
            Some(o) => o.owns_clipboard(),
            None => should_restore(true, our_seq, unsafe { GetClipboardSequenceNumber() }),
        };
        if should_restore_owned(opts.restore_clipboard, ours) {
            unsafe { restore(&snap) };
        }
    }
    sent
}

/// D93, with the clipboard opened by the owner window: our text
/// delay-rendered (NULL data), the opt-out formats set for real.
unsafe fn offer_delayed() -> bool {
    if EmptyClipboard().is_err() {
        return false;
    }
    // The return value is NULL for delayed rendering, success or not.
    let _ = SetClipboardData(CF_UNICODETEXT, HANDLE::default());
    set_privacy_formats();
    true
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

    /// A stand-in target app: reads the clipboard text every few ms (as
    /// a paste would) until dropped; `seen` = the texts it read.
    struct Reader {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Reader {
        fn start() -> Reader {
            use std::sync::atomic::Ordering;
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let (s, v) = (stop.clone(), seen.clone());
            let thread = std::thread::spawn(move || {
                while !s.load(Ordering::SeqCst) {
                    if let Some(t) = read_text() {
                        v.lock().unwrap().push(t);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            });
            Reader {
                stop,
                seen,
                thread: Some(thread),
            }
        }

        fn saw(&self, text: &str) -> bool {
            self.seen.lock().unwrap().iter().any(|t| t == text)
        }
    }

    impl Drop for Reader {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    #[test]
    fn a_paste_restores_the_users_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let reader = Reader::start();
        let r = paste("dictated", &opts());
        assert!(
            reader.saw("dictated"),
            "the delayed text renders for a reader"
        );
        drop(reader);
        if let Err(e) = r {
            // Only SendInput can fail here (no desktop): never "not confirmed".
            assert_ne!(e, NOT_CONFIRMED_MSG, "a read confirms the paste (D93)");
        }
        assert_eq!(read_text().as_deref(), Some("the user's clipboard"));
    }

    #[test]
    fn an_unread_paste_is_reported_and_left_on_the_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        // Nobody reads, unless the session's foreground window takes the
        // Ctrl+V for real (then the paste is confirmed and restored).
        match paste("dictated", &opts()) {
            Err(e) if e == NOT_CONFIRMED_MSG => {
                assert_eq!(read_text().as_deref(), Some("dictated"), "left for Ctrl+V");
            }
            Ok(()) => assert_eq!(read_text().as_deref(), Some("the user's clipboard")),
            Err(_) => {} // SendInput failed: no desktop in this session
        }
    }

    #[test]
    fn a_copy_made_meanwhile_is_not_overwritten() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let reader = Reader::start();
        let copier = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(40));
            set_plain("copied meanwhile");
        });
        let _ = paste(
            "dictated",
            &PasteOptions {
                restore_delay_ms: Some(200),
                ..opts()
            },
        );
        copier.join().unwrap();
        drop(reader);
        assert_eq!(read_text().as_deref(), Some("copied meanwhile"));
    }

    #[test]
    fn overlapping_pastes_never_lose_the_users_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let _reader = Reader::start();
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
