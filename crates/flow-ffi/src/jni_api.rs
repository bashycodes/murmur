//! JNI exports for the Android shell (feature `jni`).
//!
//! The contract is `apps/flow-android/JNI_CONTRACT.md`: static natives on
//! `dev.flowclone.core.NativeFlowCore` (Kotlin `companion object` +
//! `@JvmStatic external`), so the second C parameter is a `jclass`.
//!
//! Rules the contract imposes, and how they are met here:
//! * no panic unwinds into the JVM: every body runs inside [`guard`];
//! * no Java exception is left pending: any JNI call that fails has its
//!   exception cleared ([`clear`]) and the failure becomes a sentinel +
//!   `nativeLastError`;
//! * `nativePushAudio` never blocks or allocates in steady state: PCM is copied
//!   into a per-thread reused buffer, then into the core's ring.

use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};

use jni::objects::{JClass, JShortArray, JString};
use jni::sys::{jboolean, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::{
    clear_error, json_api, set_error, set_error_kind, FlowHandle, LAST_ERROR, LAST_ERROR_KIND,
};

thread_local! {
    // Reused per AudioRecord thread so pushAudio never allocates after the first call.
    static PCM: RefCell<Vec<i16>> = const { RefCell::new(Vec::new()) };
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        set_error("panic inside flow-core");
        fallback
    })
}

/// Drop any Java exception a failed JNI call left pending; Kotlin does not
/// expect exceptions from these methods.
fn clear(env: &mut JNIEnv) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
}

fn handle<'a>(h: jlong) -> Option<&'a FlowHandle> {
    // SAFETY: the shell only passes values returned by nativeLoadModels and not
    // yet passed to nativeUnload (it serialises unload against other calls).
    unsafe { (h as *const FlowHandle).as_ref() }
}

fn jstr(env: &mut JNIEnv, s: &JString, what: &str) -> Option<String> {
    if s.is_null() {
        set_error(format!("{what}: null string"));
        return None;
    }
    match env.get_string(s) {
        Ok(s) => Some(s.into()),
        Err(e) => {
            clear(env);
            set_error(format!("{what}: {e}"));
            None
        }
    }
}

fn out(env: &mut JNIEnv, s: &str) -> jstring {
    match env.new_string(s) {
        Ok(s) => s.into_raw(),
        Err(_) => {
            clear(env);
            std::ptr::null_mut()
        }
    }
}

fn jbool(b: bool) -> jboolean {
    if b {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeVersion(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    let v = concat!("flow-ffi ", env!("CARGO_PKG_VERSION"));
    guard(std::ptr::null_mut(), || out(&mut env, v))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeLastError(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        let e = LAST_ERROR.with(|e| e.borrow().to_string_lossy().into_owned());
        if e.is_empty() {
            std::ptr::null_mut()
        } else {
            out(&mut env, &e)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeLoadModels(
    mut env: JNIEnv,
    _c: JClass,
    cfg: JString,
) -> jlong {
    guard(0, || {
        let Some(j) = jstr(&mut env, &cfg, "bad model config") else {
            return 0;
        };
        match json_api::load_models_kind(&j) {
            Ok(c) => Box::into_raw(Box::new(FlowHandle::Core(c))) as jlong,
            // The message keeps a stable prefix per kind ("model not found:
            // <path>", "llm plugin not found (…)") for nativeLastError.
            Err((kind, e)) => {
                set_error_kind(kind, e);
                0
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeUnload(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if h != 0 {
        // SAFETY: see `handle`; Kotlin never uses `h` after this call.
        guard((), || drop(unsafe { Box::from_raw(h as *mut FlowHandle) }));
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeStartSession(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    cfg: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let Some(j) = jstr(&mut env, &cfg, "bad session config") else {
            return JNI_FALSE;
        };
        jbool(
            json_api::start_session(h, &j)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativePushAudio(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    pcm: JShortArray,
    len: jint,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else { return JNI_FALSE };
        if pcm.is_null() {
            return JNI_FALSE;
        }
        // Clamp to the array so a bad `length` cannot raise
        // ArrayIndexOutOfBoundsException inside the AudioRecord thread.
        let cap = env.get_array_length(&pcm).unwrap_or(0).max(0);
        let n = len.clamp(0, cap) as usize;
        PCM.with(|b| {
            let mut b = b.borrow_mut();
            b.resize(n, 0);
            if env.get_short_array_region(&pcm, 0, &mut b).is_err() {
                clear(&mut env);
                return JNI_FALSE;
            }
            jbool(h.push_audio(&b))
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetContext(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    ctx: JString,
) {
    guard((), || {
        let Some(h) = handle(h) else { return };
        if let Some(j) = jstr(&mut env, &ctx, "bad context") {
            // Bad JSON is ignored; the message stays in nativeLastError.
            let _ = json_api::set_context(h, &j).map_err(set_error);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativePartial(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        match handle(h).and_then(|h| h.partial()) {
            Some(p) => out(&mut env, &p),
            None => std::ptr::null_mut(),
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCommit(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    // Never fails at the JNI level: a panic becomes an ERROR CommitResult.
    let r = catch_unwind(AssertUnwindSafe(|| json_api::commit(handle(h)))).unwrap_or_else(|_| {
        set_error("panic inside flow-core");
        json_api::error_result("panic inside flow-core")
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCancel(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        guard((), || h.cancel());
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeLearnEdit(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    pasted: JString,
    edited: JString,
) -> jstring {
    let r = guard("[]".to_owned(), || {
        let Some(h) = handle(h) else {
            return "[]".to_owned();
        };
        match (
            jstr(&mut env, &pasted, "learn_edit pasted"),
            jstr(&mut env, &edited, "learn_edit edited"),
        ) {
            (Some(p), Some(e)) => json_api::learn_edit(h, &p, &e),
            _ => "[]".to_owned(),
        }
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

// ---- D17 router (JNI_CONTRACT.md "Router") -------------------------------
// The handle returned by nativeOpenRouter works with every session symbol above.

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeLastErrorKind(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        let k = LAST_ERROR_KIND.with(|e| e.borrow().to_string_lossy().into_owned());
        if k.is_empty() {
            std::ptr::null_mut()
        } else {
            out(&mut env, &k)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCapabilities(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    let r = guard("{}".to_owned(), json_api::capabilities);
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeOpenRouter(
    mut env: JNIEnv,
    _c: JClass,
    cfg: JString,
    local: jlong,
) -> jlong {
    guard(0, || {
        // Ownership of `local` moves here whatever happens next (contract).
        let local = if local == 0 {
            None
        } else {
            // SAFETY: a value from nativeLoadModels that Kotlin released to us.
            match *unsafe { Box::from_raw(local as *mut FlowHandle) } {
                FlowHandle::Core(c) => Some(c),
                FlowHandle::Router(_) => {
                    set_error_kind("bad_config", "bad_config: local handle is a router");
                    return 0;
                }
            }
        };
        let Some(j) = jstr(&mut env, &cfg, "bad_config: router config") else {
            return 0;
        };
        match json_api::open_router(&j, local) {
            Ok(h) => {
                clear_error();
                Box::into_raw(Box::new(h)) as jlong
            }
            Err((kind, e)) => {
                set_error_kind(kind, e);
                0
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeUpdateRouter(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    cfg: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let Some(j) = jstr(&mut env, &cfg, "bad_config: router config") else {
            return JNI_FALSE;
        };
        jbool(
            json_api::update_router(h, &j)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeProbeServer(
    mut env: JNIEnv,
    _c: JClass,
    cfg: JString,
) -> jstring {
    let j = guard(None, || jstr(&mut env, &cfg, "server config"));
    let r = guard(
        r#"{"ok":false,"engine":"remote","error":"error","message":"panic in probe"}"#.to_owned(),
        || json_api::probe_server(j.as_deref().unwrap_or("")),
    );
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeProbeRouter(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard(
        r#"{"ok":false,"engine":"remote","error":"error","message":"panic in probe"}"#.to_owned(),
        || match handle(h) {
            Some(h) => json_api::probe_router(h),
            None => r#"{"ok":false,"engine":"remote","error":"error","message":"null handle"}"#
                .to_owned(),
        },
    );
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeRouteStatus(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard("{}".to_owned(), || {
        handle(h).map_or_else(|| "{}".to_owned(), json_api::route_status)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeResetCircuit(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(r) = handle(h).and_then(FlowHandle::router) {
        guard((), || r.reset_circuit());
    }
}
