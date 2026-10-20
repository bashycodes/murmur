//! WH_KEYBOARD_LL hook thread (Windows only).
//!
//! One dedicated thread installs the hook and pumps messages (LL hooks are
//! called on the installing thread's message loop). The callback does only
//! the O(1) matcher step and an `mpsc::send`, staying far below
//! `LowLevelHooksTimeout`. A 1 s `WM_TIMER` probes held keys with
//! `GetAsyncKeyState` to recover from missed key-ups (lock screen, UAC
//! desktop switch, hook timeouts).

use super::{Bindings, ChordMatcher, HotkeyAction, Key, KeyEvent};
use crate::input;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN, WM_TIMER,
};

struct HookState {
    matcher: ChordMatcher,
    tx: Sender<HotkeyAction>,
    dictating: Arc<AtomicBool>,
}

static STATE: Mutex<Option<HookState>> = Mutex::new(None);

pub struct HookHandle {
    thread_id: u32,
    join: Option<JoinHandle<()>>,
}

impl HookHandle {
    pub fn set_bindings(&self, b: Bindings) {
        if let Some(s) = STATE.lock().unwrap().as_mut() {
            s.matcher.set_bindings(b);
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
        *STATE.lock().unwrap() = None;
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
    *STATE.lock().unwrap() = Some(HookState {
        matcher: ChordMatcher::new(bindings),
        tx,
        dictating,
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
    let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0) {
        Ok(h) => h,
        Err(e) => {
            let _ = ready.send(Err(format!("Failed to create keyboard hook: {e}")));
            return;
        }
    };
    let timer = SetTimer(None, 0, 1000, None);
    let _ = ready.send(Ok(GetCurrentThreadId()));
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        if msg.message == WM_TIMER {
            check_stale_keys();
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    let _ = KillTimer(None, timer);
    let _ = UnhookWindowsHookEx(hook);
}

fn is_physically_down(key: Key) -> bool {
    input::vks_for(key)
        .into_iter()
        .any(|vk| unsafe { (GetAsyncKeyState(vk as i32) as u16) & 0x8000 != 0 })
}

fn check_stale_keys() {
    let mut guard = STATE.lock().unwrap();
    let Some(s) = guard.as_mut() else { return };
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
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // Skip only *our own* injections (tagged dwExtraInfo, like Wispr's
        // "WFMK"); other injected input (RDP, on-screen keyboard, AHK) is
        // matched normally.
        if kb.dwExtraInfo != input::EXTRA_INFO_TAG {
            let msg = wparam.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let ev = KeyEvent {
                key: Key::from_vk(kb.vkCode as u16),
                down,
            };
            let decision = {
                let mut guard = STATE.lock().unwrap();
                match guard.as_mut() {
                    Some(s) => {
                        let d = s.matcher.handle(ev, s.dictating.load(Ordering::Relaxed));
                        for a in &d.actions {
                            let _ = s.tx.send(*a);
                        }
                        Some(d)
                    }
                    None => None,
                }
            };
            if let Some(d) = decision {
                if d.mask_release {
                    let lctrl_held =
                        (GetAsyncKeyState(input::vk::LCONTROL as i32) as u16) & 0x8000 != 0;
                    input::win::send(&input::masked_release(kb.vkCode as u16, lctrl_held));
                }
                if d.suppress {
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}
