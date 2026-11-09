//! WASAPI session ducking. COM interface pointers aren't `Send`, so a
//! dedicated MTA thread owns them and takes commands over a channel.

use super::{plan, restore_value, DuckEntry, SessionVol};
use std::sync::mpsc::{self, Sender};
use windows::core::Interface;
use windows::Win32::Media::Audio::{
    eMultimedia, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    ISimpleAudioVolume, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

enum Cmd {
    Duck(f32),
    Restore,
}

pub struct Ducker {
    tx: Sender<Cmd>,
}

impl Ducker {
    pub fn spawn() -> Ducker {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let _ = std::thread::Builder::new()
            .name("murmur-duck".into())
            .spawn(move || unsafe {
                let inited = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
                let mut ducked: Vec<(ISimpleAudioVolume, DuckEntry)> = Vec::new();
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Duck(factor) => {
                            if ducked.is_empty() {
                                ducked = duck(factor).unwrap_or_default();
                            }
                        }
                        Cmd::Restore => {
                            for (vol, e) in ducked.drain(..) {
                                let cur = vol.GetMasterVolume().unwrap_or(e.target);
                                if let Some(v) = restore_value(&e, cur) {
                                    let _ = vol.SetMasterVolume(v, std::ptr::null());
                                }
                            }
                        }
                    }
                }
                if inited {
                    CoUninitialize();
                }
            });
        Ducker { tx }
    }

    /// `factor` 0.0 = mute, e.g. 0.2 = duck to 20 %.
    pub fn duck(&self, factor: f32) {
        let _ = self.tx.send(Cmd::Duck(factor));
    }

    pub fn restore(&self) {
        let _ = self.tx.send(Cmd::Restore);
    }
}

unsafe fn duck(factor: f32) -> windows::core::Result<Vec<(ISimpleAudioVolume, DuckEntry)>> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
    let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None)?;
    let list = mgr.GetSessionEnumerator()?;
    let n = list.GetCount()?;
    let mut vols = Vec::new();
    let mut infos = Vec::new();
    for i in 0..n {
        let Ok(ctl) = list.GetSession(i) else {
            continue;
        };
        let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else {
            continue;
        };
        let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() else {
            continue;
        };
        let pid = ctl2.GetProcessId().unwrap_or(0);
        let volume = vol.GetMasterVolume().unwrap_or(0.0);
        infos.push(SessionVol {
            key: vols.len(),
            pid,
            volume,
        });
        vols.push(vol);
    }
    let mut out = Vec::new();
    for e in plan(&infos, std::process::id(), factor) {
        let vol = vols[e.key].clone();
        if vol.SetMasterVolume(e.target, std::ptr::null()).is_ok() {
            out.push((vol, e));
        }
    }
    Ok(out)
}
