//! Which ggml device runs the model: a GPU (Vulkan, in the Windows
//! `murmur_llm_vulkan.dll` build) or the CPU.
//!
//! [`choose`] is pure, so the policy is unit-tested without llama.cpp:
//! * `MURMUR_LLM_DEVICE=cpu` forces the CPU (A/B runs, a misbehaving driver);
//! * otherwise the best real GPU: a discrete GPU before an integrated one,
//!   then the one with the most free memory. Software rasterisers
//!   (llvmpipe, SwiftShader, Microsoft Basic Render) are not GPUs;
//! * no GPU device (the CPU-only build, no Vulkan driver, or Vulkan failed
//!   to start, which ggml reports as zero devices) = the CPU.
//!
//! The engine loads with the chosen GPU and, when the model or its context
//! fails to load there, loads again on the CPU ([`Plan::fallbacks`]).

/// ggml device type, mirrored so tests need no llama.cpp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Gpu,
    IntegratedGpu,
    Accelerator,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Device {
    /// ggml device index (`LlamaModelParams::with_devices`).
    pub index: usize,
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
    /// Every layer on this GPU device.
    Gpu {
        index: usize,
        backend: String,
        description: String,
    },
    /// The CPU. `gpus_present`: GPU devices exist but are not used, so the
    /// load must exclude them explicitly (llama.cpp offloads to every GPU
    /// by default).
    Cpu { gpus_present: bool },
}

impl Plan {
    /// The plans to try in order: the GPU, then the CPU.
    pub fn fallbacks(self) -> Vec<Plan> {
        match self {
            Plan::Gpu { .. } => vec![self, Plan::Cpu { gpus_present: true }],
            cpu => vec![cpu],
        }
    }

    /// "Vulkan" / "CPU", for the info JSON.
    pub fn backend(&self) -> &str {
        match self {
            Plan::Gpu { backend, .. } => backend,
            Plan::Cpu { .. } => "CPU",
        }
    }

    /// The GPU's name, or "CPU".
    pub fn device(&self) -> &str {
        match self {
            Plan::Gpu { description, .. } => description,
            Plan::Cpu { .. } => "CPU",
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
    matches!(d.kind, DeviceKind::Gpu | DeviceKind::IntegratedGpu)
        && !SOFTWARE_GPUS.iter().any(|s| n.contains(s))
}

/// `MURMUR_LLM_DEVICE`: "cpu" forces the CPU; anything else (or unset) = auto.
pub fn force_cpu_from_env() -> bool {
    murmur_build_info::env::var("MURMUR_LLM_DEVICE")
        .is_ok_and(|v| v.trim().eq_ignore_ascii_case("cpu"))
}

/// See the module docs.
pub fn choose(devices: &[Device], force_cpu: bool) -> Plan {
    let gpus: Vec<&Device> = devices.iter().filter(|d| is_real_gpu(d)).collect();
    let any_gpu = devices
        .iter()
        .any(|d| matches!(d.kind, DeviceKind::Gpu | DeviceKind::IntegratedGpu));
    if force_cpu {
        return Plan::Cpu {
            gpus_present: any_gpu,
        };
    }
    let best = gpus.into_iter().max_by_key(|d| {
        (
            u8::from(d.kind == DeviceKind::Gpu),
            d.memory_free,
            std::cmp::Reverse(d.index),
        )
    });
    match best {
        Some(d) => Plan::Gpu {
            index: d.index,
            backend: d.backend.clone(),
            description: d.description.clone(),
        },
        None => Plan::Cpu {
            gpus_present: any_gpu,
        },
    }
}

/// The devices llama.cpp sees in this build.
pub fn devices() -> Vec<Device> {
    use llama_cpp_2::LlamaBackendDeviceType as T;
    llama_cpp_2::list_llama_ggml_backend_devices()
        .into_iter()
        .map(|d| Device {
            index: d.index,
            name: d.name,
            description: d.description,
            backend: d.backend,
            kind: match d.device_type {
                T::Cpu => DeviceKind::Cpu,
                T::Gpu => DeviceKind::Gpu,
                T::IntegratedGpu => DeviceKind::IntegratedGpu,
                T::Accelerator => DeviceKind::Accelerator,
                T::Unknown => DeviceKind::Unknown,
            },
            memory_free: d.memory_free as u64,
            memory_total: d.memory_total as u64,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(index: usize, kind: DeviceKind, desc: &str, free_mb: u64) -> Device {
        Device {
            index,
            name: format!("Dev{index}"),
            description: desc.into(),
            backend: if kind == DeviceKind::Cpu {
                "CPU"
            } else {
                "Vulkan"
            }
            .into(),
            kind,
            memory_free: free_mb << 20,
            memory_total: free_mb << 20,
        }
    }

    fn cpu() -> Device {
        dev(0, DeviceKind::Cpu, "Intel Core Ultra", 30_000)
    }

    #[test]
    fn cpu_only_build_runs_on_the_cpu_untouched() {
        // The CPU build (and Android) sees only the CPU device: default
        // model params, nothing excluded.
        assert_eq!(
            choose(&[cpu()], false),
            Plan::Cpu {
                gpus_present: false
            }
        );
        assert_eq!(
            choose(&[], false),
            Plan::Cpu {
                gpus_present: false
            }
        );
    }

    #[test]
    fn an_integrated_gpu_is_used() {
        let p = choose(
            &[
                dev(
                    0,
                    DeviceKind::IntegratedGpu,
                    "Intel(R) Arc(TM) 140V GPU",
                    18_000,
                ),
                dev(1, DeviceKind::Cpu, "cpu", 30_000),
            ],
            false,
        );
        assert_eq!(
            p,
            Plan::Gpu {
                index: 0,
                backend: "Vulkan".into(),
                description: "Intel(R) Arc(TM) 140V GPU".into()
            }
        );
        assert_eq!(
            (p.backend(), p.device(), p.is_gpu()),
            ("Vulkan", "Intel(R) Arc(TM) 140V GPU", true)
        );
    }

    #[test]
    fn discrete_beats_integrated_then_more_free_memory_wins() {
        let devs = [
            dev(0, DeviceKind::IntegratedGpu, "iGPU", 16_000),
            dev(1, DeviceKind::Gpu, "small dGPU", 4_000),
            dev(2, DeviceKind::Gpu, "big dGPU", 8_000),
            dev(3, DeviceKind::Cpu, "cpu", 30_000),
        ];
        assert!(matches!(choose(&devs, false), Plan::Gpu { index: 2, .. }));
        // Ties go to the lower index (llama.cpp's own order).
        let tie = [
            dev(0, DeviceKind::Gpu, "a", 4_000),
            dev(1, DeviceKind::Gpu, "b", 4_000),
        ];
        assert!(matches!(choose(&tie, false), Plan::Gpu { index: 0, .. }));
    }

    #[test]
    fn software_rasterisers_are_not_gpus() {
        for name in [
            "llvmpipe (LLVM 17.0.6, 256 bits)",
            "SwiftShader Device (Subzero)",
            "Microsoft Basic Render Driver",
        ] {
            let p = choose(&[dev(0, DeviceKind::Gpu, name, 8_000), cpu()], false);
            // A GPU-type device exists, so the CPU load must exclude it.
            assert_eq!(p, Plan::Cpu { gpus_present: true }, "{name}");
        }
    }

    #[test]
    fn force_cpu_excludes_the_gpus() {
        let devs = [dev(0, DeviceKind::IntegratedGpu, "iGPU", 16_000), cpu()];
        assert_eq!(choose(&devs, true), Plan::Cpu { gpus_present: true });
        assert_eq!(
            choose(&[cpu()], true),
            Plan::Cpu {
                gpus_present: false
            }
        );
    }

    #[test]
    fn accelerators_are_not_used() {
        // e.g. a BLAS "device": not a place for the layers.
        let p = choose(&[dev(0, DeviceKind::Accelerator, "BLAS", 0), cpu()], false);
        assert_eq!(
            p,
            Plan::Cpu {
                gpus_present: false
            }
        );
    }

    #[test]
    fn a_gpu_plan_falls_back_to_the_cpu() {
        let g = choose(&[dev(0, DeviceKind::Gpu, "dGPU", 8_000)], false);
        let seq = g.clone().fallbacks();
        assert_eq!(seq, vec![g, Plan::Cpu { gpus_present: true }]);
        assert_eq!(
            Plan::Cpu {
                gpus_present: false
            }
            .fallbacks()
            .len(),
            1
        );
    }
}
