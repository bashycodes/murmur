//! Bringing one of the app's own windows to the foreground when Windows'
//! foreground lock would refuse it.
//!
//! `SetForegroundWindow` only works for the process that has the foreground
//! (or got the last input). A window the app opens on its own, such as the
//! notes window from the new-note shortcut while another app is in front,
//! would otherwise only flash in the taskbar, and typing would go elsewhere.
//! Attaching the calling thread's input to the foreground window's thread
//! for the call lifts that restriction without injecting any keys (tao's
//! `set_focus` fallback taps Alt, which the double-Alt hook would see).

/// Attach to the foreground thread's input only when there is a foreground
/// window and it belongs to another thread (attaching a thread to itself
/// fails; with no foreground window nothing needs lifting).
pub fn attach_needed(foreground_thread: u32, own_thread: u32) -> bool {
    foreground_thread != 0 && foreground_thread != own_thread
}

#[cfg(windows)]
pub mod win {
    use super::attach_needed;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };

    /// Make `hwnd` the foreground window with the keyboard focus (its
    /// webview then gets the focus too). Call it on the thread that owns
    /// `hwnd` (the event loop). True when `hwnd` is in front afterwards.
    pub fn bring_to_front(hwnd: HWND) -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg == hwnd {
                return true;
            }
            let fg_thread = if fg.is_invalid() {
                0
            } else {
                GetWindowThreadProcessId(fg, None)
            };
            let me = GetCurrentThreadId();
            let attach = attach_needed(fg_thread, me);
            let attached = attach && AttachThreadInput(me, fg_thread, true).as_bool();
            let _ = BringWindowToTop(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd);
            if attached {
                let _ = AttachThreadInput(me, fg_thread, false);
            }
            GetForegroundWindow() == hwnd
        }
    }
}

#[cfg(test)]
mod tests {
    use super::attach_needed;

    #[test]
    fn attaches_only_to_another_threads_foreground() {
        assert!(attach_needed(42, 7));
        assert!(!attach_needed(7, 7), "attaching a thread to itself fails");
        assert!(
            !attach_needed(0, 7),
            "no foreground window: nothing to lift"
        );
    }
}
