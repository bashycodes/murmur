//! JNI exports for the Android shell (feature `jni`).
//!
//! Kotlin side (provisional; align with apps/flow-android/JNI_CONTRACT.md):
//! ```kotlin
//! package com.flowclone.core
//! object NativeFlowCore {
//!     external fun loadModels(modelConfigJson: String): Long      // 0 = failed, see lastError()
//!     external fun unload(handle: Long)
//!     external fun startSession(handle: Long, sessionConfigJson: String): Boolean
//!     external fun pushAudio(handle: Long, pcm: ShortArray, len: Int): Boolean  // non-blocking
//!     external fun setContext(handle: Long, fieldContextJson: String): Boolean
//!     external fun partial(handle: Long): String?
//!     external fun commit(handle: Long): String                    // CommitResult JSON
//!     external fun cancel(handle: Long)
//!     external fun learnEdit(handle: Long, pasted: String, edited: String): String
//!     external fun lastError(): String
//! }
//! ```
//! Renaming the class/package only means editing the `Java_..._` prefix.

use std::cell::RefCell;

use jni::objects::{JClass, JShortArray, JString};
use jni::sys::{jboolean, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;

use crate::{json_api, set_error, FlowHandle, LAST_ERROR};

thread_local! {
    // Reused per AudioRecord thread so pushAudio never allocates.
    static PCM: RefCell<Vec<i16>> = const { RefCell::new(Vec::new()) };
}

fn handle<'a>(h: jlong) -> Option<&'a FlowHandle> {
    // SAFETY: the shell only passes values returned by loadModels and not yet unloaded.
    unsafe { (h as *const FlowHandle).as_ref() }
}

fn jstr(env: &mut JNIEnv, s: &JString) -> Option<String> {
    env.get_string(s).ok().map(Into::into)
}

fn out(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map_or(std::ptr::null_mut(), |s| s.into_raw())
}

fn ok(b: bool) -> jboolean {
    if b {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_loadModels(
    mut env: JNIEnv,
    _c: JClass,
    cfg: JString,
) -> jlong {
    let Some(j) = jstr(&mut env, &cfg) else {
        return 0;
    };
    match json_api::load_models(&j) {
        Ok(c) => Box::into_raw(Box::new(FlowHandle(c))) as jlong,
        Err(e) => {
            set_error(e);
            0
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_unload(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if h != 0 {
        // SAFETY: see `handle`.
        drop(unsafe { Box::from_raw(h as *mut FlowHandle) });
    }
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_startSession(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    cfg: JString,
) -> jboolean {
    let (Some(h), Some(j)) = (handle(h), jstr(&mut env, &cfg)) else {
        return JNI_FALSE;
    };
    ok(json_api::start_session(&h.0, &j).map_err(set_error).is_ok())
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_pushAudio(
    env: JNIEnv,
    _c: JClass,
    h: jlong,
    pcm: JShortArray,
    len: jint,
) -> jboolean {
    let Some(h) = handle(h) else { return JNI_FALSE };
    PCM.with(|b| {
        let mut b = b.borrow_mut();
        b.resize(len.max(0) as usize, 0);
        if env.get_short_array_region(&pcm, 0, &mut b).is_err() {
            return JNI_FALSE;
        }
        ok(h.0.push_audio(&b))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_setContext(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    ctx: JString,
) -> jboolean {
    let (Some(h), Some(j)) = (handle(h), jstr(&mut env, &ctx)) else {
        return JNI_FALSE;
    };
    ok(json_api::set_context(&h.0, &j).map_err(set_error).is_ok())
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_partial(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    match handle(h).and_then(|h| h.0.partial()) {
        Some(p) => out(&mut env, &p),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_commit(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = json_api::commit(handle(h).map(|h| &h.0));
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_cancel(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        h.0.cancel();
    }
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_learnEdit(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    pasted: JString,
    edited: JString,
) -> jstring {
    let (Some(h), Some(p), Some(e)) = (handle(h), jstr(&mut env, &pasted), jstr(&mut env, &edited))
    else {
        return out(&mut env, "[]");
    };
    let r = json_api::learn_edit(&h.0, &p, &e);
    out(&mut env, &r)
}

#[no_mangle]
pub extern "system" fn Java_com_flowclone_core_NativeFlowCore_lastError(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    let e = LAST_ERROR.with(|e| e.borrow().to_string_lossy().into_owned());
    out(&mut env, &e)
}
