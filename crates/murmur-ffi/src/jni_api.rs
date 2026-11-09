//! JNI exports for the Android shell (feature `jni`).
//!
//! The contract is `apps/murmur-android/JNI_CONTRACT.md`: static natives on
//! `dev.murmur.core.NativeMurmurCore` (Kotlin `companion object` +
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

use murmur_core::command::CommandError;

use crate::{
    clear_error, json_api, set_error, set_error_kind, MurmurHandle, LAST_ERROR, LAST_ERROR_KIND,
};

thread_local! {
    // Reused per AudioRecord thread so pushAudio never allocates after the first call.
    static PCM: RefCell<Vec<i16>> = const { RefCell::new(Vec::new()) };
    // Load 3: `LoadTimings` JSON of this thread's last successful nativeLoadModels.
    static LOAD_TIMINGS: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        set_error("panic inside murmur-core");
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

fn handle<'a>(h: jlong) -> Option<&'a MurmurHandle> {
    // SAFETY: the shell only passes values returned by nativeLoadModels and not
    // yet passed to nativeUnload (it serialises unload against other calls).
    unsafe { (h as *const MurmurHandle).as_ref() }
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeVersion(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    // D26: product version + commit (`murmur-ffi 0.2.0+abc1234`); the .so is built
    // separately from the APK, so its own build is what pins it.
    let v = format!("murmur-ffi {}", murmur_build_info::FULL);
    guard(std::ptr::null_mut(), || out(&mut env, &v))
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeLastError(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeLoadModels(
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
                let t = c.load_timings().to_json();
                LOAD_TIMINGS.with(|l| *l.borrow_mut() = Some(t));
                Box::into_raw(Box::new(MurmurHandle::Core(c))) as jlong
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

/// Load 3: how long this thread's last successful `nativeLoadModels` took,
/// per model and phase (`murmur_core::load_timings::LoadTimings` JSON); null
/// before the first one.
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeLastLoadTimings(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        match LOAD_TIMINGS.with(|l| l.borrow().clone()) {
            Some(t) => out(&mut env, &t),
            None => std::ptr::null_mut(),
        }
    })
}

/// Load 3: hold handle `h`'s own on-device LLM (a core's, or a router's
/// warm local engine's) for the next load naming the same model
/// (`MurmurCore::keep_llm`); whether there was one. Call it before
/// `nativeUnload(h)` for a language switch.
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeKeepLlm(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jboolean {
    guard(JNI_FALSE, || {
        jbool(handle(h).is_some_and(MurmurHandle::keep_llm))
    })
}

/// Load 3: free an LLM `nativeKeepLlm` holds (`llm_keep::release`).
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeReleaseKeptLlm(
    _env: JNIEnv,
    _c: JClass,
) {
    guard((), murmur_core::llm_keep::release)
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeUnload(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if h != 0 {
        // SAFETY: see `handle`; Kotlin never uses `h` after this call.
        guard((), || {
            drop(unsafe { Box::from_raw(h as *mut MurmurHandle) })
        });
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeStartSession(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeStartSessionInfo(
    env: JNIEnv,
    _c: JClass,
    h: jlong,
    cfg: JString,
) -> jstring {
    start_info(env, h, cfg, false)
}

/// D55: `nativeStartSessionInfo` on the on-device engine only (a router
/// handle never uses the server or the cloud for this session; a core handle
/// is on-device already). Same result and errors (additive, 2026-11-05).
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeStartSessionLocal(
    env: JNIEnv,
    _c: JClass,
    h: jlong,
    cfg: JString,
) -> jstring {
    start_info(env, h, cfg, true)
}

/// The two start exports: `StartInfo` JSON, or null with the error set.
fn start_info(mut env: JNIEnv, h: jlong, cfg: JString, local_only: bool) -> jstring {
    let r = guard(None, || {
        let Some(h) = handle(h) else {
            set_error("null handle");
            return None;
        };
        let j = jstr(&mut env, &cfg, "bad session config")?;
        json_api::start_session_info(h, &j, local_only)
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativePoll(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard(json_api::idle_poll(), || json_api::poll(handle(h)));
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativePushAudio(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetContext(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativePartial(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCommit(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    // Never fails at the JNI level: a panic becomes an ERROR CommitResult.
    let r = catch_unwind(AssertUnwindSafe(|| json_api::commit(handle(h)))).unwrap_or_else(|_| {
        set_error("panic inside murmur-core");
        json_api::error_result("panic inside murmur-core")
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCancel(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        guard((), || h.cancel());
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeLearnEdit(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeLastErrorKind(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCapabilities(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    let r = guard("{}".to_owned(), json_api::capabilities);
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeOpenRouter(
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
            match *unsafe { Box::from_raw(local as *mut MurmurHandle) } {
                MurmurHandle::Core(c) => Some(c),
                MurmurHandle::Router(_) => {
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeUpdateRouter(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeProbeServer(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeProbeRouter(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeRouteStatus(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeResetCircuit(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(r) = handle(h).and_then(MurmurHandle::router) {
        guard((), || r.reset_circuit());
    }
}

// ---- D24 never-fail fallback (JNI_CONTRACT.md "On-demand local engine") --

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetLocalLoader(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativePrewarm(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetCloudKey(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetCloudKeyFor(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeProbeCloud(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCloudUsage(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetCloudUsage(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeResetCloudUsage(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        guard((), || json_api::reset_cloud_usage(h));
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCloudCapabilities(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCloudPresets(
    mut env: JNIEnv,
    _c: JClass,
) -> jstring {
    let r = guard("[]".to_owned(), json_api::cloud_presets);
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

// ---- D28 OpenRouter-only cloud (JNI_CONTRACT.md "OpenRouter") -------------

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMigrateCloud(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetCleanup(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCleanupStatus(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
) -> jstring {
    let r = guard("{}".to_owned(), || {
        handle(h).map_or_else(|| "{}".to_owned(), json_api::cleanup_status)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

/// Command Mode (JNI_CONTRACT.md "Command Mode"): blocking, off Main.
/// `selected` null = generate at the cursor. Never null: failures are
/// `{"ok":false,"kind":…,"error":…}` JSON.
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCommand(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    instruction: JString,
    selected: JString,
) -> jstring {
    let r = guard(
        json_api::command_json(Err(CommandError::Backend(
            "panic inside murmur-core".into(),
        ))),
        || {
            let Some(h) = handle(h) else {
                return json_api::command_json(Err(CommandError::Backend("null handle".into())));
            };
            let Some(i) = jstr(&mut env, &instruction, "command instruction") else {
                return json_api::command_json(Err(CommandError::NoInstruction));
            };
            let sel = if selected.is_null() {
                None
            } else {
                match jstr(&mut env, &selected, "command selection") {
                    Some(s) => Some(s),
                    None => {
                        return json_api::command_json(Err(CommandError::Backend(
                            "bad selection string".into(),
                        )))
                    }
                }
            };
            json_api::command(h, &i, sel.as_deref())
        },
    );
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

/// Benchmark hook (`dev.murmur.debug.BenchActivity`): one AI-cleanup
/// request on a core handle's on-device LLM ([`crate::bench`]). Blocking.
/// Never null: the result JSON, or `{"error":…}`.
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeBenchClean(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    request: JString,
) -> jstring {
    let err = |m: &str| serde_json::json!({ "error": m }).to_string();
    let r = guard(err("panic inside murmur-core"), || {
        let Some(h) = handle(h) else {
            return err("null handle");
        };
        let Some(req) = jstr(&mut env, &request, "bench request") else {
            return err("bad bench request string");
        };
        json_api::bench_clean(h, &req)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCloudNotice(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeAckCloudNotice(
    _env: JNIEnv,
    _c: JClass,
    h: jlong,
) {
    if let Some(h) = handle(h) {
        guard((), || json_api::ack_cloud_notice(h));
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetCloudModel(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeSetCloudCacheDir(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCloudCatalog(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeRefreshCloudCatalog(
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
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCloudKeyInfo(
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

/// D52 custom cleanup server: `GET {base}/models` (blocking).
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCustomCleanupModels(
    mut env: JNIEnv,
    _c: JClass,
    h: jlong,
    base_url: JString,
    key: JString,
) -> jstring {
    let args = guard(None, || {
        Some((
            opt_jstr(&mut env, &base_url).unwrap_or_default(),
            opt_jstr(&mut env, &key),
        ))
    });
    let r = guard(r#"{"error":"error","message":"error"}"#.to_owned(), || {
        let (b, k) = args.unwrap_or_default();
        json_api::custom_cleanup_models(handle(h), &b, k)
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

/// D52 custom cleanup server address: normalised URL + canonical host, or
/// an error, plus `model_ok` (`json_api::custom_cleanup_address`). Pure.
#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeCustomCleanupAddress(
    mut env: JNIEnv,
    _c: JClass,
    base_url: JString,
    model: JString,
) -> jstring {
    let args = guard(None, || {
        Some((
            opt_jstr(&mut env, &base_url).unwrap_or_default(),
            opt_jstr(&mut env, &model).unwrap_or_default(),
        ))
    });
    let r = guard(
        r#"{"error":"error","message":"error","model_ok":false}"#.to_owned(),
        || {
            let (b, m) = args.unwrap_or_default();
            json_api::custom_cleanup_address(&b, &m)
        },
    );
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

// ---- D44/D46 meeting transcription (JNI_CONTRACT.md "Meeting transcription") --
// A job owns its engine handle (`nativeMeetingOpen` takes it over), so it never
// shares a session with the shell's dictation engine. `push` / `finish` block
// (worker thread); `progress` / `cancel` never wait for a chunk.

fn meeting<'a>(m: jlong) -> Option<&'a crate::meeting_api::MeetingJob> {
    // SAFETY: the shell only passes values returned by nativeMeetingOpen and
    // not yet passed to nativeMeetingClose (it serialises close against the
    // other calls).
    unsafe { (m as *const crate::meeting_api::MeetingJob).as_ref() }
}

/// Segments JSON, or `null` with the last error / kind (`cancelled`, `failed`).
fn meeting_out(env: &mut JNIEnv, r: Result<String, crate::meeting_api::JobError>) -> jstring {
    match r {
        Ok(s) => {
            clear_error();
            out(env, &s)
        }
        Err((kind, e)) => {
            set_error_kind(kind, e);
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMeetingOpen(
    mut env: JNIEnv,
    _c: JClass,
    engine: jlong,
    session: JString,
    params: JString,
) -> jlong {
    guard(0, || {
        if engine == 0 {
            set_error_kind("bad_config", "bad_config: no engine handle");
            return 0;
        }
        // Ownership of `engine` moves here whatever happens next (contract).
        // SAFETY: a value from nativeLoadModels / nativeOpenRouter that Kotlin released to us.
        let engine = *unsafe { Box::from_raw(engine as *mut MurmurHandle) };
        let Some(s) = jstr(&mut env, &session, "bad_config: session config") else {
            return 0;
        };
        let p = if params.is_null() {
            String::new()
        } else {
            match jstr(&mut env, &params, "bad_config: meeting params") {
                Some(p) => p,
                None => return 0,
            }
        };
        match crate::meeting_api::MeetingJob::open(engine, &s, &p) {
            Ok(j) => {
                clear_error();
                Box::into_raw(Box::new(j)) as jlong
            }
            Err((kind, e)) => {
                set_error_kind(kind, e);
                0
            }
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMeetingSetTotal(
    _env: JNIEnv,
    _c: JClass,
    m: jlong,
    samples: jlong,
) {
    guard((), || {
        if let Some(j) = meeting(m) {
            j.set_total_samples(samples.max(0) as usize)
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMeetingPush(
    mut env: JNIEnv,
    _c: JClass,
    m: jlong,
    pcm: JShortArray,
    len: jint,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        let Some(j) = meeting(m) else {
            set_error_kind("bad_config", "bad_config: no meeting job");
            return std::ptr::null_mut();
        };
        if pcm.is_null() {
            set_error_kind("bad_config", "bad_config: null pcm");
            return std::ptr::null_mut();
        }
        let cap = match env.get_array_length(&pcm) {
            Ok(n) => n.max(0),
            Err(e) => {
                clear(&mut env);
                set_error(format!("pcm: {e}"));
                return std::ptr::null_mut();
            }
        };
        let mut b = vec![0i16; len.clamp(0, cap) as usize];
        if let Err(e) = env.get_short_array_region(&pcm, 0, &mut b) {
            clear(&mut env);
            set_error(format!("pcm: {e}"));
            return std::ptr::null_mut();
        }
        let r = j.push(&b);
        meeting_out(&mut env, r)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMeetingFinish(
    mut env: JNIEnv,
    _c: JClass,
    m: jlong,
) -> jstring {
    guard(std::ptr::null_mut(), || {
        let Some(j) = meeting(m) else {
            set_error_kind("bad_config", "bad_config: no meeting job");
            return std::ptr::null_mut();
        };
        let r = j.finish();
        meeting_out(&mut env, r)
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMeetingProgress(
    mut env: JNIEnv,
    _c: JClass,
    m: jlong,
) -> jstring {
    let r = guard("{}".to_string(), || {
        meeting(m).map_or("{}".into(), |j| j.progress())
    });
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMeetingCancel(
    _env: JNIEnv,
    _c: JClass,
    m: jlong,
) {
    guard((), || {
        if let Some(j) = meeting(m) {
            j.cancel()
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_murmur_core_NativeMurmurCore_nativeMeetingClose(
    _env: JNIEnv,
    _c: JClass,
    m: jlong,
) {
    if m != 0 {
        // SAFETY: see `meeting`; Kotlin never uses `m` after this call.
        guard((), || {
            drop(unsafe { Box::from_raw(m as *mut crate::meeting_api::MeetingJob) })
        });
    }
}

// ---- D41 model library (JNI_CONTRACT.md "Model library"; feature `library`) --
// Static natives on `dev.murmur.server.ServerLibraryNative`, a class only
// the `hybrid` / `remote` flavours compile; the `local` library has neither
// these symbols nor any network code. Both calls block (worker thread).

#[cfg(feature = "library")]
#[no_mangle]
pub extern "system" fn Java_dev_murmur_server_ServerLibraryNative_nativeList(
    mut env: JNIEnv,
    _c: JClass,
    server: JString,
) -> jstring {
    let j = guard(None, || jstr(&mut env, &server, "server config"));
    let r = guard(
        crate::library_api::error_json("protocol", "panic in library list"),
        || match j.as_deref() {
            Some(j) => crate::library_api::list(j),
            None => crate::library_api::error_json("bad_config", "server config: null"),
        },
    );
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}

/// `listener`: any object with `void onProgress(long bytesOnDisk)` and
/// `boolean isCancelled()` (`ServerLibraryNative.Listener`). A listener
/// method that throws is cleared; a throwing `isCancelled` counts as false.
#[cfg(feature = "library")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_dev_murmur_server_ServerLibraryNative_nativeDownload(
    mut env: JNIEnv,
    _c: JClass,
    server: JString,
    model_id: JString,
    file: JString,
    dest: JString,
    part: JString,
    listener: jni::objects::JObject,
) -> jstring {
    use jni::objects::JValue;
    let args = guard(None, || {
        Some((
            jstr(&mut env, &server, "server config")?,
            jstr(&mut env, &model_id, "model id")?,
            jstr(&mut env, &file, "file")?,
            jstr(&mut env, &dest, "dest")?,
            jstr(&mut env, &part, "part")?,
        ))
    });
    let Some((server, model_id, file, dest, part)) = args else {
        return guard(std::ptr::null_mut(), || {
            out(
                &mut env,
                &crate::library_api::error_json("bad_config", "null argument"),
            )
        });
    };
    let cell = RefCell::new(env);
    let r = guard(
        crate::library_api::error_json("protocol", "panic in library download"),
        || {
            let mut progress = |n: u64| {
                if listener.is_null() {
                    return;
                }
                let mut env = cell.borrow_mut();
                if env
                    .call_method(&listener, "onProgress", "(J)V", &[JValue::Long(n as i64)])
                    .is_err()
                {
                    clear(&mut env);
                }
            };
            let cancel = || {
                if listener.is_null() {
                    return false;
                }
                let mut env = cell.borrow_mut();
                match env
                    .call_method(&listener, "isCancelled", "()Z", &[])
                    .and_then(|v| v.z())
                {
                    Ok(b) => b,
                    Err(_) => {
                        clear(&mut env);
                        false
                    }
                }
            };
            crate::library_api::download(
                &server,
                &model_id,
                &file,
                &dest,
                &part,
                &mut progress,
                &cancel,
            )
        },
    );
    let mut env = cell.into_inner();
    guard(std::ptr::null_mut(), || out(&mut env, &r))
}
