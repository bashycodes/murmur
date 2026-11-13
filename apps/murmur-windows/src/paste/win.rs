//! Win32 clipboard paste (see module docs in `paste`).
//!
//! 1. snapshot every restorable clipboard format
//! 2. set history/monitor opt-out formats, and CF_UNICODETEXT delay-rendered
//!    by a message-only window of this thread (D93)
//! 3. pump messages until the clipboard is quiet: listeners that fetch the
//!    text get it now (rendered, then offered delay-rendered again), so
//!    none holds the clipboard when the target's paste opens it
//! 4. masked key-up for held Alt/Win, then [Ctrl↓,V↓] … 50 ms … [V↑,Ctrl↑]
//! 5. pump messages: a read by the target's process arrives as
//!    WM_RENDERFORMAT and confirms the paste; wait the per-app restore
//!    delay (500/3000/5000 ms), and without a target read at least
//!    [`CONFIRM_WAIT_MS`]
//! 6. no target read: no second chord (it could paste twice), an error
//!    instead ([`not_confirmed_message`]) with our text left on the
//!    clipboard; read: restore iff the clipboard still holds our paste
//!
//! Every clipboard operation (paste, copy) holds [`CLIPBOARD_OP`] for its
//! whole snapshot → set → chord → restore cycle (review S15): two pastes
//! that overlap (paste-last pressed twice, a dictation finishing while a
//! History re-paste runs) would otherwise snapshot each other's text as
//! "the user's clipboard" and lose the real one.
//!
//! [`paste`] runs on a worker thread, never the UI thread: it creates and
//! pumps its own window there (D93).

use super::{
    after_wait, fallback_after_clipboard_failure, initial_strategy, integrity,
    is_restorable_format, not_confirmed_message, reader_is_target, restore_delay_ms, segments,
    selection_from_copy, should_restore, uipi_blocks, unicode_text_bytes, AfterWait, PasteOptions,
    Segment, Strategy, BREAK_PAUSE_MS, COPY_WAIT_MS, ELEVATED_TARGET_MSG,
};
use crate::input::{self, Stroke};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, BOOL, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, GetOpenClipboardWindow, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumChildWindows,
    GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, MsgWaitForMultipleObjects,
    PeekMessageW, RegisterClassExW, SetWindowLongPtrW, GWLP_USERDATA, HWND_MESSAGE, MSG, PM_REMOVE,
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
        if should_restore(true, copied_seq == unsafe { GetClipboardSequenceNumber() }) {
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
/// D93: the text the owner window renders, and who read it. Owned by
/// [`OwnerWindow`]; its window proc reaches it through `GWLP_USERDATA`.
struct Pending {
    bytes: Vec<u8>,
    /// Processes whose read confirms the paste (set right before the
    /// chord; empty before it, so no early read confirms).
    targets: RefCell<Vec<u32>>,
    /// Someone made us render the text since it was last offered.
    rendered: Cell<bool>,
    /// The target's process read it.
    target_read: Cell<bool>,
}

impl Pending {
    /// Put the text on the clipboard (the reader, or we, hold it open).
    /// `reader` = the process that opened the clipboard (`None` = unknown,
    /// or us).
    fn render(&self, reader: Option<u32>) {
        if unsafe { set_bytes(CF_UNICODETEXT, &self.bytes) } {
            self.rendered.set(true);
            // A listener's read renders the text but doesn't confirm.
            if self
                .targets
                .try_borrow()
                .is_ok_and(|t| reader_is_target(reader, &t))
            {
                self.target_read.set(true);
            }
        }
    }
}

const OWNER_CLASS: PCWSTR = w!("MurmurPasteClipboardOwner");

/// The process of `h` (0 = none).
fn window_pid(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid as *mut u32)) };
    pid
}

/// The process that has the clipboard open, if it opened it with a window.
fn clipboard_reader() -> Option<u32> {
    let h = unsafe { GetOpenClipboardWindow() }.ok()?;
    Some(window_pid(h)).filter(|p| *p != 0)
}

/// The target of a paste: the foreground window's process and those of
/// its child windows (a UWP app's CoreWindow sits in its frame).
fn foreground_targets() -> Vec<u32> {
    unsafe extern "system" fn add(h: HWND, lp: LPARAM) -> BOOL {
        let pids = &mut *(lp.0 as *mut Vec<u32>);
        let pid = window_pid(h);
        if pid != 0 && !pids.contains(&pid) {
            pids.push(pid);
        }
        BOOL(1)
    }
    let fg = unsafe { GetForegroundWindow() };
    // No foreground: no target (a NULL parent would list every window).
    if fg.is_invalid() {
        return Vec::new();
    }
    let mut pids = vec![window_pid(fg)];
    unsafe {
        let _ = EnumChildWindows(fg, Some(add), LPARAM(&mut pids as *mut Vec<u32> as isize));
    }
    pids.retain(|p| *p != 0);
    pids
}

unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let pending = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Pending;
    match (msg, pending.as_ref()) {
        // An app's GetClipboardData (or a synthesised CF_TEXT): the app
        // holds the clipboard open, so only SetClipboardData here.
        (WM_RENDERFORMAT, Some(p)) if wp.0 == CF_UNICODETEXT as usize => {
            p.render(clipboard_reader());
            LRESULT(0)
        }
        // The window is going away with the text unrendered: leave the
        // text on the clipboard.
        (WM_RENDERALLFORMATS, Some(p)) => {
            if OpenClipboard(hwnd).is_ok() {
                if GetClipboardOwner().ok() == Some(hwnd) {
                    p.render(None);
                }
                let _ = CloseClipboard();
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// D93: a message-only window that owns the clipboard while a paste runs.
/// It lives on, and is pumped by, the paste's (worker) thread.
struct OwnerWindow {
    hwnd: HWND,
    /// Read by the window proc; outlives the window (dropped after it).
    pending: Box<Pending>,
}

impl OwnerWindow {
    fn new(text: &str) -> Result<OwnerWindow, String> {
        static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)
                .map_err(|e| format!("GetModuleHandleW: {e}"))?
                .into();
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
                return Err("RegisterClassExW failed".into());
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
            .map_err(|e| format!("CreateWindowExW: {e}"))?;
            let pending = Box::new(Pending {
                bytes: unicode_text_bytes(text),
                targets: RefCell::new(Vec::new()),
                rendered: Cell::new(false),
                target_read: Cell::new(false),
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*pending as *const Pending as isize);
            Ok(OwnerWindow { hwnd, pending })
        }
    }

    fn owns_clipboard(&self) -> bool {
        unsafe { GetClipboardOwner() }.ok() == Some(self.hwnd)
    }

    /// With the clipboard opened by this window: our text delay-rendered
    /// (NULL data), the opt-out formats set for real.
    unsafe fn offer(&self) -> bool {
        if EmptyClipboard().is_err() {
            return false;
        }
        // The return value is NULL for delayed rendering, success or not.
        let _ = SetClipboardData(CF_UNICODETEXT, HANDLE::default());
        set_privacy_formats();
        self.pending.rendered.set(false);
        true
    }
}

impl Drop for OwnerWindow {
    fn drop(&mut self) {
        // Unrendered text gets WM_RENDERALLFORMATS while the window is
        // destroyed: `pending` is still alive then.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
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

/// Before the chord, the clipboard must be quiet for this long…
const SETTLE_QUIET_MS: u64 = 20;
/// …waiting at most this long per offer…
const SETTLE_CAP_MS: u64 = 150;
/// …and offering the text again at most this often after a listener
/// fetched it.
const SETTLE_REOFFERS: u32 = 2;

/// D93: our write wakes every clipboard listener, and each opens the
/// clipboard. Pump until none has it open, so the target's single
/// `OpenClipboard` (an EDIT's WM_PASTE) doesn't find it busy. A listener
/// that fetched the text got it rendered: offer it delay-rendered again,
/// so the target's read still reaches us.
fn settle(owner: &OwnerWindow) {
    for _ in 0..=SETTLE_REOFFERS {
        let t0 = Instant::now();
        let mut quiet_since = t0;
        while t0.elapsed() < Duration::from_millis(SETTLE_CAP_MS) {
            pump_for(Duration::from_millis(2));
            if unsafe { GetOpenClipboardWindow() }.is_ok() {
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() >= Duration::from_millis(SETTLE_QUIET_MS) {
                break;
            }
        }
        if !owner.pending.rendered.get() {
            return;
        }
        let Some(_g) = ClipGuard::open_as(Some(owner.hwnd)) else {
            return;
        };
        if !owner.owns_clipboard() || !unsafe { owner.offer() } {
            return;
        }
    }
}

/// Insert `text` into the focused control. Blocking (waits the restore
/// delay, [`CONFIRM_WAIT_MS`](super::CONFIRM_WAIT_MS) when unconfirmed) —
/// call from a worker thread, never the UI thread: it creates and pumps
/// its own window. Serialised with every other clipboard operation of
/// this process (S15).
pub fn paste(text: &str, opts: &PasteOptions) -> Result<(), String> {
    debug_assert_ne!(
        std::thread::current().name(),
        Some("main"),
        "paste blocks and pumps its own window: call it from a worker thread"
    );
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
    let chord = || {
        let release = input::release_before_paste(&held_alt_win_vks(), ctrl_held());
        let sent = send_all(&release);
        let chord = send_chord(shift_insert);
        sent.and(chord)
    };
    paste_clipboard(text, opts, &chord, &foreground_targets)
}

/// The clipboard paste, with the chord and the target lookup passed in
/// (tests). The caller holds [`CLIPBOARD_OP`].
fn paste_clipboard(
    text: &str,
    opts: &PasteOptions,
    chord: &dyn Fn() -> Result<(), String>,
    targets: &dyn Fn() -> Vec<u32>,
) -> Result<(), String> {
    let owner = match OwnerWindow::new(text) {
        Ok(o) => o,
        Err(e) => {
            // Should not happen: paste as before D93, unconfirmed.
            eprintln!("[paste] no clipboard owner window ({e}): pasting unconfirmed");
            crate::debug_log::event("paste", &format!("unconfirmed paste: {e}"));
            return paste_unconfirmed(text, opts, chord);
        }
    };
    let snap = {
        let Some(_g) = ClipGuard::open_as(Some(owner.hwnd)) else {
            return match fallback_after_clipboard_failure(text, opts.type_fallback_max_chars) {
                Some(_) => type_text(text),
                None => Err("clipboard busy and text too long to type".into()),
            };
        };
        unsafe {
            let snap = snapshot();
            if !owner.offer() {
                return Err("failed to write clipboard".into());
            }
            snap
        }
    };
    let delay_ms = opts.restore_delay_ms.unwrap_or_else(|| {
        let (_, exe, _) = crate::context::uia::foreground_app();
        restore_delay_ms(&crate::context::exe_name(&exe))
    });
    settle(&owner);
    *owner.pending.targets.borrow_mut() = targets();
    let sent = chord();
    let t0 = Instant::now();
    let confirmed = loop {
        pump_for(Duration::from_millis(10));
        if sent.is_err() && t0.elapsed() >= Duration::from_millis(delay_ms) {
            break true; // the keys didn't go out: report that, below
        }
        let waited = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        match after_wait(owner.pending.target_read.get(), waited, delay_ms) {
            AfterWait::Wait => {}
            AfterWait::Confirmed => break true,
            AfterWait::NotConfirmed => break false,
        }
    };
    if !confirmed {
        // Our text stays on the clipboard for a manual Ctrl+V (rendered:
        // the window goes); the user's earlier copy is not restored.
        let ours = match ClipGuard::open_as(Some(owner.hwnd)) {
            Some(_g) if owner.owns_clipboard() => {
                owner.pending.render(None);
                true
            }
            _ => false,
        };
        return Err(not_confirmed_message(ours).into());
    }
    if let Some(_g) = ClipGuard::open() {
        // Checked with the clipboard held: a copy the user/target made
        // since is theirs and stays (C14).
        if should_restore(opts.restore_clipboard, owner.owns_clipboard()) {
            unsafe { restore(&snap) };
        }
    }
    sent
}

/// The paste before D93 (no owner window): the text set for real, the
/// restore keyed on the sequence number, success = the keys went out.
fn paste_unconfirmed(
    text: &str,
    opts: &PasteOptions,
    chord: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
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
    let sent = chord();
    std::thread::sleep(Duration::from_millis(delay));
    if let Some(_g) = ClipGuard::open() {
        let ours = our_seq == unsafe { GetClipboardSequenceNumber() };
        if should_restore(opts.restore_clipboard, ours) {
            unsafe { restore(&snap) };
        }
    }
    sent
}

/// Real-clipboard tests (Windows CI). They skip, saying why, when the
/// session has no usable clipboard (service desktops). The D93 tests
/// inject the chord (no keys are sent) and the target process: whether a
/// real app takes a real Ctrl+V can't be checked in CI, only on a desktop
/// (tools/e2e-windows, scenario c).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::paste::{NOT_CONFIRMED_CLIPBOARD_TAKEN_MSG, NOT_CONFIRMED_MSG};
    use crate::prefs::PasteMethod;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// The clipboard is process-global: one test at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn usable() -> bool {
        if copy_text("probe").is_err() || read_text().as_deref() != Some("probe") {
            eprintln!("SKIPPED: no usable clipboard in this session (service desktop)");
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

    /// The clipboard paste with no keys sent, as the process `targets`
    /// names the target.
    fn paste_as(text: &str, opts: &PasteOptions, targets: Vec<u32>) -> Result<(), String> {
        let _op = clipboard_op();
        paste_clipboard(text, opts, &|| Ok(()), &move || targets.clone())
    }

    fn us() -> Vec<u32> {
        vec![std::process::id()]
    }

    /// A stand-in for another app: reads the clipboard text every few ms
    /// with a window of its own (so its process is known), until dropped.
    /// It runs in this process, so it is the target when the paste's
    /// targets are [`us`], and a clipboard listener otherwise.
    struct Reader {
        stop: Arc<AtomicBool>,
        seen: Arc<Mutex<Vec<String>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Reader {
        fn start() -> Reader {
            let stop = Arc::new(AtomicBool::new(false));
            let seen = Arc::new(Mutex::new(Vec::new()));
            let (s, v) = (stop.clone(), seen.clone());
            let thread = std::thread::spawn(move || {
                let hwnd = unsafe {
                    CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        w!("STATIC"),
                        PCWSTR::null(),
                        WINDOW_STYLE(0),
                        0,
                        0,
                        0,
                        0,
                        HWND_MESSAGE,
                        None,
                        None,
                        None,
                    )
                }
                .expect("reader window");
                while !s.load(Ordering::SeqCst) {
                    if let Some(t) = read_text_as(hwnd) {
                        v.lock().unwrap().push(t);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                unsafe {
                    let _ = DestroyWindow(hwnd);
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
            self.stop.store(true, Ordering::SeqCst);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// CF_UNICODETEXT, read with the clipboard opened by `hwnd`.
    fn read_text_as(hwnd: HWND) -> Option<String> {
        let _g = ClipGuard::open_as(Some(hwnd))?;
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
    fn a_target_read_confirms_the_paste_and_restores_the_users_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let reader = Reader::start();
        let r = paste_as("dictated", &opts(), us());
        assert!(
            reader.saw("dictated"),
            "the delayed text renders for a reader"
        );
        drop(reader);
        assert_eq!(r, Ok(()));
        assert_eq!(read_text().as_deref(), Some("the user's clipboard"));
    }

    /// Regression (ticket "paste reports success but nothing lands"): 1.3.0
    /// returned Ok whenever the keys went out.
    #[test]
    fn a_paste_nobody_reads_is_reported_and_left_on_the_clipboard() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        assert_eq!(
            paste_as("dictated", &opts(), us()),
            Err(NOT_CONFIRMED_MSG.to_string())
        );
        assert_eq!(read_text().as_deref(), Some("dictated"), "left for Ctrl+V");
    }

    #[test]
    fn a_listener_read_does_not_confirm_the_paste() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let reader = Reader::start();
        // The target is some other process: our reader is a listener.
        let r = paste_as("dictated", &opts(), vec![u32::MAX]);
        assert!(
            reader.saw("dictated"),
            "a listener's read is still rendered"
        );
        drop(reader);
        assert_eq!(r, Err(NOT_CONFIRMED_MSG.to_string()));
        assert_eq!(read_text().as_deref(), Some("dictated"));
    }

    #[test]
    fn a_copy_made_meanwhile_stays_and_the_message_says_so() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        if !usable() {
            return;
        }
        set_plain("the user's clipboard");
        let copier = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(400));
            set_plain("copied meanwhile");
        });
        let r = paste_as("dictated", &opts(), us());
        copier.join().unwrap();
        assert_eq!(r, Err(NOT_CONFIRMED_CLIPBOARD_TAKEN_MSG.to_string()));
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
            .map(|t| std::thread::spawn(move || paste_as(t, &opts(), us())))
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), Ok(()));
        }
        assert_eq!(
            read_text().as_deref(),
            Some("the user's clipboard"),
            "S15: a paste must not snapshot another paste's text"
        );
    }
}
