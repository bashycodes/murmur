//! Which models suit this machine (D117): one machine-class table, used by
//! the Windows setup and Settings and, through a Kotlin twin checked by
//! `fixtures/model-fit/vectors.json`, by the Android model pages.
//!
//! * [`recommend`]: total RAM and the GPU probes pick the speech, AI cleanup
//!   and Hebrew model.
//! * [`fit`]: whether one model fits (free disk for its download, total RAM,
//!   a graphics chip for the GPU-only ones), with the reasons it does not.
//!
//! Every number cites a finding (D117 lists them); where the evidence is
//! thin the comment says so. CPU class is not an input: the two CPUs
//! measured (i7-6700, Core Ultra 9 288V) both run the CPU picks inside the
//! budget, and nothing measured tells a slow CPU apart cheaply.

/// A model as the table knows it. Each platform maps its own catalogue
/// ids or file names onto these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// Parakeet 0.6B int8 English (TDT v2 or unified).
    Parakeet,
    WhisperTiny,
    WhisperBase,
    WhisperSmall,
    /// Stock whisper large-v3-turbo.
    WhisperTurbo,
    ZipformerHe,
    ParakeetHe,
    /// ivrit.ai whisper large-v3-turbo (Hebrew).
    IvritTurbo,
    Qwen05,
    Qwen15,
    Llama3b,
}

impl Model {
    pub const ALL: [Model; 11] = [
        Model::Parakeet,
        Model::WhisperTiny,
        Model::WhisperBase,
        Model::WhisperSmall,
        Model::WhisperTurbo,
        Model::ZipformerHe,
        Model::ParakeetHe,
        Model::IvritTurbo,
        Model::Qwen05,
        Model::Qwen15,
        Model::Llama3b,
    ];

    /// The key the shared vectors and the Kotlin twin use.
    pub fn key(self) -> &'static str {
        match self {
            Model::Parakeet => "parakeet",
            Model::WhisperTiny => "whisper-tiny",
            Model::WhisperBase => "whisper-base",
            Model::WhisperSmall => "whisper-small",
            Model::WhisperTurbo => "whisper-turbo",
            Model::ZipformerHe => "zipformer-he",
            Model::ParakeetHe => "parakeet-he",
            Model::IvritTurbo => "ivrit-turbo",
            Model::Qwen05 => "qwen2.5-0.5b",
            Model::Qwen15 => "qwen2.5-1.5b",
            Model::Llama3b => "llama-3.2-3b",
        }
    }

    pub fn from_key(key: &str) -> Option<Model> {
        Model::ALL.into_iter().find(|m| m.key() == key)
    }

    /// What the model needs besides disk space.
    pub fn need(self) -> Need {
        match self {
            // D4: Parakeet with >= 4 GB RAM, else whisper base. Measured
            // 769 MB (English) / 797 MB (Hebrew) PSS on the phone
            // (2026-10-31-phone-on-device-bench).
            Model::Parakeet | Model::ParakeetHe => Need {
                ram_gb: 4,
                gpu: Gpu::No,
            },
            // D11: Qwen2.5-1.5B on Android >= 8 GB RAM; it holds 1.9 GB RSS
            // (2026-10-26-ai-cleanup-benchmark) / 2.1 GB PSS on the phone,
            // next to the speech model.
            Model::Qwen15 => Need {
                ram_gb: 8,
                gpu: Gpu::No,
            },
            // A 3B Q4_K_M holds 3.6 GB RSS on the CPU (2026-10-26) and 2.8 GB
            // of GPU memory on the laptop's iGPU, which shares system RAM
            // (2026-10-31-laptop-model-setup). Only a 32 GB laptop was
            // measured: 8 GB is the 1.5B's bar, not a measured minimum.
            // Too slow on a CPU (2026-10-30-laptop-model-benchmark).
            Model::Llama3b => Need {
                ram_gb: 8,
                gpu: Gpu::Llm,
            },
            // 14.5-25 s per clip on a CPU (model-benchmarks,
            // 2026-10-30-laptop-model-benchmark); about 1 s on an iGPU.
            Model::WhisperTurbo | Model::IvritTurbo => Need {
                ram_gb: 0,
                gpu: Gpu::Whisper,
            },
            // Small models: 219 MB (whisper base) / 311 MB (zipformer) PSS
            // on the phone; tiny, small and the 0.5B were not measured for
            // RAM, so they carry no RAM bar.
            Model::WhisperTiny
            | Model::WhisperBase
            | Model::WhisperSmall
            | Model::ZipformerHe
            | Model::Qwen05 => Need {
                ram_gb: 0,
                gpu: Gpu::No,
            },
        }
    }
}

/// Which GPU probe a model needs to say yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gpu {
    No,
    /// AI cleanup on a GPU (Windows `llm_backend`).
    Llm,
    /// whisper on a GPU (Windows `whisper_backend`).
    Whisper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Need {
    /// Nominal RAM in GB (0 = no bar); see [`has_ram`].
    pub ram_gb: u32,
    pub gpu: Gpu,
}

/// What the app knows about the machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Machine {
    /// Total physical RAM the OS reports; 0 = unknown.
    pub total_ram: u64,
    /// Free bytes where models are saved; None = unknown.
    pub free_disk: Option<u64>,
    /// Local AI cleanup would run on a GPU.
    pub gpu_llm: bool,
    /// whisper would run on a GPU.
    pub gpu_whisper: bool,
}

const GIB: u64 = 1 << 30;

/// The OS reports less than the nominal RAM (firmware, the integrated GPU
/// and the kernel keep part of it: an "8 GB" phone shows about 7.3 GiB).
/// A machine has N GB when it reports at least 85 % of N GiB. Unknown RAM
/// (0) passes every bar: the app never blocks a pick on a failed probe.
pub fn has_ram(total_ram: u64, gb: u32) -> bool {
    total_ram == 0 || gb == 0 || total_ram * 100 >= u64::from(gb) * GIB * 85
}

/// The models the table picks for a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picks {
    pub speech: Model,
    /// None: no local AI cleanup (the rules only, or a server / cloud).
    pub cleanup: Option<Model>,
    pub hebrew: Model,
}

/// The machine-class table (D117).
///
/// | RAM | GPU | speech | AI cleanup | Hebrew |
/// |---|---|---|---|---|
/// | >= 8 GB | cleanup GPU | Parakeet | Llama 3.2 3B | |
/// | >= 8 GB | none | Parakeet | Qwen2.5 1.5B | |
/// | 4-8 GB | any | Parakeet | none | |
/// | 2-4 GB | any | whisper base | none | |
/// | < 2 GB | any | whisper tiny | none | |
/// | any | whisper GPU | | | ivrit turbo |
/// | any | no whisper GPU | | | zipformer |
pub fn recommend(m: &Machine) -> Picks {
    let ram = |gb| has_ram(m.total_ram, gb);
    let speech = if ram(4) {
        Model::Parakeet
    } else if ram(2) {
        // D4: whisper base below 4 GB. 2 GB is Android's low-RAM (Go)
        // class, where D4 names tiny; no phone that small was measured.
        Model::WhisperBase
    } else {
        Model::WhisperTiny
    };
    let cleanup = if !ram(8) {
        None
    } else if m.gpu_llm {
        Some(Model::Llama3b)
    } else {
        Some(Model::Qwen15)
    };
    let hebrew = if m.gpu_whisper {
        Model::IvritTurbo
    } else {
        Model::ZipformerHe
    };
    Picks {
        speech,
        cleanup,
        hebrew,
    }
}

/// Room kept free next to a download (the Android import checks keep
/// 50-100 MB too).
pub const DISK_MARGIN: u64 = 100_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fit {
    pub fits: bool,
    /// Why it does not fit, e.g. "needs 0.8 GB free", "needs 8 GB RAM".
    pub reasons: Vec<String>,
}

/// Whether `model` (a download of `size` bytes, or already `installed`)
/// fits the machine. An unknown probe never fails a model.
pub fn fit(model: Model, size: u64, installed: bool, m: &Machine) -> Fit {
    let need = model.need();
    let mut reasons = Vec::new();
    if !installed {
        if let Some(free) = m.free_disk {
            let want = size + DISK_MARGIN;
            if free < want {
                reasons.push(format!("needs {} free", bytes_up(want)));
            }
        }
    }
    if !has_ram(m.total_ram, need.ram_gb) {
        reasons.push(format!("needs {} GB RAM", need.ram_gb));
    }
    let gpu_ok = match need.gpu {
        Gpu::No => true,
        Gpu::Llm => m.gpu_llm,
        Gpu::Whisper => m.gpu_whisper,
    };
    if !gpu_ok {
        reasons.push("needs a graphics chip".into());
    }
    Fit {
        fits: reasons.is_empty(),
        reasons,
    }
}

/// Bytes rounded up to the next tenth of a GB from 1 GB: "2.2 GB" from
/// 2_119_377_696 (decimal units, as the apps show sizes),
/// else to the next MB: "762 MB" from 761_200_000.
pub fn bytes_up(b: u64) -> String {
    if b >= 1_000_000_000 {
        let tenths = b.div_ceil(100_000_000);
        format!("{}.{} GB", tenths / 10, tenths % 10)
    } else {
        format!("{} MB", b.div_ceil(1_000_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gb(n: f64) -> u64 {
        (n * GIB as f64) as u64
    }

    #[test]
    fn reported_ram_meets_its_nominal_band() {
        // An 8 GB phone reports about 7.3 GiB; a 6 GB one about 5.5 GiB.
        assert!(has_ram(gb(7.3), 8));
        assert!(!has_ram(gb(5.5), 8));
        assert!(has_ram(gb(3.6), 4));
        assert!(!has_ram(gb(2.8), 4));
        assert!(has_ram(0, 8), "unknown RAM never blocks");
    }

    #[test]
    fn table_rows() {
        let m = |ram, gl, gw| Machine {
            total_ram: gb(ram),
            free_disk: None,
            gpu_llm: gl,
            gpu_whisper: gw,
        };
        let p = recommend(&m(32.0, true, true));
        assert_eq!(
            (p.speech, p.cleanup, p.hebrew),
            (Model::Parakeet, Some(Model::Llama3b), Model::IvritTurbo)
        );
        let p = recommend(&m(16.0, false, false));
        assert_eq!(
            (p.speech, p.cleanup, p.hebrew),
            (Model::Parakeet, Some(Model::Qwen15), Model::ZipformerHe)
        );
        assert_eq!(recommend(&m(6.0, true, false)).cleanup, None);
        assert_eq!(recommend(&m(3.0, false, false)).speech, Model::WhisperBase);
        assert_eq!(recommend(&m(1.5, false, false)).speech, Model::WhisperTiny);
    }

    #[test]
    fn every_pick_fits_the_machine_it_was_picked_for() {
        for ram in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 16.0] {
            for (gl, gw) in [(false, false), (true, false), (false, true), (true, true)] {
                let m = Machine {
                    total_ram: gb(ram),
                    free_disk: None,
                    gpu_llm: gl,
                    gpu_whisper: gw,
                };
                let p = recommend(&m);
                for model in [Some(p.speech), p.cleanup, Some(p.hebrew)]
                    .into_iter()
                    .flatten()
                {
                    assert!(fit(model, 0, false, &m).fits, "{model:?} on {m:?}");
                }
            }
        }
    }

    #[test]
    fn reasons() {
        let m = Machine {
            total_ram: gb(5.5),
            free_disk: Some(500_000_000),
            gpu_llm: false,
            gpu_whisper: false,
        };
        let f = fit(Model::Llama3b, 2_019_000_000, false, &m);
        assert!(!f.fits);
        assert_eq!(
            f.reasons,
            [
                "needs 2.2 GB free",
                "needs 8 GB RAM",
                "needs a graphics chip"
            ]
        );
        // Installed: the disk no longer matters.
        assert_eq!(
            fit(Model::Qwen15, 1_120_000_000, true, &m).reasons,
            ["needs 8 GB RAM"]
        );
        assert!(fit(Model::WhisperBase, 60_000_000, false, &m).fits);
        assert_eq!(bytes_up(161_000_001), "162 MB");
    }

    #[test]
    fn keys_round_trip() {
        for m in Model::ALL {
            assert_eq!(Model::from_key(m.key()), Some(m));
        }
        assert_eq!(Model::from_key("nope"), None);
    }
}
