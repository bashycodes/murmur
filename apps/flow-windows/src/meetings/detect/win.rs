//! The Windows probe of meeting detection (D48). One look:
//!
//! 1. the process list (`CreateToolhelp32Snapshot`, ~1 ms) — and if no
//!    known client runs, that's all;
//! 2. the visible top-level windows of the clients' processes (class
//!    only: titles can name a private meeting and aren't needed);
//! 3. the WASAPI audio sessions of the clients on every active capture
//!    and render endpoint (`IAudioSessionManager2`), with their state.

use super::{known_exes, AudioSession, Flow, Probe, Snapshot, Window};
use std::collections::HashMap;
use windows::core::Interface;
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
use windows::Win32::Media::Audio::{
    eAll, eCapture, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
};

pub struct WinProbe;

impl Probe for WinProbe {
    fn snapshot(&mut self) -> Snapshot {
        // pid → exe of the known clients only.
        let all = processes();
        let known: Vec<&str> = known_exes().collect();
        let pids: HashMap<u32, String> = all
            .iter()
            .filter(|(_, e)| known.contains(&e.as_str()))
            .cloned()
            .collect();
        let processes: Vec<String> = all.into_iter().map(|(_, e)| e).collect();
        let mut snap = Snapshot {
            processes,
            ..Snapshot::default()
        };
        if !super::any_client(&snap.processes) {
            return snap;
        }
        snap.windows = windows_of(&pids);
        snap.audio = unsafe { sessions_of(&pids) };
        snap
    }
}

fn utf16(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// (pid, lowercase exe file name) of every process.
fn processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(h) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(h, &mut e).is_ok();
        while ok {
            out.push((e.th32ProcessID, utf16(&e.szExeFile).to_ascii_lowercase()));
            ok = Process32NextW(h, &mut e).is_ok();
        }
        let _ = CloseHandle(h);
    }
    out
}

struct Collect<'a> {
    pids: &'a HashMap<u32, String>,
    out: Vec<Window>,
}

unsafe extern "system" fn on_window(hwnd: HWND, lp: LPARAM) -> BOOL {
    let c = &mut *(lp.0 as *mut Collect);
    if !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if let Some(exe) = c.pids.get(&pid) {
        let mut buf = [0u16; 256];
        let n = GetClassNameW(hwnd, &mut buf).max(0) as usize;
        c.out.push(Window {
            exe: exe.clone(),
            class: String::from_utf16_lossy(&buf[..n]),
            title: String::new(),
        });
    }
    true.into()
}

fn windows_of(pids: &HashMap<u32, String>) -> Vec<Window> {
    let mut c = Collect {
        pids,
        out: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(on_window), LPARAM(&mut c as *mut Collect as isize));
    }
    c.out
}

unsafe fn sessions_of(pids: &HashMap<u32, String>) -> Vec<AudioSession> {
    let inited = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
    let out = sessions(pids).unwrap_or_default();
    if inited {
        CoUninitialize();
    }
    out
}

unsafe fn sessions(pids: &HashMap<u32, String>) -> windows::core::Result<Vec<AudioSession>> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let devices = en.EnumAudioEndpoints(eAll, DEVICE_STATE_ACTIVE)?;
    let mut out = Vec::new();
    for i in 0..devices.GetCount()? {
        let Ok(dev) = devices.Item(i) else { continue };
        let flow = match dev.cast::<IMMEndpoint>().and_then(|e| e.GetDataFlow()) {
            Ok(f) if f == eCapture => Flow::Capture,
            Ok(_) => Flow::Render,
            Err(_) => continue,
        };
        let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
            continue;
        };
        let Ok(list) = mgr.GetSessionEnumerator() else {
            continue;
        };
        for j in 0..list.GetCount().unwrap_or(0) {
            let Ok(ctl) = list.GetSession(j) else {
                continue;
            };
            let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else {
                continue;
            };
            let pid = ctl2.GetProcessId().unwrap_or(0);
            let Some(exe) = pids.get(&pid) else { continue };
            out.push(AudioSession {
                exe: exe.clone(),
                flow,
                active: ctl.GetState().is_ok_and(|s| s == AudioSessionStateActive),
            });
        }
    }
    Ok(out)
}
