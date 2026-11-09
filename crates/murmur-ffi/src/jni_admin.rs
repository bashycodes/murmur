//! JNI exports for the Android "Users & keys" page (D39; features `jni` +
//! `admin`, so only the `hybrid` / `remote` flavours' library has them).
//!
//! Static natives on `dev.flowclone.admin.NativeAdmin` (Kotlin `object` +
//! `@JvmStatic external`), kept apart from `NativeFlowCore` so the admin
//! surface is one file on each side. Contract:
//! `apps/flow-android/JNI_CONTRACT.md` "Admin API". Each call is blocking
//! (≤ `flow_remote::admin::REQUEST_TIMEOUT`) and answers the JSON envelope of
//! [`crate::admin_api`]; it never returns null, panics into the JVM or leaves
//! a Java exception pending. Nothing is logged.

use std::panic::{catch_unwind, AssertUnwindSafe};

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jstring};
use jni::JNIEnv;

use crate::admin_api;

const PANIC: &str = r#"{"ok":false,"code":"server_error","message":"panic inside flow-ffi"}"#;

fn clear(env: &mut JNIEnv) {
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
}

fn arg(env: &mut JNIEnv, s: &JString) -> Option<String> {
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

fn bad_arg(what: &str) -> String {
    serde_json::json!({"ok": false, "code": "bad_request", "message": format!("{what}: null or invalid string")})
        .to_string()
}

/// Run `f` (panics become an error answer) and hand its answer to Java,
/// wiping the Rust copy (it may hold a new key's secret).
fn reply(env: &mut JNIEnv, f: impl FnOnce(&mut JNIEnv) -> String) -> jstring {
    let mut s = catch_unwind(AssertUnwindSafe(|| f(env))).unwrap_or_else(|_| PANIC.to_owned());
    let r = match env.new_string(&s) {
        Ok(j) => j.into_raw(),
        Err(_) => {
            clear(env);
            match env.new_string(PANIC) {
                Ok(j) => j.into_raw(),
                Err(_) => {
                    clear(env);
                    std::ptr::null_mut()
                }
            }
        }
    };
    admin_api::wipe(&mut s);
    r
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_admin_NativeAdmin_nativeAdminWhoami(
    mut env: JNIEnv,
    _c: JClass,
    server: JString,
) -> jstring {
    reply(&mut env, |env| match arg(env, &server) {
        Some(cfg) => admin_api::whoami(&cfg),
        None => bad_arg("server config"),
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_admin_NativeAdmin_nativeAdminListKeys(
    mut env: JNIEnv,
    _c: JClass,
    server: JString,
) -> jstring {
    reply(&mut env, |env| match arg(env, &server) {
        Some(cfg) => admin_api::list_keys(&cfg),
        None => bad_arg("server config"),
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_admin_NativeAdmin_nativeAdminCreateKey(
    mut env: JNIEnv,
    _c: JClass,
    server: JString,
    name: JString,
    admin: jboolean,
) -> jstring {
    reply(&mut env, |env| match (arg(env, &server), arg(env, &name)) {
        (Some(cfg), Some(name)) => admin_api::create_key(&cfg, &name, admin != 0),
        (None, _) => bad_arg("server config"),
        (_, None) => bad_arg("key name"),
    })
}

#[no_mangle]
pub extern "system" fn Java_dev_flowclone_admin_NativeAdmin_nativeAdminRevokeKey(
    mut env: JNIEnv,
    _c: JClass,
    server: JString,
    id: JString,
) -> jstring {
    reply(&mut env, |env| match (arg(env, &server), arg(env, &id)) {
        (Some(cfg), Some(id)) => admin_api::revoke_key(&cfg, &id),
        (None, _) => bad_arg("server config"),
        (_, None) => bad_arg("key id"),
    })
}
