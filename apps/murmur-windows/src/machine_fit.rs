//! This PC for the machine-class table (D116, `murmur_core::model_fit`):
//! total and available RAM, free disk where models are saved, and the GPU
//! probes Settings already runs (`llm_backend`, `whisper_backend`). The
//! `machine_fit` command returns the picks as catalogue ids plus a fits /
//! doesn't-fit line for every catalogue model; setup steps 3 and 4 and
//! Settings' model rows read it, so they never disagree.

use crate::model_download::{catalog_id, table_model, CATALOG};
use murmur_core::model_fit::{self, Machine};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// One catalogue model on this PC.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FitView {
    pub fits: bool,
    /// Why not: "needs 2.2 GB free", "needs 8 GB RAM", "needs a graphics chip".
    pub reasons: Vec<String>,
}

/// The `machine_fit` answer.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MachineView {
    /// Bytes; 0 = not known.
    pub total_ram: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_ram: Option<u64>,
    /// Free bytes on the models folder's drive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_disk: Option<u64>,
    /// Local AI cleanup would run on a GPU (`llm_backend`).
    pub gpu: bool,
    /// The GPU's name when `gpu`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub gpu_device: String,
    /// whisper would run on a GPU (`whisper_backend`).
    pub whisper_gpu: bool,
    /// Catalogue ids the table picks.
    pub speech: &'static str,
    /// None: no local AI cleanup recommended (below 8 GB RAM).
    pub cleanup: Option<&'static str>,
    pub hebrew: &'static str,
    /// Every catalogue model, by id.
    pub fit: BTreeMap<&'static str, FitView>,
}

/// What the probes found (each may be unknown).
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub total_ram: u64,
    pub available_ram: Option<u64>,
    pub free_disk: Option<u64>,
    pub gpu: bool,
    pub gpu_device: String,
    pub whisper_gpu: bool,
}

/// The table's answer for this PC; `installed` says which catalogue models
/// are already in the models folder (their download needs no disk).
pub fn view(p: &Probe, installed: impl Fn(&str) -> bool) -> MachineView {
    let m = Machine {
        total_ram: p.total_ram,
        free_disk: p.free_disk,
        gpu_llm: p.gpu,
        gpu_whisper: p.whisper_gpu,
    };
    let picks = model_fit::recommend(&m);
    let fit = CATALOG
        .iter()
        .filter_map(|e| {
            let model = table_model(e.id)?;
            let f = model_fit::fit(model, e.size(), installed(e.id), &m);
            Some((
                e.id,
                FitView {
                    fits: f.fits,
                    reasons: f.reasons,
                },
            ))
        })
        .collect();
    MachineView {
        total_ram: p.total_ram,
        available_ram: p.available_ram,
        free_disk: p.free_disk,
        gpu: p.gpu,
        gpu_device: if p.gpu {
            p.gpu_device.clone()
        } else {
            String::new()
        },
        whisper_gpu: p.whisper_gpu,
        speech: catalog_id(picks.speech),
        cleanup: picks.cleanup.map(catalog_id),
        hebrew: catalog_id(picks.hebrew),
        fit,
    }
}

/// Total and available physical RAM (0 / None when the OS won't say).
pub fn ram() -> (u64, Option<u64>) {
    #[cfg(windows)]
    {
        win::ram()
    }
    #[cfg(not(windows))]
    {
        // Development builds on Linux: /proc/meminfo.
        let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let kb = |key: &str| -> Option<u64> {
            text.lines()
                .find_map(|l| l.strip_prefix(key))
                .and_then(|r| r.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
                .map(|n| n * 1024)
        };
        (kb("MemTotal:").unwrap_or(0), kb("MemAvailable:"))
    }
}

/// Free bytes for this user on the drive of `dir` (its nearest existing
/// parent: the models folder may not exist yet). None when unknown.
pub fn free_disk(dir: &Path) -> Option<u64> {
    let existing = dir.ancestors().find(|p| p.exists())?;
    #[cfg(windows)]
    {
        win::free_disk(existing)
    }
    #[cfg(not(windows))]
    {
        let _ = existing;
        None
    }
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    pub fn ram() -> (u64, Option<u64>) {
        let mut s = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        // SAFETY: `s` is a valid MEMORYSTATUSEX with dwLength set, as the
        // API requires; it is only written during the call.
        match unsafe { GlobalMemoryStatusEx(&mut s) } {
            Ok(()) => (s.ullTotalPhys, Some(s.ullAvailPhys)),
            Err(_) => (0, None),
        }
    }

    pub fn free_disk(dir: &Path) -> Option<u64> {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // SAFETY: `wide` is NUL-terminated and outlives the call; `free` is
        // a valid out pointer; the other outputs are not requested.
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide.as_ptr()),
                Some(&mut free as *mut u64),
                None,
                None,
            )
        }
        .ok()
        .map(|()| free)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn a_big_pc_with_a_gpu_gets_the_gpu_picks() {
        let p = Probe {
            total_ram: 31 * GIB,
            free_disk: Some(100_000_000_000),
            gpu: true,
            gpu_device: "Test iGPU".into(),
            whisper_gpu: true,
            ..Probe::default()
        };
        let v = view(&p, |_| false);
        assert_eq!(v.speech, "parakeet-tdt-0.6b-v2-int8");
        assert_eq!(v.cleanup, Some(crate::model_download::LLAMA32_3B_ID));
        assert_eq!(v.hebrew, crate::model_download::IVRIT_TURBO_ID);
        assert!(v.fit.values().all(|f| f.fits));
        assert_eq!(v.fit.len(), CATALOG.len());
        assert_eq!(v.gpu_device, "Test iGPU");
    }

    #[test]
    fn a_small_pc_without_room_says_why() {
        let p = Probe {
            total_ram: 6 * GIB,
            free_disk: Some(400_000_000),
            gpu_device: "ignored without a GPU".into(),
            ..Probe::default()
        };
        let v = view(&p, |id| id == "qwen2.5-1.5b-instruct-q4_k_m");
        assert_eq!(v.speech, "parakeet-tdt-0.6b-v2-int8");
        assert_eq!(v.cleanup, None);
        assert_eq!(v.hebrew, crate::model_download::ZIPFORMER_HE_ID);
        assert_eq!(v.gpu_device, "");
        let why = |id: &str| v.fit[id].reasons.join(", ");
        assert_eq!(why("parakeet-tdt-0.6b-v2-int8"), "needs 762 MB free");
        // Installed: only the RAM bar is left.
        assert_eq!(why("qwen2.5-1.5b-instruct-q4_k_m"), "needs 8 GB RAM");
        assert_eq!(
            why(crate::model_download::LLAMA32_3B_ID),
            "needs 2.2 GB free, needs 8 GB RAM, needs a graphics chip"
        );
        assert!(v.fit["whisper-base"].fits);
    }

    #[test]
    fn unknown_probes_never_fail_a_model() {
        let v = view(&Probe::default(), |_| false);
        assert!(v
            .fit
            .values()
            .filter(|f| !f.fits)
            .all(|f| f.reasons == ["needs a graphics chip"]));
        assert_eq!(v.cleanup, Some("qwen2.5-1.5b-instruct-q4_k_m"));
    }

    #[test]
    fn this_machine_reports_some_ram() {
        let (total, avail) = ram();
        if cfg!(target_os = "linux") {
            assert!(total > 0);
            assert!(avail.is_some_and(|a| a <= total));
        }
        if !cfg!(windows) {
            assert_eq!(free_disk(Path::new("/definitely/not/here/x")), None);
        }
    }
}
