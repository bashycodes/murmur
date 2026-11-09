//! `PluginWhisper`: whisper.cpp from the `murmur-whisper` plugin library
//! (feature `whisper-plugin`), the GPU speech engine of the Windows app.
//!
//! The app links whisper.cpp for the CPU (feature `whisper`). The Vulkan
//! build cannot be linked in: the exe would then need `vulkan-1.dll` to
//! start. So it ships as `murmur_whisper_vulkan.dll`, loaded here at runtime
//! (C ABI documented in `crates/murmur-whisper/src/lib.rs`), the way the
//! cleanup engine is (`crate::llama`, D10). When it does not load (no
//! Vulkan driver), [`PluginWhisper::load`] says so and the shell uses the
//! built-in CPU [`crate::whisper::WhisperEngine`]. Inside the plugin, no
//! usable GPU or a GPU that fails to load the model means the CPU.
//!
//! Plugin lookup, first hit wins:
//! 1. an explicit path, or the `MURMUR_WHISPER_LIB` environment variable;
//! 2. [`GPU_PLUGIN_FILE`] next to the current executable, then in its
//!    parent directory (cargo puts test binaries in `target/<profile>/deps/`).

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use libloading::{Library, Symbol};

use crate::asr::{AsrEngine, AsrError, AsrOutput, AsrRequest};

const ABI_VERSION: u32 = 1;

/// The GPU (Vulkan) build of the plugin, shipped next to the exe by the
/// Windows installers.
pub const GPU_PLUGIN_FILE: &str = if cfg!(windows) {
    "murmur_whisper_vulkan.dll"
} else if cfg!(target_os = "macos") {
    "libmurmur_whisper_vulkan.dylib"
} else {
    "libmurmur_whisper_vulkan.so"
};

#[repr(C)]
struct Handle {
    _opaque: [u8; 0],
}

type LoadFn = unsafe extern "C" fn(*const c_char, *const c_char, *mut *mut c_char) -> *mut Handle;
type TranscribeFn = unsafe extern "C" fn(
    *const Handle,
    *const f32,
    usize,
    *const c_char,
    *const bool,
    *mut *mut c_char,
) -> i32;
type InfoFn = unsafe extern "C" fn(*const Handle) -> *mut c_char;
type BackendsFn = unsafe extern "C" fn() -> *mut c_char;
type FreeStrFn = unsafe extern "C" fn(*mut c_char);
type UnloadFn = unsafe extern "C" fn(*mut Handle);

/// Why [`PluginWhisper::load`] or [`probe_backend`] failed.
#[derive(Debug, thiserror::Error)]
pub enum WhisperPluginError {
    /// No lookup location held a loadable plugin (no plugin shipped, or
    /// `vulkan-1.dll` missing); `tried` = "path: error".
    #[error("whisper GPU plugin not found ({})", tried.join("; "))]
    PluginNotFound { tried: Vec<String> },
    /// The plugin loaded but failed (bad ABI, model load error, ...).
    #[error("{0}")]
    Backend(String),
}

/// Where a whisper model runs, from the plugin's JSON (`murmur_whisper_info`
/// or `murmur_whisper_backends`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct WhisperBackend {
    /// "Vulkan", "CPU", …
    pub backend: String,
    /// The GPU's name, or "CPU".
    pub device: String,
    /// Set when a GPU was found but failed and the CPU took over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_error: Option<String>,
}

impl WhisperBackend {
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

    /// The CPU (the built-in engine, or the plugin without a GPU).
    pub fn cpu() -> Self {
        Self {
            backend: "CPU".into(),
            device: "CPU".into(),
            gpu_error: None,
        }
    }

    /// The model runs on a GPU (any backend but the CPU).
    pub fn is_gpu(&self) -> bool {
        !self.backend.eq_ignore_ascii_case("cpu")
    }
}

/// Candidate plugin paths in lookup order.
pub fn plugin_candidates(explicit: Option<&str>) -> Vec<PathBuf> {
    if let Some(p) = explicit.filter(|p| !p.is_empty()) {
        return vec![PathBuf::from(p)];
    }
    if let Some(p) = murmur_build_info::env::var_os("MURMUR_WHISPER_LIB") {
        return vec![PathBuf::from(p)];
    }
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        let up = dir.parent().map(PathBuf::from);
        for d in std::iter::once(dir).chain(up) {
            v.push(d.join(GPU_PLUGIN_FILE));
        }
    }
    v
}

/// Plugins are loaded once per path and never closed (as for the cleanup
/// plugin: unloading a library with live driver threads crashes at exit).
static PLUGINS: Mutex<Vec<(PathBuf, &'static Library)>> = Mutex::new(Vec::new());

fn open_plugin(p: &PathBuf) -> Result<&'static Library, libloading::Error> {
    let mut v = PLUGINS.lock().unwrap();
    if let Some((_, l)) = v.iter().find(|(q, _)| q == p) {
        return Ok(l);
    }
    // SAFETY: loading our own plugin; its initialisers are whisper.cpp's.
    let l: &'static Library = Box::leak(Box::new(unsafe { Library::new(p) }?));
    v.push((p.clone(), l));
    Ok(l)
}

fn find_plugin(plugin: Option<&str>) -> Result<(&'static Library, PathBuf), WhisperPluginError> {
    let mut errs = Vec::new();
    for p in plugin_candidates(plugin) {
        match open_plugin(&p) {
            Ok(l) => return Ok((l, p)),
            Err(e) => errs.push(format!("{}: {e}", p.display())),
        }
    }
    if errs.is_empty() {
        errs.push("no location to look in".into());
    }
    Err(WhisperPluginError::PluginNotFound { tried: errs })
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

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("NULs removed")
}

/// The plugin [`PluginWhisper::load`] would use and where it would run a
/// model, without loading one.
pub fn probe_backend(
    plugin: Option<&str>,
) -> Result<(PathBuf, WhisperBackend), WhisperPluginError> {
    let (lib, path) = find_plugin(plugin)?;
    let sym_err =
        |e: libloading::Error| WhisperPluginError::Backend(format!("whisper plugin: {e}"));
    // SAFETY: symbol types match the ABI in crates/murmur-whisper/src/lib.rs.
    unsafe {
        let free_str: FreeStrFn = *lib
            .get::<FreeStrFn>(b"murmur_whisper_free_string\0")
            .map_err(sym_err)?;
        let f: BackendsFn = *lib
            .get::<BackendsFn>(b"murmur_whisper_backends\0")
            .map_err(sym_err)?;
        let json = take_string(free_str, f());
        Ok((
            path,
            WhisperBackend::from_info(&json).unwrap_or_else(WhisperBackend::cpu),
        ))
    }
}

/// Load options sent to the plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct PluginOptions {
    pub threads: usize,
    /// `ModelConfig.whisper_fit_ctx`.
    pub fit_ctx: bool,
    /// Silero VAD model for whisper.cpp's built-in VAD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vad_model: Option<String>,
}

/// A whisper model loaded in the plugin.
pub struct PluginWhisper {
    handle: *mut Handle,
    transcribe_fn: TranscribeFn,
    free_str: FreeStrFn,
    unload: UnloadFn,
    /// Plugin-reported load info (JSON).
    pub info: String,
    pub plugin_path: PathBuf,
}

// SAFETY: the plugin's engine serialises decodes behind a mutex; every
// entry point may be called from any thread.
unsafe impl Send for PluginWhisper {}
unsafe impl Sync for PluginWhisper {}

impl PluginWhisper {
    pub fn load(
        model: &str,
        opts: &PluginOptions,
        plugin: Option<&str>,
    ) -> Result<Self, WhisperPluginError> {
        let (lib, plugin_path) = find_plugin(plugin)?;
        let sym_err =
            |e: libloading::Error| WhisperPluginError::Backend(format!("whisper plugin: {e}"));
        // SAFETY: symbol types match the ABI in crates/murmur-whisper/src/lib.rs;
        // the version check guards against a mismatched plugin.
        unsafe {
            let ver: Symbol<unsafe extern "C" fn() -> u32> =
                lib.get(b"murmur_whisper_abi_version\0").map_err(sym_err)?;
            if ver() != ABI_VERSION {
                return Err(WhisperPluginError::Backend(format!(
                    "whisper plugin ABI {} != {ABI_VERSION}",
                    ver()
                )));
            }
            let load: LoadFn = *lib
                .get::<LoadFn>(b"murmur_whisper_load\0")
                .map_err(sym_err)?;
            let transcribe_fn: TranscribeFn = *lib
                .get::<TranscribeFn>(b"murmur_whisper_transcribe\0")
                .map_err(sym_err)?;
            let info_fn: InfoFn = *lib
                .get::<InfoFn>(b"murmur_whisper_info\0")
                .map_err(sym_err)?;
            let free_str: FreeStrFn = *lib
                .get::<FreeStrFn>(b"murmur_whisper_free_string\0")
                .map_err(sym_err)?;
            let unload: UnloadFn = *lib
                .get::<UnloadFn>(b"murmur_whisper_unload\0")
                .map_err(sym_err)?;
            let (m, o) = (
                cstr(model),
                cstr(&serde_json::to_string(opts).expect("options serialise")),
            );
            let mut err: *mut c_char = std::ptr::null_mut();
            let handle = load(m.as_ptr(), o.as_ptr(), &mut err);
            if handle.is_null() {
                return Err(WhisperPluginError::Backend(take_string(free_str, err)));
            }
            let info = take_string(free_str, info_fn(handle));
            Ok(Self {
                handle,
                transcribe_fn,
                free_str,
                unload,
                info,
                plugin_path,
            })
        }
    }

    /// Where the model runs (GPU or CPU), from the load-time info.
    pub fn backend(&self) -> WhisperBackend {
        WhisperBackend::from_info(&self.info).unwrap_or_else(WhisperBackend::cpu)
    }
}

impl AsrEngine for PluginWhisper {
    fn transcribe(&self, pcm: &[f32], req: &AsrRequest) -> Result<AsrOutput, AsrError> {
        let r = cstr(
            &serde_json::json!({
                "initial_prompt": req.initial_prompt,
                "language": req.language,
                "prior_text": req.prior_text,
            })
            .to_string(),
        );
        // `req.abort` (an Arc) is borrowed for the whole blocking call.
        let flag: *const bool = req
            .abort
            .0
            .as_ref()
            .map_or(std::ptr::null(), |a| AtomicBool::as_ptr(a) as *const bool);
        let mut out: *mut c_char = std::ptr::null_mut();
        // SAFETY: live handle; `pcm` and the strings outlive the call.
        let rc = unsafe {
            (self.transcribe_fn)(
                self.handle,
                pcm.as_ptr(),
                pcm.len(),
                r.as_ptr(),
                flag,
                &mut out,
            )
        };
        let s = take_string(self.free_str, out);
        if rc != 0 {
            return Err(AsrError::Transcribe(s));
        }
        let v: serde_json::Value = serde_json::from_str(&s)
            .map_err(|e| AsrError::Transcribe(format!("plugin json: {e}")))?;
        Ok(AsrOutput {
            text: v["text"].as_str().unwrap_or_default().to_string(),
            language: v["language"].as_str().map(str::to_string),
            no_speech_prob: v["no_speech_prob"].as_f64().map(|p| p as f32),
        })
    }
}

impl Drop for PluginWhisper {
    fn drop(&mut self) {
        // SAFETY: handle from murmur_whisper_load, freed exactly once; the
        // library is never closed.
        unsafe { (self.unload)(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_path_is_the_only_candidate() {
        assert_eq!(
            plugin_candidates(Some("/x/libp.so")),
            vec![PathBuf::from("/x/libp.so")]
        );
    }

    #[test]
    fn default_lookup_is_the_gpu_build_next_to_the_exe() {
        if std::env::var_os("MURMUR_WHISPER_LIB").is_some() {
            return;
        }
        let c = plugin_candidates(None);
        assert!(!c.is_empty());
        assert!(
            c.iter().all(|p| p.file_name().unwrap() == GPU_PLUGIN_FILE),
            "{c:?}"
        );
    }

    #[test]
    fn backend_from_plugin_info() {
        let gpu = WhisperBackend::from_info(
            r#"{"backend":"Vulkan","device":"Intel(R) Arc(TM) 140V GPU","gpu_error":null,"load_ms":900}"#,
        )
        .unwrap();
        assert!(gpu.is_gpu());
        assert_eq!(gpu.device, "Intel(R) Arc(TM) 140V GPU");
        let fell_back = WhisperBackend::from_info(
            r#"{"backend":"CPU","device":"CPU","gpu_error":"iGPU: out of memory"}"#,
        )
        .unwrap();
        assert!(!fell_back.is_gpu());
        assert!(fell_back.gpu_error.unwrap().contains("out of memory"));
        assert_eq!(WhisperBackend::from_info("not json"), None);
        assert!(!WhisperBackend::cpu().is_gpu());
    }

    #[test]
    fn missing_plugin_is_plugin_not_found() {
        let e = probe_backend(Some("/nonexistent/murmur_whisper_vulkan.dll"))
            .err()
            .unwrap();
        assert!(
            matches!(e, WhisperPluginError::PluginNotFound { .. }),
            "{e}"
        );
        let e = PluginWhisper::load(
            "m.bin",
            &PluginOptions::default(),
            Some("/nonexistent/x.so"),
        )
        .err()
        .unwrap();
        assert!(e.to_string().contains("plugin not found"), "{e}");
    }

    #[test]
    fn options_json_leaves_out_an_unset_vad_model() {
        let j = serde_json::to_value(PluginOptions {
            threads: 4,
            fit_ctx: true,
            vad_model: None,
        })
        .unwrap();
        assert_eq!(j, serde_json::json!({"threads": 4, "fit_ctx": true}));
    }
}
