//! WH_KEYBOARD_LL hook thread (Windows only).
//!
//! One dedicated thread installs the hook and pumps messages (LL hooks are
//! called on the installing thread's message loop). The callback does only
//! the O(1) matcher step and an `mpsc::send`, staying far below
//! `LowLevelHooksTimeout`. A 1 s `WM_TIMER` probes held keys with
//! `GetAsyncKeyState` to recover from missed key-ups (lock screen, UAC
//! desktop switch, hook timeouts). The same timer runs the liveness
//! [`watchdog`](super::watchdog): Windows silently removes a hook that
//! overran `LowLevelHooksTimeout`, so we probe it and re-install when dead.
//! While Settings records a new shortcut ([`HookHandle::start_capture`]),
//! key events go to a [`ChordRecorder`] instead of the matcher: no hotkey
//! fires, and the recorded keys reach no app.

use super::capture::{ChordRecorder, RecordStep};
use super::watchdog::{self, Watchdog};
use super::{Bindings, ChordMatcher, HotkeyAction, Key};
use crate::input;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetLastInputInfo, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_KEYUP, LASTINPUTINFO, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN, WM_TIMER,
};

/// dwExtraInfo of watchdog probes (distinct from our paste/mask tag); the
/// hook swallows them so no app ever sees the probe key.
const PROBE_TAG: usize = input::EXTRA_INFO_TAG ^ 0x5052_4F42; // "PROB"
/// VK 0xFF: no key is mapped to it.
const PROBE_VK: u16 = 0xFF;

/// Every hook callback bumps this (watchdog liveness signal).
static HOOK_EVENTS: AtomicU64 = AtomicU64::new(0);

struct HookState {
    matcher: ChordMatcher,
    tx: Sender<HotkeyAction>,
    dictating: Arc<AtomicBool>,
    /// Recording a shortcut: the recorder and where its result goes.
    capture: Option<(ChordRecorder, Sender<RecordStep>)>,
}

static STATE: Mutex<Option<HookState>> = Mutex::new(None);

/// Never panics on a poisoned lock: a panic inside the `extern "system"`
/// hook callback would abort the whole process (review C8).
fn state() -> std::sync::MutexGuard<'static, Option<HookState>> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct HookHandle {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl HookHandle {
    pub fn set_bindings(&self, b: Bindings) {
        if let Some(s) = state().as_mut() {
            s.matcher.set_bindings(b);
        }
    }

    /// Record the next chord the user holds and releases instead of
    /// matching hotkeys; the result (done or Esc-cancelled) goes to `tx`.
    /// Replaces a recording already running (its sender is dropped).
    pub fn start_capture(&self, tx: Sender<RecordStep>) {
        if let Some(s) = state().as_mut() {
            // End whatever the matcher holds (a PTT in progress stops).
            let held: Vec<Key> = s.matcher.pressed().collect();
            for k in held {
                for a in s.matcher.force_release(k) {
                    let _ = s.tx.send(a);
                }
            }
            s.matcher.reset();
            s.capture = Some((ChordRecorder::new(), tx));
        }
    }

    /// Back to matching hotkeys (timeout, Settings lost focus, Cancel).
    pub fn stop_capture(&self) {
        if let Some(s) = state().as_mut() {
            s.capture = None;
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
        *state() = None;
    }
}

impl Drop for HookHandle {
    fn drop(&mut self) {
        if self.join.is_some() {
            self.shutdown();
        }
    }
}

/// Install the global keyboard hook. Actions are delivered on `tx`;
/// `dictating` tells the hook whether to swallow the cancel key.
pub fn install(
    bindings: Bindings,
    dictating: Arc<AtomicBool>,
    tx: Sender<HotkeyAction>,
) -> Result<HookHandle, String> {
    *state() = Some(HookState {
        matcher: ChordMatcher::new(bindings),
        tx,
        dictating,
        capture: None,
    });
    let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
    let join = std::thread::Builder::new()
        .name("flow-kbd-hook".into())
        .spawn(move || unsafe { hook_thread(ready_tx) })
        .map_err(|e| e.to_string())?;
    let thread_id = ready_rx.recv().map_err(|e| e.to_string()).and_then(|r| r)?;
    Ok(HookHandle {
        thread_id,
        join: Some(join),
    })
}

unsafe fn hook_thread(ready: Sender<Result<u32, String>>) {
    let hmod = GetModuleHandleW(None)
        .map(|h| HINSTANCE(h.0))
        .unwrap_or_default();
    let mut hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0) {
        Ok(h) => h,
        Err(e) => {
            let _ = ready.send(Err(format!("Failed to create keyboard hook: {e}")));
            return;
        }
    };
    let timer = SetTimer(None, 0, 1000, None);
    let _ = ready.send(Ok(GetCurrentThreadId()));
    let start = std::time::Instant::now();
    let mut dog = Watchdog::new(watchdog::Config::default());
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        if msg.message == WM_TIMER {
            check_stale_keys();
            let now = start.elapsed().as_millis() as u64;
            let events = HOOK_EVENTS.load(Ordering::Relaxed);
            match dog.tick(now, last_input_tick(), events) {
                watchdog::Action::Nothing => {}
                watchdog::Action::Probe => {
                    if send_probe() {
                        dog.probe_sent(now, events);
                    }
                }
                watchdog::Action::Reinstall => {
                    reinstall(&mut hook, hmod);
                    dog.reset(HOOK_EVENTS.load(Ordering::Relaxed));
                }
            }
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    let _ = KillTimer(None, timer);
    let _ = UnhookWindowsHookEx(hook);
}

/// Our own integrity level (constant for the process).
fn own_integrity() -> Option<u32> {
    static OWN: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *OWN.get_or_init(crate::paste::integrity::own)
}

fn last_input_tick() -> u32 {
    let mut li = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        let _ = GetLastInputInfo(&mut li);
    }
    li.dwTime
}

/// Tagged VK 0xFF down+up; the hook swallows it. False = not injected
/// (e.g. a secure desktop is active), so there is nothing to wait for.
fn send_probe() -> bool {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(PROBE_VK),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: PROBE_TAG,
            },
        },
    };
    let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) as usize == inputs.len() }
}

/// The OS dropped our hook: install a fresh one on this thread. Held keys
/// are force-released first so an in-progress PTT ends instead of sticking.
unsafe fn reinstall(hook: &mut HHOOK, hmod: HINSTANCE) {
    eprintln!("[hotkeys] keyboard hook stopped receiving events; re-installing");
    crate::debug_log::event(
        "hotkeys",
        "keyboard hook stopped receiving events; re-installing",
    );
    let _ = UnhookWindowsHookEx(*hook);
    match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0) {
        Ok(h) => *hook = h,
        Err(e) => {
            eprintln!("[hotkeys] re-install failed: {e}");
            crate::debug_log::event("hotkeys", &format!("re-install failed: {e}"));
        }
    }
    let mut guard = state();
    if let Some(s) = guard.as_mut() {
        let held: Vec<Key> = s.matcher.pressed().collect();
        for k in held {
            for a in s.matcher.force_release(k) {
                let _ = s.tx.send(a);
            }
        }
        s.matcher.reset();
    }
}

fn is_physically_down(key: Key) -> bool {
    input::vks_for(key)
        .into_iter()
        .any(|vk| unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 })
}

fn check_stale_keys() {
    let mut guard = state();
    let Some(s) = guard.as_mut() else { return };
    if s.matcher.altgr_ctrl_down() && !is_physically_down(Key::LCtrl) {
        s.matcher.forget_altgr_ctrl();
    }
    let held: Vec<Key> = s.matcher.pressed().collect();
    for k in held {
        if !is_physically_down(k) {
            for a in s.matcher.force_release(k) {
                let _ = s.tx.send(a);
            }
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        HOOK_EVENTS.fetch_add(1, Ordering::Relaxed);
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if kb.dwExtraInfo == PROBE_TAG {
            return LRESULT(1); // watchdog probe: seen, never delivered
        }
        // Skip only *our own* injections (tagged dwExtraInfo, like Wispr's
        // "WFMK"); other injected input (RDP, on-screen keyboard, AHK) is
        // matched normally.
        if kb.dwExtraInfo != input::EXTRA_INFO_TAG {
            let msg = wparam.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let vk = kb.vkCode as u16;
            let decision = {
                let mut guard = state();
                match guard.as_mut() {
                    Some(s) if s.capture.is_some() => {
                        if let Some((rec, out)) = s.capture.as_mut() {
                            let r = rec.handle_raw(vk, kb.scanCode, down);
                            if r.step != RecordStep::Pending {
                                let _ = out.send(r.step);
                                s.capture = None;
                            }
                            if r.suppress {
                                return LRESULT(1);
                            }
                        }
                        None
                    }
                    Some(s) => {
                        // AltGr's fake Left Ctrl (scan 0x21D) is filtered here.
                        let d = s.matcher.handle_raw(
                            vk,
                            kb.scanCode,
                            down,
                            s.dictating.load(Ordering::Relaxed),
                        );
                        for a in &d.actions {
                            let _ = s.tx.send(*a);
                        }
                        // Only the user's own Left Ctrl must stay down through
                        // a masked release; AltGr's fake one is released.
                        let lctrl_held = d.mask_release
                            && s.matcher.real_lctrl_held(
                                (GetAsyncKeyState(input::vk::LCONTROL as i32) as u16) & 0x8000 != 0,
                            );
                        Some((d, lctrl_held))
                    }
                    None => None,
                }
            };
            if let Some((d, lctrl_held)) = decision {
                let mut suppress = d.suppress;
                if d.mask_release {
                    // C11: swallow the physical Win/Alt key-up only if our
                    // masked release really landed; otherwise let it
                    // through so the key is never left logically down.
                    let blocked = crate::paste::uipi_blocks(
                        own_integrity(),
                        crate::paste::integrity::foreground(),
                    );
                    let strokes = input::masked_release(vk, lctrl_held);
                    let sent = if blocked {
                        0
                    } else {
                        input::win::send(&strokes)
                    };
                    if !input::suppress_physical_release(blocked, sent, strokes.len()) {
                        suppress = false;
                        if sent > 0 && !lctrl_held {
                            // A partial batch may have left our LCtrl down.
                            input::win::send(&[input::Stroke::up(input::vk::LCONTROL)]);
                        }
                    }
                }
                if suppress {
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}
