//! Thin C ABI (+ feature `jni`) over flow-core. Configs and results cross
//! as JSON strings; the handle is an opaque pointer (a `jlong` on Android).
//!
//! Conventions: functions returning `c_int` give 0 = ok, -1 = error (see
//! `flow_last_error`), 1 = ignored (no active session). Returned strings
//! are owned by the caller and must be freed with `flow_string_free`.
//! No function panics across the boundary.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};

use flow_core::config::{FieldContext, ModelConfig, SessionConfig};
use flow_core::FlowCore;

#[cfg(feature = "jni")]
pub mod jni_api;

/// Opaque handle.
pub struct FlowHandle(FlowCore);

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: impl Into<String>) {
    let s = CString::new(msg.into().replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = s);
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        set_error("panic inside flow-core");
        fallback
    })
}

unsafe fn str_arg<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn out_string(s: String) -> *mut c_char {
    CString::new(s.replace('\0', " ")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Shared JSON-level API, reused by the JNI layer.
pub mod json_api {
    use super::*;

    pub fn load_models(cfg_json: &str) -> Result<FlowCore, String> {
        let cfg: ModelConfig =
            serde_json::from_str(cfg_json).map_err(|e| format!("bad model config: {e}"))?;
        FlowCore::load_models(&cfg).map_err(|e| e.to_string())
    }

    pub fn start_session(core: &FlowCore, cfg_json: &str) -> Result<(), String> {
        let cfg: SessionConfig =
            serde_json::from_str(cfg_json).map_err(|e| format!("bad session config: {e}"))?;
        core.start_session(cfg).map_err(|e| e.to_string())
    }

    pub fn set_context(core: &FlowCore, ctx_json: &str) -> Result<(), String> {
        let ctx: FieldContext =
            serde_json::from_str(ctx_json).map_err(|e| format!("bad context: {e}"))?;
        core.set_context(ctx);
        Ok(())
    }

    pub fn commit(core: Option<&FlowCore>) -> String {
        match core {
            Some(c) => serde_json::to_string(&c.commit())
                .unwrap_or_else(|e| error_result(&format!("serialize: {e}"))),
            None => error_result("null handle"),
        }
    }

    /// A `CommitResult` JSON with status ERROR, for failures outside the core.
    pub fn error_result(msg: &str) -> String {
        serde_json::json!({
            "status": "ERROR", "asr_text": "", "final_text": "", "language": null,
            "timings": {"audio_ms": 0, "asr_ms": 0, "llm_ms": 0, "postprocess_ms": 0,
                        "total_ms": 0, "dropped_samples": 0},
            "press_enter": false, "fallback_reason": null, "truncated": false, "error": msg,
        })
        .to_string()
    }

    pub fn learn_edit(core: &FlowCore, pasted: &str, edited: &str) -> String {
        serde_json::to_string(&core.learn_edit(pasted, edited)).unwrap()
    }
}

/// Load models; returns NULL on failure (see `flow_last_error`).
///
/// # Safety
/// `model_config_json` must be NULL or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn flow_load_models(model_config_json: *const c_char) -> *mut FlowHandle {
    guard(std::ptr::null_mut(), || {
        let Some(j) = str_arg(model_config_json) else {
            set_error("bad model config: null or non-UTF-8");
            return std::ptr::null_mut();
        };
        match json_api::load_models(j) {
            Ok(c) => Box::into_raw(Box::new(FlowHandle(c))),
            Err(e) => {
                set_error(e);
                std::ptr::null_mut()
            }
        }
    })
}

/// Free models and the handle. NULL is a no-op.
///
/// # Safety
/// `h` must be NULL or a handle from `flow_load_models`, not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn flow_unload(h: *mut FlowHandle) {
    if !h.is_null() {
        guard((), || drop(Box::from_raw(h)));
    }
}

/// # Safety
/// `h` valid handle or NULL; `session_config_json` valid string or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_start_session(
    h: *mut FlowHandle,
    session_config_json: *const c_char,
) -> c_int {
    guard(-1, || {
        let (Some(h), Some(j)) = (h.as_ref(), str_arg(session_config_json)) else {
            set_error("null handle or config");
            return -1;
        };
        json_api::start_session(&h.0, j).map_or_else(
            |e| {
                set_error(e);
                -1
            },
            |_| 0,
        )
    })
}

/// Non-blocking copy of PCM16 16 kHz mono samples into the ring buffer.
///
/// # Safety
/// `pcm` must point to `len` readable samples (may be NULL when len == 0).
#[no_mangle]
pub unsafe extern "C" fn flow_push_audio(h: *mut FlowHandle, pcm: *const i16, len: usize) -> c_int {
    guard(-1, || {
        let Some(h) = h.as_ref() else { return -1 };
        let s = if len == 0 || pcm.is_null() {
            &[][..]
        } else {
            std::slice::from_raw_parts(pcm, len)
        };
        if h.0.push_audio(s) {
            0
        } else {
            1
        }
    })
}

/// # Safety
/// As `flow_start_session`.
#[no_mangle]
pub unsafe extern "C" fn flow_set_context(
    h: *mut FlowHandle,
    context_json: *const c_char,
) -> c_int {
    guard(-1, || {
        let (Some(h), Some(j)) = (h.as_ref(), str_arg(context_json)) else {
            return -1;
        };
        json_api::set_context(&h.0, j).map_or_else(
            |e| {
                set_error(e);
                -1
            },
            |_| 0,
        )
    })
}

/// Latest partial transcript, or NULL.
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_partial(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        match h.as_ref().and_then(|h| h.0.partial()) {
            Some(p) => out_string(p),
            None => std::ptr::null_mut(),
        }
    })
}

/// Blocking: finish the session and return the CommitResult JSON.
///
/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_commit(h: *mut FlowHandle) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        out_string(json_api::commit(h.as_ref().map(|h| &h.0)))
    })
}

/// # Safety
/// `h` valid handle or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_cancel(h: *mut FlowHandle) {
    if let Some(h) = h.as_ref() {
        guard((), || h.0.cancel());
    }
}

/// JSON array of `{original, corrected}` dictionary candidates.
///
/// # Safety
/// `h` valid handle; strings valid or NULL.
#[no_mangle]
pub unsafe extern "C" fn flow_learn_edit(
    h: *mut FlowHandle,
    pasted: *const c_char,
    edited: *const c_char,
) -> *mut c_char {
    guard(std::ptr::null_mut(), || {
        let (Some(h), Some(p), Some(e)) = (h.as_ref(), str_arg(pasted), str_arg(edited)) else {
            return std::ptr::null_mut();
        };
        out_string(json_api::learn_edit(&h.0, p, e))
    })
}

/// Last error on this thread ("" if none). Valid until the next call on it.
#[no_mangle]
pub extern "C" fn flow_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// # Safety
/// `s` must be NULL or a string returned by this library.
#[no_mangle]
pub unsafe extern "C" fn flow_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}
