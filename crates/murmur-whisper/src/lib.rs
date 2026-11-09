//! murmur-whisper: whisper.cpp as a plugin library, for a GPU build of the
//! speech engine next to the app's built-in CPU whisper.
//!
//! The app links whisper.cpp for the CPU. A Vulkan whisper.cpp in the same
//! binary would make the exe need `vulkan-1.dll` to start at all, so the
//! GPU build lives here and is loaded at runtime by murmur-core's
//! `whisper_plugin` (Windows: `murmur_whisper_vulkan.dll`). When it does not
//! load (no Vulkan driver) the app keeps its CPU engine. The decode is
//! murmur-core's [`WhisperEngine`], so both transcribe the same way.
//!
//! All strings are UTF-8, NUL-terminated. Strings returned by the plugin
//! are freed with [`murmur_whisper_free_string`].
//!
//! ABI version 1:
//! ```c
//! uint32_t murmur_whisper_abi_version(void);
//! char*    murmur_whisper_backends(void);
//!          // JSON {"devices":[…],"backend":"Vulkan"|"CPU","device":"…"}; no model needed
//! void*    murmur_whisper_load(const char* model, const char* opts, char** err);
//!          // opts = JSON {"threads":4,"fit_ctx":true,"vad_model":null}
//! char*    murmur_whisper_info(void* h);
//!          // JSON {"backend","device","gpu_error","load_ms","multilingual"}
//! int32_t  murmur_whisper_transcribe(void* h, const float* pcm, size_t n,
//!                                  const char* req, const bool* cancel /* nullable */,
//!                                  char** out);
//!          // req = JSON {"initial_prompt":"…","language":"he"|null,"prior_text":null}
//!          // 0 = ok (*out = JSON {"text","language","no_speech_prob"}),
//!          // 2 = error (*out = message)
//! void     murmur_whisper_free_string(char*);
//! void     murmur_whisper_unload(void* h);
//! ```

pub mod backend;

use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use murmur_core::asr::{AsrOutput, AsrRequest};
use murmur_core::whisper::{WhisperDevice, WhisperEngine};

pub const ABI_VERSION: u32 = 1;

pub const OK: i32 = 0;
pub const ERROR: i32 = 2;

/// Load options (`murmur_whisper_load`'s `opts`).
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct LoadOptions {
    pub threads: usize,
    /// `ModelConfig.whisper_fit_ctx`.
    pub fit_ctx: bool,
    /// Silero VAD model for whisper.cpp's built-in VAD.
    pub vad_model: Option<String>,
}

/// One request (`murmur_whisper_transcribe`'s `req`): the fields of
/// [`AsrRequest`] whisper uses.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Request {
    pub initial_prompt: String,
    pub language: Option<String>,
    pub prior_text: Option<String>,
}

/// A loaded model and where it runs.
pub struct Engine {
    engine: WhisperEngine,
    pub info: Info,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Info {
    pub backend: String,
    pub device: String,
    /// Set when a GPU was chosen but failed and the CPU took over.
    pub gpu_error: Option<String>,
    pub load_ms: u64,
    pub multilingual: bool,
}

impl Engine {
    /// Load on the best device ([`backend::choose`]), then on the CPU when
    /// the GPU fails.
    pub fn load(path: &str, opts: &LoadOptions) -> Result<Self, String> {
        let t0 = Instant::now();
        let plan = backend::choose(&backend::devices(), backend::force_cpu_from_env());
        let mut gpu_error = None;
        for p in plan.fallbacks() {
            let device = match &p {
                backend::Plan::Gpu { gpu_index, .. } => {
                    if let Err(e) = backend::gpu_starts(*gpu_index) {
                        gpu_error = Some(format!("{}: {e}", p.device()));
                        continue;
                    }
                    WhisperDevice::Gpu(*gpu_index)
                }
                backend::Plan::Cpu => WhisperDevice::Cpu,
            };
            match WhisperEngine::load_on(path, opts.threads.max(1), device) {
                Ok(mut e) => {
                    e = e.with_fit_ctx(opts.fit_ctx);
                    if let Some(v) = opts.vad_model.as_deref().filter(|v| !v.is_empty()) {
                        e = e.with_silero_vad(v);
                    }
                    return Ok(Self {
                        info: Info {
                            backend: p.backend().to_string(),
                            device: p.device().to_string(),
                            gpu_error,
                            load_ms: t0.elapsed().as_millis() as u64,
                            multilingual: e.is_multilingual(),
                        },
                        engine: e,
                    });
                }
                Err(e) if p.is_gpu() => gpu_error = Some(format!("{}: {e}", p.device())),
                Err(e) => return Err(e.to_string()),
            }
        }
        Err(gpu_error.unwrap_or_else(|| "no device".into()))
    }

    pub fn transcribe(
        &self,
        pcm: &[f32],
        req: &Request,
        abort: Option<&AtomicBool>,
    ) -> Result<AsrOutput, String> {
        let r = AsrRequest {
            initial_prompt: req.initial_prompt.clone(),
            language: req.language.clone(),
            prior_text: req.prior_text.clone(),
            ..AsrRequest::default()
        };
        self.engine
            .transcribe_abortable(pcm, &r, abort)
            .map_err(|e| e.to_string())
    }
}

fn to_c(s: &str) -> *mut c_char {
    CString::new(s.replace('\0', ""))
        .expect("NULs removed")
        .into_raw()
}

/// # Safety
/// `p` is null or a valid NUL-terminated string.
unsafe fn from_c(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

#[no_mangle]
pub extern "C" fn murmur_whisper_abi_version() -> u32 {
    ABI_VERSION
}

/// The devices this build sees and where a model would load, without
/// loading one. Free with [`murmur_whisper_free_string`].
#[no_mangle]
pub extern "C" fn murmur_whisper_backends() -> *mut c_char {
    let r = std::panic::catch_unwind(|| {
        let devices = backend::devices();
        let plan = backend::choose(&devices, backend::force_cpu_from_env());
        serde_json::json!({
            "devices": devices,
            "backend": plan.backend(),
            "device": plan.device(),
        })
        .to_string()
    });
    to_c(&r.unwrap_or_else(|_| {
        r#"{"devices":[],"backend":"CPU","device":"CPU","error":"panicked"}"#.into()
    }))
}

/// Load a whisper ggml model. Returns null and sets `*err` on failure.
///
/// # Safety
/// String arguments are null or NUL-terminated; `err` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn murmur_whisper_load(
    model: *const c_char,
    opts: *const c_char,
    err: *mut *mut c_char,
) -> *mut Engine {
    let set_err = |e: &str| {
        if !err.is_null() {
            *err = to_c(e);
        }
    };
    let path = from_c(model);
    let opts_s = from_c(opts);
    let opts: LoadOptions = if opts_s.trim().is_empty() {
        LoadOptions::default()
    } else {
        match serde_json::from_str(&opts_s) {
            Ok(o) => o,
            Err(e) => {
                set_err(&format!("options json: {e}"));
                return std::ptr::null_mut();
            }
        }
    };
    match std::panic::catch_unwind(|| Engine::load(&path, &opts)) {
        Ok(Ok(e)) => Box::into_raw(Box::new(e)),
        Ok(Err(e)) => {
            set_err(&e);
            std::ptr::null_mut()
        }
        Err(_) => {
            set_err("whisper load panicked");
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `h` is a live handle from [`murmur_whisper_load`].
#[no_mangle]
pub unsafe extern "C" fn murmur_whisper_info(h: *const Engine) -> *mut c_char {
    match h.as_ref() {
        Some(e) => to_c(&serde_json::to_string(&e.info).expect("info serialises")),
        None => std::ptr::null_mut(),
    }
}

/// # Safety
/// `h` is a live handle from [`murmur_whisper_load`]; `pcm` points at `n`
/// floats (16 kHz mono); `req` is null or NUL-terminated; `cancel` is null
/// or points at a `bool` that stays valid for the call and is only written
/// atomically; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn murmur_whisper_transcribe(
    h: *const Engine,
    pcm: *const f32,
    n: usize,
    req: *const c_char,
    cancel: *const bool,
    out: *mut *mut c_char,
) -> i32 {
    if out.is_null() {
        return ERROR;
    }
    let Some(e) = h.as_ref() else {
        *out = to_c("no engine");
        return ERROR;
    };
    let audio: &[f32] = if pcm.is_null() || n == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(pcm, n)
    };
    let req_s = from_c(req);
    let r: Request = if req_s.trim().is_empty() {
        Request::default()
    } else {
        match serde_json::from_str(&req_s) {
            Ok(r) => r,
            Err(err) => {
                *out = to_c(&format!("request json: {err}"));
                return ERROR;
            }
        }
    };
    // SAFETY: AtomicBool has the same layout as bool.
    let abort = (cancel as *const AtomicBool).as_ref();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.transcribe(audio, &r, abort)
    })) {
        Ok(Ok(o)) => {
            *out = to_c(
                &serde_json::json!({
                    "text": o.text,
                    "language": o.language,
                    "no_speech_prob": o.no_speech_prob,
                })
                .to_string(),
            );
            OK
        }
        Ok(Err(m)) => {
            *out = to_c(&m);
            ERROR
        }
        Err(_) => {
            *out = to_c("whisper transcribe panicked");
            ERROR
        }
    }
}

/// # Safety
/// `s` is null or a string returned by this library, freed once.
#[no_mangle]
pub unsafe extern "C" fn murmur_whisper_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Free the model.
///
/// # Safety
/// `h` is null or a handle from [`murmur_whisper_load`], not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn murmur_whisper_unload(h: *mut Engine) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_model_is_an_error_not_a_panic() {
        let m = CString::new("/nonexistent/ggml.bin").unwrap();
        let mut err: *mut c_char = std::ptr::null_mut();
        // SAFETY: valid strings; err writable.
        let h = unsafe { murmur_whisper_load(m.as_ptr(), std::ptr::null(), &mut err) };
        assert!(h.is_null());
        let e = unsafe { from_c(err) };
        unsafe { murmur_whisper_free_string(err) };
        assert!(e.contains("nonexistent"), "{e}");
    }

    #[test]
    fn bad_options_are_refused() {
        let m = CString::new("x.bin").unwrap();
        let o = CString::new("{not json").unwrap();
        let mut err: *mut c_char = std::ptr::null_mut();
        let h = unsafe { murmur_whisper_load(m.as_ptr(), o.as_ptr(), &mut err) };
        assert!(h.is_null());
        let e = unsafe { from_c(err) };
        unsafe { murmur_whisper_free_string(err) };
        assert!(e.starts_with("options json"), "{e}");
    }

    #[test]
    fn backends_json_names_a_device() {
        let p = murmur_whisper_backends();
        let s = unsafe { from_c(p) };
        unsafe { murmur_whisper_free_string(p) };
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v["devices"].is_array(), "{s}");
        let b = v["backend"].as_str().unwrap();
        if !cfg!(feature = "vulkan") {
            assert_eq!(b, "CPU");
        }
        assert!(!v["device"].as_str().unwrap().is_empty());
    }

    #[test]
    fn null_handle_transcribe_is_an_error() {
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = unsafe {
            murmur_whisper_transcribe(
                std::ptr::null(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
                &mut out,
            )
        };
        assert_eq!(rc, ERROR);
        unsafe { murmur_whisper_free_string(out) };
    }
}
