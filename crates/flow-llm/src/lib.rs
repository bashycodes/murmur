//! flow-llm: the llama.cpp cleanup engine behind flow-core's `llm` feature.
//!
//! flow-core never links this crate. It loads the cdylib at runtime
//! (`libflow_llm.so` / `flow_llm.dll` / `libflow_llm.dylib`) and calls the
//! C ABI below, because llama.cpp's ggml and whisper.cpp's ggml cannot be
//! linked into the same binary (duplicate symbols). All strings are UTF-8,
//! NUL-terminated. Strings returned by the plugin are freed with
//! [`flow_llm_free_string`].
//!
//! ABI version 1:
//! ```c
//! uint32_t flow_llm_abi_version(void);
//! void*   flow_llm_load(const char* model, int32_t threads,
//!                       const char* warm_messages /* nullable */, char** err);
//! int32_t flow_llm_clean(void* h, const char* messages, const char* transcript,
//!                        uint64_t budget_ms,
//!                        const bool* cancel /* nullable, read atomically */, char** out);
//!         // 0 = ok (*out = text), 1 = cancelled/deadline, 2 = error (*out = message)
//! // messages = JSON [["system","…"],["user","…"],["assistant","…"],…,["user","…"]]
//! char*   flow_llm_info(void* h);   // JSON
//! // Optional (added 2026-10, still ABI 1; loaders look them up lazily):
//! int32_t flow_llm_prefill(void* h, const char* messages); // non-blocking, 0 ok / 2 bad json
//! void    flow_llm_prefill_cancel(void* h);
//! void    flow_llm_free_string(char*);
//! void    flow_llm_unload(void* h);
//! ```

pub mod engine;

use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

pub use engine::{AheadStats, CleanErr, Engine, Info, Prompt, Stats};

pub const ABI_VERSION: u32 = 1;

pub const CLEAN_OK: i32 = 0;
pub const CLEAN_CANCELLED: i32 = 1;
pub const CLEAN_ERROR: i32 = 2;

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
pub extern "C" fn flow_llm_abi_version() -> u32 {
    ABI_VERSION
}

fn parse_messages(json: &str) -> Result<Vec<(String, String)>, String> {
    serde_json::from_str(json).map_err(|e| format!("messages json: {e}"))
}

/// Load a GGUF model and prefill the warm prompt (null = no warm-up).
/// Returns null and sets `*err` on failure.
///
/// # Safety
/// String arguments are null or NUL-terminated; `err` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn flow_llm_load(
    model: *const c_char,
    threads: i32,
    warm_messages: *const c_char,
    err: *mut *mut c_char,
) -> *mut Engine {
    let path = from_c(model);
    let warm = if warm_messages.is_null() {
        None
    } else {
        match parse_messages(&from_c(warm_messages)) {
            Ok(messages) => Some(Prompt {
                messages,
                transcript: String::new(),
            }),
            Err(e) => {
                if !err.is_null() {
                    *err = to_c(&e);
                }
                return std::ptr::null_mut();
            }
        }
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Engine::load(&path, threads.max(1) as usize, warm)
    }));
    match r {
        Ok(Ok(e)) => Box::into_raw(Box::new(e)),
        Ok(Err(e)) => {
            if !err.is_null() {
                *err = to_c(&e);
            }
            std::ptr::null_mut()
        }
        Err(_) => {
            if !err.is_null() {
                *err = to_c("llm load panicked");
            }
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// `h` is a live handle from [`flow_llm_load`]; strings are null or
/// NUL-terminated; `cancel` is null or points at a `bool` that stays valid
/// for the call and is only written atomically; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn flow_llm_clean(
    h: *const Engine,
    messages: *const c_char,
    transcript: *const c_char,
    budget_ms: u64,
    cancel: *const bool,
    out: *mut *mut c_char,
) -> i32 {
    if h.is_null() || out.is_null() {
        return CLEAN_ERROR;
    }
    let messages = match parse_messages(&from_c(messages)) {
        Ok(m) => m,
        Err(e) => {
            *out = to_c(&e);
            return CLEAN_ERROR;
        }
    };
    let p = Prompt {
        messages,
        transcript: from_c(transcript),
    };
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    // SAFETY: AtomicBool has the same layout as bool.
    let cancel = (cancel as *const AtomicBool).as_ref();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (*h).clean(&p, deadline, cancel)
    }));
    match r {
        Ok(Ok(s)) => {
            *out = to_c(&s);
            CLEAN_OK
        }
        Ok(Err(CleanErr::Cancelled)) => CLEAN_CANCELLED,
        Ok(Err(CleanErr::Backend(e))) => {
            *out = to_c(&e);
            CLEAN_ERROR
        }
        Err(_) => {
            *out = to_c("llm clean panicked");
            CLEAN_ERROR
        }
    }
}

/// Non-blocking prefill-ahead of `messages` (same JSON as
/// [`flow_llm_clean`]). 0 = queued, 2 = bad arguments.
///
/// # Safety
/// `h` is a live handle from [`flow_llm_load`]; `messages` is null or
/// NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn flow_llm_prefill(h: *const Engine, messages: *const c_char) -> i32 {
    let Some(e) = h.as_ref() else {
        return CLEAN_ERROR;
    };
    let Ok(messages) = parse_messages(&from_c(messages)) else {
        return CLEAN_ERROR;
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.prefill(Prompt {
            messages,
            transcript: String::new(),
        })
    }));
    if r.is_ok() {
        CLEAN_OK
    } else {
        CLEAN_ERROR
    }
}

/// Stop a running prefill-ahead.
///
/// # Safety
/// `h` is null or a live handle from [`flow_llm_load`].
#[no_mangle]
pub unsafe extern "C" fn flow_llm_prefill_cancel(h: *const Engine) {
    if let Some(e) = h.as_ref() {
        e.cancel_prefill();
    }
}

/// # Safety
/// `h` is a live handle from [`flow_llm_load`].
#[no_mangle]
pub unsafe extern "C" fn flow_llm_info(h: *const Engine) -> *mut c_char {
    let Some(e) = h.as_ref() else {
        return std::ptr::null_mut();
    };
    let i = &e.info;
    let st = e.last_stats();
    let ah = e.ahead_stats();
    to_c(
        &serde_json::json!({
            "n_params": i.n_params,
            "chat_template": i.chat_template,
            "warm_prefill_tokens": i.warm_prefill_tokens,
            "warm_ms": i.warm_ms,
            "load_ms": i.load_ms,
            "last": {
                "prompt_tokens": st.prompt_tokens,
                "prefill_tokens": st.prefill_tokens,
                "prefill_ms": st.prefill_ms,
                "gen_tokens": st.gen_tokens,
                "gen_ms": st.gen_ms,
                "gen_steps": st.gen_steps,
                "draft_accepted": st.draft_accepted,
            },
            "ahead": {
                "requests": ah.requests,
                "completed": ah.completed,
                "preempted": ah.preempted,
                "tokens": ah.tokens,
                "ms": ah.ms,
            },
        })
        .to_string(),
    )
}

/// # Safety
/// `s` is null or a string returned by this library, freed once.
#[no_mangle]
pub unsafe extern "C" fn flow_llm_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Free the context and the model.
///
/// # Safety
/// `h` is null or a handle from [`flow_llm_load`], not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn flow_llm_unload(h: *mut Engine) {
    if !h.is_null() {
        drop(Box::from_raw(h));
    }
}
