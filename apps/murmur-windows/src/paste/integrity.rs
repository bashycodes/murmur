//! Process integrity levels for the UIPI pre-check (Windows only).

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

unsafe fn token_rid(process: HANDLE) -> Option<u32> {
    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
    let mut buf = vec![0u8; 256];
    let mut len = 0u32;
    let ok = GetTokenInformation(
        token,
        TokenIntegrityLevel,
        Some(buf.as_mut_ptr() as *mut _),
        buf.len() as u32,
        &mut len,
    )
    .is_ok();
    let _ = CloseHandle(token);
    if !ok {
        return None;
    }
    let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
    let sid = label.Label.Sid;
    let count = *GetSidSubAuthorityCount(sid);
    if count == 0 {
        return None;
    }
    Some(*GetSidSubAuthority(sid, (count - 1) as u32))
}

/// Our own integrity RID.
pub fn own() -> Option<u32> {
    unsafe { token_rid(GetCurrentProcess()) }
}

/// Integrity RID of the process owning `hwnd`. `None` when it can't be read:
/// opening an elevated process' token from medium IL is normally allowed
/// with PROCESS_QUERY_LIMITED_INFORMATION, but protected processes refuse.
pub fn of_window(hwnd: HWND) -> Option<u32> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let r = token_rid(h);
        let _ = CloseHandle(h);
        r
    }
}

pub fn foreground() -> Option<u32> {
    of_window(unsafe { GetForegroundWindow() })
}
