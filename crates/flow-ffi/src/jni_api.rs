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
    // D26: product version + commit (`flow-ffi 0.2.0+abc1234`); the .so is built
    // separately from the APK, so its own build is what pins it.
    let v = format!("flow-ffi {}", flow_build_info::FULL);
    guard(std::ptr::null_mut(), || out(&mut env, &v))
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
            Ok(c) => {
                // Like the C ABI: a success leaves no stale error from an earlier failed load.
                clear_error();
                Box::into_raw(Box::new(FlowHandle::Core(c))) as jlong
            }
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

/// `nativeStartSession` returning the `StartInfo` JSON, or `null` with the
/// last error / kind set (2026-10-22, additive).
#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeStartSessionInfo(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    cfg: JString,
) -> jstring {
    let r = guard(None, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return None;
        };
        let j = jstr(&mut env, &cfg, "bad session config")?;
        json_api::start_session_info(h, &j)
            .map_err(|(kind, e)| set_error_kind(kind, e))
            .ok()
    });
    match r {
        Some(s) => guard(std::ptr::null_mut(), || out(&mut env, &s)),
        None => std::ptr::null_mut(),
    }
}

/// Non-blocking `RouterPoll` JSON (partial + endpoint + live route). Never
/// null: a `0` handle gives the idle poll (2026-10-22, additive).
#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativePoll(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard(json_api::idle_poll(), || json_api::poll(handle(h)));
    guard(std::ptr::null_mut(), || out(&mut env, &r))
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
        // A failed length query leaves a Java exception pending: clear it, push nothing.
        let cap = match env.get_array_length(&pcm) {
            Ok(n) => n.max(0),
            Err(_) => {
                clear(&mut env);
                return JNI_FALSE;
            }
        };
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

// ---- D24 never-fail fallback (JNI_CONTRACT.md "On-demand local engine") --

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetLocalLoader(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    configs: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let Some(j) = jstr(&mut env, &configs, "bad_config: model configs") else {
            return JNI_FALSE;
        };
        jbool(
            json_api::set_local_loader(h, &j)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativePrewarm(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jboolean {
    guard(JNI_FALSE, || {
        jbool(handle(h).is_some_and(json_api::prewarm))
    })
}

// ---- D18 cloud STT (JNI_CONTRACT.md "Cloud") -----------------------------
// Keys arrive only through nativeSetCloudKey / nativeProbeCloud and are moved
// into the router's SecretString; nothing here logs or returns them.

/// A nullable Java string (`null` → `None`; conversion failure → `None`).
fn opt_jstr(env: &mut JNIEnv, s: &JString) -> Option<String> {
    if s.is_null() {
        return None;
    }
    match env.get_string(s) {
        Ok(s) => Some(s.into()),
        Err(_) => {
            clear(env);
            None
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudKey(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    provider_id: JString,
    key: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let Some(id) = jstr(&mut env, &provider_id, "cloud provider id") else {
            return JNI_FALSE;
        };
        let key = opt_jstr(&mut env, &key);
        jbool(
            json_api::set_cloud_key(h, &id, key)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

/// `nativeSetCloudKey` bound to a provider config (host + adapter), review S4.
#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudKeyFor(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    provider_json: JString,
    key: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let Some(p) = jstr(
            &mut env,
            &provider_json,
            "bad_config: cloud provider config",
        ) else {
            return JNI_FALSE;
        };
        let key = opt_jstr(&mut env, &key);
        jbool(
            json_api::set_cloud_key_for(h, &p, key)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeProbeCloud(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    provider_json: JString,
    key: JString,
) -> jstring {
    let args = guard(None, || {
        Some((
            jstr(&mut env, &provider_json, "cloud provider config"),
            opt_jstr(&mut env, &key),
        ))
    });
    let r = guard(
        r#"{"ok":false,"engine":"cloud","error":"error","message":"panic in probe"}"#.to_owned(),
        || {
            let (p, k) = args.unwrap_or((None, None));
            json_api::probe_cloud(handle(h), p.as_deref().unwrap_or(""), k)
        },
    );
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCloudUsage(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard("{}".to_owned(), || {
        handle(h).map_or_else(|| "{}".to_owned(), json_api::cloud_usage)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudUsage(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    usage: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let Some(j) = jstr(&mut env, &usage, "bad_config: cloud usage") else {
            return JNI_FALSE;
        };
        jbool(
            json_api::set_cloud_usage(h, &j)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeResetCloudUsage(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        guard((), || json_api::reset_cloud_usage(h));
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCloudCapabilities(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    let r = guard(
        r#"{"cloud":false,"adapters":[]}"#.to_owned(),
        json_api::cloud_capabilities,
    );
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCloudPresets(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    let r = guard("[]".to_owned(), json_api::cloud_presets);
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

// ---- D28 OpenRouter-only cloud (JNI_CONTRACT.md "OpenRouter") -------------

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeMigrateCloud(
    mut env: JNIEnv,
    _c: JClass,
    dev_json: JString,
) -> jstring {
    let j = guard(None, || {
        jstr(&mut env, &dev_json, "bad_config: router config")
    });
    let r = guard(None, || {
        let j = j?;
        json_api::migrate_cloud(&j)
            .map_err(|(kind, e)| set_error_kind(kind, e))
            .ok()
    });
    match r {
        Some(s) => guard(std::ptr::null_mut(), || out(&mut env, &s)),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetCleanup(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    cleanup_json: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let Some(j) = jstr(&mut env, &cleanup_json, "bad_config: cleanup config") else {
            return JNI_FALSE;
        };
        jbool(
            json_api::set_cleanup(h, &j)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCleanupStatus(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard("{}".to_owned(), || {
        handle(h).map_or_else(|| "{}".to_owned(), json_api::cleanup_status)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCloudNotice(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    match guard(None, || handle(h).and_then(json_api::cloud_notice)) {
        Some(n) => guard(std::ptr::null_mut(), || out(&mut env, &n)),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeAckCloudNotice(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        guard((), || json_api::ack_cloud_notice(h));
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudModel(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    use_: JString,
    model: JString,
) -> jboolean {
    guard(JNI_FALSE, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return JNI_FALSE;
        };
        let (Some(u), Some(m)) = (
            jstr(&mut env, &use_, "bad_config: use"),
            jstr(&mut env, &model, "bad_config: model"),
        ) else {
            return JNI_FALSE;
        };
        jbool(
            json_api::set_cloud_model(h, &u, &m)
                .map_err(|(kind, e)| set_error_kind(kind, e))
                .is_ok(),
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeSetCloudCacheDir(
    mut env: JNIEnv,
    _c: JClass,
    dir: JString,
) {
    guard((), || {
        if let Some(d) = jstr(&mut env, &dir, "cache dir") {
            json_api::set_cloud_cache_dir(&d);
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCloudCatalog(
    mut env: JNIEnv,
    _c: JClass,
    _h: jlong,
    use_: JString,
    query: JString,
    filters_json: JString,
) -> jstring {
    let args = guard(None, || {
        Some((
            opt_jstr(&mut env, &use_).unwrap_or_default(),
            opt_jstr(&mut env, &query).unwrap_or_default(),
            opt_jstr(&mut env, &filters_json).unwrap_or_else(|| "{}".into()),
        ))
    });
    let r = guard("[]".to_owned(), || {
        let (u, q, f) = args.unwrap_or_default();
        json_api::cloud_catalog(&u, &q, &f)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeRefreshCloudCatalog(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    force: jboolean,
) -> jstring {
    let r = guard(r#"{"error":"error"}"#.to_owned(), || {
        json_api::refresh_cloud_catalog(handle(h), force != JNI_FALSE)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_core_NativeFlowCore_nativeCloudKeyInfo(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard(r#"{"error":"error"}"#.to_owned(), || {
        handle(h).map_or_else(
            || r#"{"error":"cloud_not_configured"}"#.to_owned(),
            json_api::cloud_key_info,
        )
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}
