//! UI Automation snapshot of the focused field (Windows only).
//!
//! Order of attempts: TextPattern (caret-aware
//! before/selected/after) → ValuePattern (whole value treated as "before").
//! Runs on its own MTA thread with a timeout so a hung target app can never
//! stall dictation start.

use super::RawSnapshot;
use std::sync::mpsc;
use std::time::Duration;
use windows::core::{BSTR, PWSTR, VARIANT};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    IUIAutomationValuePattern, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
    TreeScope_Descendants, UIA_ControlTypePropertyId, UIA_EditControlTypeId, UIA_TextPatternId,
    UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

const BROWSERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "vivaldi.exe",
    "opera.exe",
    "arc.exe",
];

/// Snapshot the foreground app + focused field; `None` on timeout.
pub fn snapshot(timeout: Duration, use_text: bool) -> Option<RawSnapshot> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("flow-uia".into())
        .spawn(move || {
            let _ = tx.send(unsafe { snapshot_blocking(use_text) });
        })
        .ok()?;
    rx.recv_timeout(timeout).ok()
}

/// Foreground window → (exe path, title). Cheap; no COM.
pub fn foreground_app() -> (HWND, String, String) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title).max(0) as usize;
        let title = String::from_utf16_lossy(&title[..n]);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut exe = String::new();
        if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                .is_ok()
            {
                exe = String::from_utf16_lossy(&buf[..len as usize]);
            }
            let _ = CloseHandle(h);
        }
        (hwnd, exe, title)
    }
}

unsafe fn snapshot_blocking(use_text: bool) -> RawSnapshot {
    let (hwnd, exe_path, window_title) = foreground_app();
    let mut snap = RawSnapshot {
        exe_path,
        window_title,
        ..Default::default()
    };
    let inited = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
    if let Ok(uia) =
        CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
    {
        if let Ok(el) = uia.GetFocusedElement() {
            snap.is_password = el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false);
            if use_text && !snap.is_password {
                read_text(&el, &mut snap);
            }
        }
        let exe = super::exe_name(&snap.exe_path);
        if BROWSERS.contains(&exe.as_str()) {
            snap.url = browser_url(&uia, hwnd);
        }
    }
    if inited {
        CoUninitialize();
    }
    snap
}

fn bstr(b: windows::core::Result<BSTR>) -> String {
    b.map(|s| s.to_string()).unwrap_or_default()
}

unsafe fn read_text(el: &IUIAutomationElement, snap: &mut RawSnapshot) {
    if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
        if let (Ok(sel), Ok(doc)) = (tp.GetSelection(), tp.DocumentRange()) {
            if sel.Length().unwrap_or(0) > 0 {
                if let Ok(r) = sel.GetElement(0) {
                    snap.selected = bstr(r.GetText(super::MAX_FIELD_CHARS as i32));
                    if let Ok(b) = doc.Clone() {
                        if b.MoveEndpointByRange(
                            TextPatternRangeEndpoint_End,
                            &r,
                            TextPatternRangeEndpoint_Start,
                        )
                        .is_ok()
                        {
                            // Can't bound from the end; trimmed by build_field_context.
                            snap.before = bstr(b.GetText(-1));
                        }
                    }
                    if let Ok(a) = doc.Clone() {
                        if a.MoveEndpointByRange(
                            TextPatternRangeEndpoint_Start,
                            &r,
                            TextPatternRangeEndpoint_End,
                        )
                        .is_ok()
                        {
                            snap.after = bstr(a.GetText(super::MAX_FIELD_CHARS as i32));
                        }
                    }
                    return;
                }
            }
        }
    }
    if let Ok(vp) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
        // No caret information: assume the caret is at the end.
        snap.before = bstr(vp.CurrentValue());
    }
}

/// First Edit control in the browser window whose value looks like a URL
/// (Chrome/Edge "Address and search bar", Firefox urlbar).
unsafe fn browser_url(uia: &IUIAutomation, hwnd: HWND) -> Option<String> {
    let root = uia.ElementFromHandle(hwnd).ok()?;
    let cond = uia
        .CreatePropertyCondition(
            UIA_ControlTypePropertyId,
            &VARIANT::from(UIA_EditControlTypeId.0),
        )
        .ok()?;
    let edit = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
    let vp = edit
        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        .ok()?;
    let v = bstr(vp.CurrentValue());
    let looks_like_url = !v.contains(' ') && v.contains('.');
    looks_like_url.then_some(v)
}

// ------------------------------------------------------------ learn-from-edits

/// Longest field text read back while watching (characters).
const WATCH_MAX_CHARS: i32 = 20_000;

/// Re-reads the control that had focus when it was created
/// ([`crate::learn::FieldReader`]). Lives on one MTA thread: create, use
/// and drop it there.
pub struct UiaFieldReader {
    // `Option` so Drop can release the COM objects before CoUninitialize.
    com: Option<(IUIAutomation, IUIAutomationElement)>,
    inited: bool,
}

/// Bind to the focused control; `None` for password fields and controls
/// exposing neither TextPattern nor ValuePattern.
pub fn field_reader() -> Option<UiaFieldReader> {
    unsafe {
        let inited = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let mut r = UiaFieldReader { com: None, inited };
        let uia: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        let el = uia.GetFocusedElement().ok()?;
        if el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(true) {
            return None;
        }
        read_whole(&el)?;
        r.com = Some((uia, el));
        Some(r)
    }
}

/// Whole text of `el` (TextPattern document, else ValuePattern value).
unsafe fn read_whole(el: &IUIAutomationElement) -> Option<String> {
    if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
        if let Ok(doc) = tp.DocumentRange() {
            if let Ok(t) = doc.GetText(WATCH_MAX_CHARS) {
                return Some(t.to_string());
            }
        }
    }
    let vp = el
        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        .ok()?;
    vp.CurrentValue().ok().map(|v| v.to_string())
}

impl crate::learn::FieldReader for UiaFieldReader {
    fn read(&mut self) -> crate::learn::Read {
        use crate::learn::Read;
        let Some((uia, el)) = &self.com else {
            return Read::FocusChanged;
        };
        unsafe {
            match uia.GetFocusedElement() {
                Ok(now) => {
                    let same = uia.CompareElements(el, &now).map(|b| b.as_bool());
                    if !same.unwrap_or(false) {
                        return Read::FocusChanged;
                    }
                }
                Err(_) => return Read::Unavailable,
            }
            match read_whole(el) {
                Some(t) => Read::Text(t),
                None => Read::Unavailable,
            }
        }
    }
}

impl Drop for UiaFieldReader {
    fn drop(&mut self) {
        self.com = None;
        if self.inited {
            unsafe { CoUninitialize() };
        }
    }
}
