//! Which ggml device runs the whisper model: a GPU (Vulkan, in the Windows
//! `flow_whisper_vulkan.dll` build) or the CPU. Same policy as the cleanup
//! engine (flow-llm `backend`).
//!
//! [`choose`] is pure, so the policy is unit-tested without whisper.cpp:
//! * `FLOW_WHISPER_DEVICE=cpu` forces the CPU (A/B runs, a misbehaving
//!   driver);
//! * otherwise the best real GPU: a discrete GPU before an integrated one,
//!   then the one with the most free memory. Software rasterisers
//!   (llvmpipe, SwiftShader, Microsoft Basic Render) are not GPUs;
//! * no GPU device (a CPU build, or Vulkan found no device) = the CPU.
//!
//! The engine loads on the chosen GPU and, when that fails, again on the
//! CPU ([`Plan::fallbacks`]).

use std::ffi::CStr;

/// ggml device type, mirrored so tests need no whisper.cpp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Gpu,
    IntegratedGpu,
    Accelerator,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Device {
    /// whisper.cpp's `gpu_device` for a GPU: its index among the GPU
    /// devices (discrete and integrated) in ggml's order. `None` otherwise.
    pub gpu_index: Option<i32>,
    /// e.g. "Vulkan0".
    pub name: String,
    /// e.g. "Intel(R) Arc(TM) 140V GPU (16GB)".
    pub description: String,
    /// ggml backend: "Vulkan", "CPU", …
    pub backend: String,
    pub kind: DeviceKind,
    pub memory_free: u64,
    pub memory_total: u64,
}

/// Where the model goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Gpu {
        gpu_index: i32,
        backend: String,
        description: String,
    },
    Cpu,
}

impl Plan {
    /// The plans to try in order: the GPU, then the CPU.
    pub fn fallbacks(self) -> Vec<Plan> {
        match self {
            Plan::Gpu { .. } => vec![self, Plan::Cpu],
            Plan::Cpu => vec![Plan::Cpu],
        }
    }

    /// "Vulkan" / "CPU".
    pub fn backend(&self) -> &str {
        match self {
            Plan::Gpu { backend, .. } => backend,
            Plan::Cpu => "CPU",
        }
    }

    /// The GPU's name, or "CPU".
    pub fn device(&self) -> &str {
        match self {
            Plan::Gpu { description, .. } => description,
            Plan::Cpu => "CPU",
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(self, Plan::Gpu { .. })
    }
}

/// Device names that are CPU emulations of a GPU API.
const SOFTWARE_GPUS: &[&str] = &["llvmpipe", "lavapipe", "swiftshader", "basic render"];

fn is_real_gpu(d: &Device) -> bool {
    let n = d.description.to_ascii_lowercase();
    d.gpu_index.is_some()
        && matches!(d.kind, DeviceKind::Gpu | DeviceKind::IntegratedGpu)
        && !SOFTWARE_GPUS.iter().any(|s| n.contains(s))
}

/// `FLOW_WHISPER_DEVICE`: "cpu" forces the CPU; anything else (or unset) = auto.
pub fn force_cpu_from_env() -> bool {
    std::env::var("FLOW_WHISPER_DEVICE").is_ok_and(|v| v.trim().eq_ignore_ascii_case("cpu"))
}

/// See the module docs.
pub fn choose(devices: &[Device], force_cpu: bool) -> Plan {
    if force_cpu {
        return Plan::Cpu;
    }
    let best = devices.iter().filter(|d| is_real_gpu(d)).max_by_key(|d| {
        (
            u8::from(d.kind == DeviceKind::Gpu),
            d.memory_free,
            std::cmp::Reverse(d.gpu_index),
        )
    });
    match best {
        Some(d) => Plan::Gpu {
            gpu_index: d.gpu_index.expect("real GPUs have an index"),
            backend: d.backend.clone(),
            description: d.description.clone(),
        },
        None => Plan::Cpu,
    }
}

fn c_str(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: ggml returns NUL-terminated strings it owns.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

/// The devices whisper.cpp's ggml sees in this build (the CPU build: only
/// the CPU).
pub fn devices() -> Vec<Device> {
    use whisper_rs::whisper_rs_sys as sys;
    let mut out = Vec::new();
    let mut gpus = 0;
    // SAFETY: ggml's device registry API; every pointer comes from it.
    unsafe {
        for i in 0..sys::ggml_backend_dev_count() {
            let dev = sys::ggml_backend_dev_get(i);
            if dev.is_null() {
                continue;
            }
            let t = sys::ggml_backend_dev_type(dev);
            let kind = if t == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU {
                DeviceKind::Gpu
            } else if t == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU {
                DeviceKind::IntegratedGpu
            } else if t == sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_CPU {
                DeviceKind::Cpu
            } else {
                DeviceKind::Accelerator
            };
            // whisper.cpp counts GPU and integrated GPU devices for
            // `gpu_device` (whisper_backend_init_gpu).
            let gpu_index = matches!(kind, DeviceKind::Gpu | DeviceKind::IntegratedGpu).then(|| {
                gpus += 1;
                gpus - 1
            });
            let (mut free, mut total) = (0usize, 0usize);
            sys::ggml_backend_dev_memory(dev, &mut free, &mut total);
            let reg = sys::ggml_backend_dev_backend_reg(dev);
            out.push(Device {
                gpu_index,
                name: c_str(sys::ggml_backend_dev_name(dev)),
                description: c_str(sys::ggml_backend_dev_description(dev)),
                backend: if reg.is_null() {
                    String::new()
                } else {
                    c_str(sys::ggml_backend_reg_name(reg))
                },
                kind,
                memory_free: free as u64,
                memory_total: total as u64,
            });
        }
    }
    out
}

/// Start the plan's GPU once and free it again. whisper.cpp quietly falls
/// back to the CPU when its GPU backend does not start, so without this
/// check the app would say "GPU" for a model that runs on the CPU.
pub fn gpu_starts(gpu_index: i32) -> Result<(), String> {
    use whisper_rs::whisper_rs_sys as sys;
    let mut seen = 0;
    // SAFETY: as in `devices`; the backend is freed right away.
    unsafe {
        for i in 0..sys::ggml_backend_dev_count() {
            let dev = sys::ggml_backend_dev_get(i);
            if dev.is_null() {
                continue;
            }
            let t = sys::ggml_backend_dev_type(dev);
            if t != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU
                && t != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU
            {
                continue;
            }
            if seen == gpu_index {
                let b = sys::ggml_backend_dev_init(dev, std::ptr::null());
                if b.is_null() {
                    return Err(format!(
                        "{} did not start",
                        c_str(sys::ggml_backend_dev_name(dev))
                    ));
                }
                sys::ggml_backend_free(b);
                return Ok(());
            }
            seen += 1;
        }
    }
    Err(format!("no GPU device {gpu_index}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(gpu_index: Option<i32>, kind: DeviceKind, desc: &str, free_mb: u64) -> Device {
        Device {
            gpu_index,
            name: desc.into(),
            description: desc.into(),
            backend: if kind == DeviceKind::Cpu { "CPU" } else { "Vulkan" }.into(),
            kind,
            memory_free: free_mb << 20,
            memory_total: free_mb << 20,
        }
    }

    fn cpu() -> Device {
        dev(None, DeviceKind::Cpu, "Intel Core Ultra", 30_000)
    }

    #[test]
    fn cpu_only_build_runs_on_the_cpu() {
        assert_eq!(choose(&[cpu()], false), Plan::Cpu);
        assert_eq!(choose(&[], false), Plan::Cpu);
    }

    #[test]
    fn an_integrated_gpu_is_used() {
        let p = choose(
            &[
                dev(Some(0), DeviceKind::IntegratedGpu, "Intel(R) Arc(TM) 140V GPU", 18_000),
                cpu(),
            ],
            false,
        );
        assert_eq!(
            p,
            Plan::Gpu {
                gpu_index: 0,
                backend: "Vulkan".into(),
                description: "Intel(R) Arc(TM) 140V GPU".into()
            }
        );
        assert_eq!((p.backend(), p.device(), p.is_gpu()), ("Vulkan", "Intel(R) Arc(TM) 140V GPU", true));
    }

    #[test]
    fn discrete_beats_integrated_then_more_free_memory_wins() {
        let devs = [
            dev(Some(0), DeviceKind::IntegratedGpu, "iGPU", 16_000),
            dev(Some(1), DeviceKind::Gpu, "small dGPU", 4_000),
            dev(Some(2), DeviceKind::Gpu, "big dGPU", 8_000),
            cpu(),
        ];
        assert!(matches!(choose(&devs, false), Plan::Gpu { gpu_index: 2, .. }));
        let tie = [
            dev(Some(0), DeviceKind::Gpu, "a", 4_000),
            dev(Some(1), DeviceKind::Gpu, "b", 4_000),
        ];
        assert!(matches!(choose(&tie, false), Plan::Gpu { gpu_index: 0, .. }));
    }

    #[test]
    fn software_rasterisers_are_not_gpus() {
        for name in [
            "llvmpipe (LLVM 17.0.6, 256 bits)",
            "SwiftShader Device (Subzero)",
            "Microsoft Basic Render Driver",
        ] {
            assert_eq!(choose(&[dev(Some(0), DeviceKind::Gpu, name, 8_000), cpu()], false), Plan::Cpu, "{name}");
        }
    }

    #[test]
    fn force_cpu_wins_and_accelerators_are_not_used() {
        let devs = [dev(Some(0), DeviceKind::IntegratedGpu, "iGPU", 16_000), cpu()];
        assert_eq!(choose(&devs, true), Plan::Cpu);
        assert_eq!(choose(&[dev(None, DeviceKind::Accelerator, "BLAS", 0), cpu()], false), Plan::Cpu);
    }

    #[test]
    fn a_gpu_plan_falls_back_to_the_cpu() {
        let g = choose(&[dev(Some(0), DeviceKind::Gpu, "dGPU", 8_000)], false);
        assert_eq!(g.clone().fallbacks(), vec![g, Plan::Cpu]);
        assert_eq!(Plan::Cpu.fallbacks(), vec![Plan::Cpu]);
    }

    #[test]
    fn this_build_lists_its_devices_without_panicking() {
        // CPU build: the CPU only. A Vulkan build may add GPUs.
        let d = devices();
        assert!(d.iter().all(|d| d.gpu_index.is_some() == matches!(d.kind, DeviceKind::Gpu | DeviceKind::IntegratedGpu)));
        if !cfg!(feature = "vulkan") {
            assert!(d.iter().all(|d| d.kind != DeviceKind::Gpu && d.kind != DeviceKind::IntegratedGpu), "{d:?}");
            assert!(gpu_starts(0).is_err());
        }
    }
}
