//! `LlamaCleaner`: the llama.cpp [`Cleaner`] (feature `llm`).
//!
//! The engine lives in the `flow-llm` plugin library, which this module
//! loads at runtime. It cannot be linked in: llama.cpp and whisper.cpp each
//! bundle a different ggml, and one binary holding both fails to link with
//! duplicate symbols. The plugin exports only `flow_llm_*`, so its ggml stays
//! private (C ABI documented in `crates/flow-llm/src/lib.rs`).
//!
//! Plugin lookup, first hit wins:
//! 1. `ModelConfig.llm_lib`, or the `FLOW_LLM_LIB` environment variable;
//! 2. next to the current executable, then its parent directory (cargo
//!    puts test binaries in `target/<profile>/deps/`): in each, the GPU
//!    build [`GPU_PLUGIN_FILE`] first, then [`PLUGIN_FILE`]. The GPU build
//!    links the Vulkan loader, so on a PC without `vulkan-1.dll` it does not
//!    load and the CPU build is next; with Vulkan but no usable GPU it runs
//!    on the CPU itself (flow-llm `backend`);
//! 3. the bare file name, which leaves the search to the system loader
//!    (on Android this finds the app's packaged `libflow_llm.so`).

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Instant;

use libloading::{Library, Symbol};

use crate::asr::AbortFlag;
use crate::cleanup::{self, CleanError, CleanPrompt, Cleaner};

const ABI_VERSION: u32 = 1;

/// Platform file name of the plugin.
pub const PLUGIN_FILE: &str = if cfg!(windows) {
    "flow_llm.dll"
} else if cfg!(target_os = "macos") {
    "libflow_llm.dylib"
} else {
    "libflow_llm.so"
};

/// The GPU (Vulkan) build of the plugin, shipped next to [`PLUGIN_FILE`]
/// by the Windows installers.
pub const GPU_PLUGIN_FILE: &str = if cfg!(windows) {
    "flow_llm_vulkan.dll"
} else if cfg!(target_os = "macos") {
    "libflow_llm_vulkan.dylib"
} else {
    "libflow_llm_vulkan.so"
};

type LoadFn =
    unsafe extern "C" fn(*const c_char, i32, *const c_char, *mut *mut c_char) -> *mut Handle;
type CleanFn = unsafe extern "C" fn(
    *const Handle,
    *const c_char,
    *const c_char,
    u64,
    *const bool,
    *mut *mut c_char,
) -> i32;
type InfoFn = unsafe extern "C" fn(*const Handle) -> *mut c_char;
type FreeStrFn = unsafe extern "C" fn(*mut c_char);
type UnloadFn = unsafe extern "C" fn(*mut Handle);
type PrefillFn = unsafe extern "C" fn(*const Handle, *const c_char) -> i32;
type PrefillCancelFn = unsafe extern "C" fn(*const Handle);

/// Why [`LlamaCleaner::load`] failed.
#[derive(Debug, thiserror::Error)]
pub enum LlamaLoadError {
    /// No lookup location held a loadable plugin; `tried` = "path: error".
    #[error("llm plugin not found ({})", tried.join("; "))]
    PluginNotFound { tried: Vec<String> },
    /// The plugin loaded but failed (bad ABI, model load error, ...).
    #[error("{0}")]
    Backend(String),
}

#[repr(C)]
struct Handle {
    _opaque: [u8; 0],
}

/// Candidate plugin paths in lookup order.
pub fn plugin_candidates(explicit: Option<&str>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = explicit.filter(|p| !p.is_empty()) {
        v.push(PathBuf::from(p));
        return v;
    }
    if let Some(p) = std::env::var_os("FLOW_LLM_LIB") {
        v.push(PathBuf::from(p));
        return v;
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        let up = dir.parent().map(PathBuf::from);
        for d in std::iter::once(dir).chain(up) {
            v.push(d.join(GPU_PLUGIN_FILE));
            v.push(d.join(PLUGIN_FILE));
        }
    }
    v.push(PathBuf::from(PLUGIN_FILE));
    v
}

/// Where the plugin runs a model, from its info JSON (`flow_llm_info` or
/// `flow_llm_backends`). Plugins built before 2026-10-30 report neither
/// field: `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct LlmBackend {
    /// "Vulkan", "CPU", …
    pub backend: String,
    /// The GPU's name, or "CPU".
    pub device: String,
    /// Set when a GPU was found but its load failed and the CPU took over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_error: Option<String>,
}

impl LlmBackend {
    pub fn from_info(json: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let backend = v.get("backend")?.as_str()?.to_string();
        Some(Self {
            device: v
                .get("device")
                .and_then(|d| d.as_str())
                .unwrap_or(&backend)
                .to_string(),
            gpu_error: v
                .get("gpu_error")
                .and_then(|e| e.as_str())
                .map(str::to_string),
            backend,
        })
    }

    /// The model runs on a GPU (any backend but the CPU).
    pub fn is_gpu(&self) -> bool {
        !self.backend.eq_ignore_ascii_case("cpu")
    }
}

/// The plugin that [`LlamaCleaner::load`] would use and where it would run
/// a model, without loading one (`flow_llm_backends`). A plugin too old to
/// export it reports the CPU.
pub fn probe_backend(plugin: Option<&str>) -> Result<(PathBuf, LlmBackend), LlamaLoadError> {
    let (lib, path) = find_plugin(plugin)?;
    type BackendsFn = unsafe extern "C" fn() -> *mut c_char;
    // SAFETY: symbol types match the ABI in crates/flow-llm/src/lib.rs.
    unsafe {
        let Ok(free_str) = lib.get::<FreeStrFn>(b"flow_llm_free_string\0").map(|f| *f) else {
            return Err(LlamaLoadError::Backend(
                "llm plugin: no flow_llm_free_string".into(),
            ));
        };
        let cpu = LlmBackend {
            backend: "CPU".into(),
            device: "CPU".into(),
            gpu_error: None,
        };
        let Ok(f) = lib.get::<BackendsFn>(b"flow_llm_backends\0").map(|f| *f) else {
            return Ok((path, cpu));
        };
        let json = take_string(free_str, f());
        Ok((path, LlmBackend::from_info(&json).unwrap_or(cpu)))
    }
}

/// The first loadable plugin of [`plugin_candidates`].
fn find_plugin(plugin: Option<&str>) -> Result<(&'static Library, PathBuf), LlamaLoadError> {
    let mut errs = Vec::new();
    for p in plugin_candidates(plugin) {
        match open_plugin(&p) {
            Ok(l) => return Ok((l, p)),
            Err(e) => errs.push(format!("{}: {e}", p.display())),
        }
    }
    Err(LlamaLoadError::PluginNotFound { tried: errs })
}

fn take_string(free: FreeStrFn, p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the plugin returned a NUL-terminated string it owns.
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { free(p) };
    s
}

/// The plugin's message format: `[[role, content], …]`.
fn messages_json(p: &CleanPrompt) -> String {
    serde_json::to_string(&p.messages()).expect("strings serialise")
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("NULs removed")
}

pub struct LlamaCleaner {
    handle: *mut Handle,
    clean: CleanFn,
    info_fn: InfoFn,
    free_str: FreeStrFn,
    unload: UnloadFn,
    /// Optional (plugins built before prefill-ahead lack them).
    prefill: Option<PrefillFn>,
    prefill_cancel: Option<PrefillCancelFn>,
    /// Plugin-reported load info (JSON), for the CLI and tests.
    pub info: String,
    pub plugin_path: PathBuf,
}

/// Plugins are loaded once per path and never closed. dlclose of a library
/// that owns OpenMP worker threads and C++/Rust TLS destructors segfaults at
/// exit. `unload` still frees the model and context, which is the memory
/// that matters; the code pages stay mapped.
static PLUGINS: Mutex<Vec<(PathBuf, &'static Library)>> = Mutex::new(Vec::new());

fn open_plugin(p: &PathBuf) -> Result<&'static Library, libloading::Error> {
    let mut v = PLUGINS.lock().unwrap();
    if let Some((_, l)) = v.iter().find(|(q, _)| q == p) {
        return Ok(l);
    }
    // SAFETY: loading our own plugin; its initialisers are llama.cpp's.
    let l: &'static Library = Box::leak(Box::new(unsafe { Library::new(p) }?));
    v.push((p.clone(), l));
    Ok(l)
}

// SAFETY: the plugin's handle is internally synchronised (a job channel
// onto its own thread); every entry point may be called from any thread.
unsafe impl Send for LlamaCleaner {}
unsafe impl Sync for LlamaCleaner {}

impl LlamaCleaner {
    /// Load the plugin and the GGUF model, then prefill the static prompt
    /// prefix ([`cleanup::warm_prompt`]) so the first commit is already warm.
    pub fn load(model: &str, threads: usize, plugin: Option<&str>) -> Result<Self, LlamaLoadError> {
        let (lib, plugin_path) = find_plugin(plugin)?;
        let sym_err = |e: libloading::Error| LlamaLoadError::Backend(format!("llm plugin: {e}"));
        // SAFETY: symbol types match the ABI in crates/flow-llm/src/lib.rs;
        // the version check guards against a mismatched plugin.
        unsafe {
            let ver: Symbol<unsafe extern "C" fn() -> u32> =
                lib.get(b"flow_llm_abi_version\0").map_err(sym_err)?;
            if ver() != ABI_VERSION {
                return Err(LlamaLoadError::Backend(format!(
                    "llm plugin ABI {} != {ABI_VERSION}",
                    ver()
                )));
            }
            let load: LoadFn = *lib.get::<LoadFn>(b"flow_llm_load\0").map_err(sym_err)?;
            let clean: CleanFn = *lib.get::<CleanFn>(b"flow_llm_clean\0").map_err(sym_err)?;
            let info_fn: InfoFn = *lib.get::<InfoFn>(b"flow_llm_info\0").map_err(sym_err)?;
            let free_str: FreeStrFn = *lib
                .get::<FreeStrFn>(b"flow_llm_free_string\0")
                .map_err(sym_err)?;
            let unload: UnloadFn = *lib.get::<UnloadFn>(b"flow_llm_unload\0").map_err(sym_err)?;
            let prefill = lib.get::<PrefillFn>(b"flow_llm_prefill\0").ok().map(|f| *f);
            let prefill_cancel = lib
                .get::<PrefillCancelFn>(b"flow_llm_prefill_cancel\0")
                .ok()
                .map(|f| *f);

            // The static prompt prefix (system + example turns) of every
            // light-level commit; see `cleanup::warm_prompt`.
            let warm = cleanup::warm_prompt();
            let (m, wm) = (cstr(model), cstr(&messages_json(&warm)));
            let mut err: *mut c_char = std::ptr::null_mut();
            let threads = i32::try_from(threads.max(1)).unwrap_or(4);
            let handle = load(m.as_ptr(), threads, wm.as_ptr(), &mut err);
            if handle.is_null() {
                return Err(LlamaLoadError::Backend(take_string(free_str, err)));
            }
            let info = take_string(free_str, info_fn(handle));
            Ok(Self {
                handle,
                clean,
                info_fn,
                free_str,
                unload,
                prefill,
                prefill_cancel,
                info,
                plugin_path,
            })
        }
    }
}

impl LlamaCleaner {
    /// Plugin info JSON, including `last` = prefill/decode counters of the
    /// most recent call.
    /// Where the model runs (GPU or CPU), from the load-time info.
    pub fn backend(&self) -> Option<LlmBackend> {
        LlmBackend::from_info(&self.info)
    }

    pub fn info_json(&self) -> String {
        // SAFETY: live handle.
        take_string(self.free_str, unsafe { (self.info_fn)(self.handle) })
    }
}

impl Cleaner for LlamaCleaner {
    fn clean(&self, prompt: &CleanPrompt, deadline: Instant) -> Result<String, CleanError> {
        self.clean_cancellable(prompt, deadline, &AbortFlag::default())
    }

    fn prefill(&self, prompt: &CleanPrompt) {
        if let Some(f) = self.prefill {
            let m = cstr(&messages_json(prompt));
            // SAFETY: live handle, NUL-terminated string; the plugin copies it.
            unsafe { f(self.handle, m.as_ptr()) };
        }
    }

    fn cancel_prefill(&self) {
        if let Some(f) = self.prefill_cancel {
            // SAFETY: live handle.
            unsafe { f(self.handle) };
        }
    }

    fn info_json(&self) -> Option<String> {
        Some(LlamaCleaner::info_json(self))
    }

    fn clean_cancellable(
        &self,
        prompt: &CleanPrompt,
        deadline: Instant,
        cancel: &AbortFlag,
    ) -> Result<String, CleanError> {
        let budget = deadline.saturating_duration_since(Instant::now());
        let (msgs, t) = (cstr(&messages_json(prompt)), cstr(&prompt.transcript));
        // `cancel` (an Arc clone) is borrowed for the whole blocking call.
        let flag: *const bool = cancel
            .0
            .as_ref()
            .map_or(std::ptr::null(), |a| AtomicBool::as_ptr(a) as *const bool);
        let mut out: *mut c_char = std::ptr::null_mut();
        // SAFETY: valid handle and NUL-terminated strings for the call.
        let rc = unsafe {
            (self.clean)(
                self.handle,
                msgs.as_ptr(),
                t.as_ptr(),
                budget.as_millis() as u64,
                flag,
                &mut out,
            )
        };
        let text = take_string(self.free_str, out);
        match rc {
            0 => Ok(text),
            1 => Err(CleanError::Backend("llm deadline".into())),
            _ => Err(CleanError::Backend(text)),
        }
    }
}

impl Drop for LlamaCleaner {
    fn drop(&mut self) {
        // SAFETY: handle from flow_llm_load, freed exactly once; the library
        // is never closed.
        unsafe { (self.unload)(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_plugin_path_wins() {
        assert_eq!(
            plugin_candidates(Some("/x/libp.so")),
            vec![PathBuf::from("/x/libp.so")]
        );
    }

    #[test]
    fn default_lookup_ends_with_bare_name() {
        if std::env::var_os("FLOW_LLM_LIB").is_some() {
            return;
        }
        let c = plugin_candidates(None);
        assert_eq!(c.last().unwrap(), &PathBuf::from(PLUGIN_FILE));
        assert!(c.len() >= 2);
    }

    #[test]
    fn the_gpu_build_is_tried_before_the_cpu_build() {
        if std::env::var_os("FLOW_LLM_LIB").is_some() {
            return;
        }
        let c = plugin_candidates(None);
        let names: Vec<_> = c
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        // exe dir, then its parent: GPU, CPU in each; then the bare name.
        assert_eq!(
            &names[..4],
            [GPU_PLUGIN_FILE, PLUGIN_FILE, GPU_PLUGIN_FILE, PLUGIN_FILE]
        );
        assert_eq!(c[0].parent(), c[1].parent());
        // An explicit path is the only candidate (no GPU detour).
        assert_eq!(plugin_candidates(Some("/x/flow_llm.dll")).len(), 1);
    }

    #[test]
    fn backend_from_plugin_info() {
        let gpu = LlmBackend::from_info(
            r#"{"n_params":1,"backend":"Vulkan","device":"Intel(R) Arc(TM) 140V GPU","gpu_error":null}"#,
        )
        .unwrap();
        assert!(gpu.is_gpu());
        assert_eq!(gpu.device, "Intel(R) Arc(TM) 140V GPU");
        assert_eq!(gpu.gpu_error, None);
        let fell_back = LlmBackend::from_info(
            r#"{"backend":"CPU","device":"CPU","gpu_error":"llm context (iGPU): out of memory"}"#,
        )
        .unwrap();
        assert!(!fell_back.is_gpu());
        assert!(fell_back.gpu_error.unwrap().contains("out of memory"));
        // A plugin from before the GPU build says nothing about it.
        assert_eq!(LlmBackend::from_info(r#"{"n_params":1,"load_ms":3}"#), None);
        assert_eq!(LlmBackend::from_info("not json"), None);
    }

    #[test]
    fn missing_plugin_is_a_probe_error_too() {
        let e = probe_backend(Some("/nonexistent/libflow_llm.so"))
            .err()
            .unwrap();
        assert!(matches!(e, LlamaLoadError::PluginNotFound { .. }), "{e}");
    }

    #[test]
    fn missing_plugin_is_a_load_error() {
        let e = LlamaCleaner::load("m.gguf", 1, Some("/nonexistent/libflow_llm.so"))
            .err()
            .unwrap();
        assert!(e.to_string().contains("plugin not found"), "{e}");
        assert!(matches!(e, LlamaLoadError::PluginNotFound { .. }), "{e}");
    }
}
