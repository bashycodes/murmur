//! Drive the C ABI exactly as a C/Tauri caller would (mock ASR backend).

use std::ffi::{CStr, CString};
use std::ptr;

use flow_ffi::*;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

unsafe fn take(p: *mut std::os::raw::c_char) -> String {
    assert!(!p.is_null());
    let s = CStr::from_ptr(p).to_str().unwrap().to_string();
    flow_string_free(p);
    s
}

fn tone(ms: usize) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect()
}

#[test]
fn full_session_over_c_abi() {
    unsafe {
        let h = flow_load_models(c(r#"{"asr_backend":{"mock":{"text":"hello btw"}}}"#).as_ptr());
        assert!(!h.is_null());
        let sess =
            c(r#"{"dictionary":[{"word":"btw","replacement":"by the way","isSnippet":false}]}"#);
        assert_eq!(flow_start_session(h, sess.as_ptr()), 0);
        let pcm = tone(500);
        for ch in pcm.chunks(640) {
            assert_eq!(flow_push_audio(h, ch.as_ptr(), ch.len()), 0);
        }
        assert_eq!(
            flow_set_context(h, c(r#"{"app_id":"notepad"}"#).as_ptr()),
            0
        );
        let r: serde_json::Value = serde_json::from_str(&take(flow_commit(h))).unwrap();
        assert_eq!(r["status"], "RAW_TRANSCRIPT");
        assert_eq!(r["final_text"], "hello by the way ");
        flow_unload(h);
    }
}

#[test]
fn bad_json_reports_error() {
    unsafe {
        assert!(flow_load_models(c("{not json").as_ptr()).is_null());
        let e = CStr::from_ptr(flow_last_error()).to_str().unwrap();
        assert!(e.contains("config"), "{e}");
        let h = flow_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        assert_eq!(flow_start_session(h, c("[").as_ptr()), -1);
        flow_unload(h);
    }
}

#[test]
fn null_handles_are_safe() {
    unsafe {
        assert_eq!(flow_push_audio(ptr::null_mut(), ptr::null(), 0), -1);
        assert_eq!(flow_start_session(ptr::null_mut(), ptr::null()), -1);
        flow_cancel(ptr::null_mut());
        flow_unload(ptr::null_mut());
        assert!(flow_partial(ptr::null_mut()).is_null());
        let r: serde_json::Value =
            serde_json::from_str(&take(flow_commit(ptr::null_mut()))).unwrap();
        assert_eq!(r["status"], "ERROR");
    }
}

#[test]
fn push_without_session_returns_1_and_cancel_works() {
    unsafe {
        let h = flow_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        let pcm = tone(40);
        assert_eq!(flow_push_audio(h, pcm.as_ptr(), pcm.len()), 1);
        flow_start_session(h, c("{}").as_ptr());
        flow_cancel(h);
        let r: serde_json::Value = serde_json::from_str(&take(flow_commit(h))).unwrap();
        assert_eq!(r["status"], "ERROR");
        flow_unload(h);
    }
}

#[test]
fn learn_edit_returns_json_candidates() {
    unsafe {
        let h = flow_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        flow_start_session(h, c("{}").as_ptr());
        let j = take(flow_learn_edit(
            h,
            c("ask whisper now").as_ptr(),
            c("ask Wispr now").as_ptr(),
        ));
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v[0]["corrected"], "Wispr");
        flow_unload(h);
    }
}

#[test]
fn whisper_without_feature_or_model_fails_cleanly() {
    unsafe {
        let h = flow_load_models(
            c(r#"{"asr_backend":"whisper","asr_path":"/nonexistent.bin"}"#).as_ptr(),
        );
        assert!(h.is_null());
        assert!(!CStr::from_ptr(flow_last_error())
            .to_str()
            .unwrap()
            .is_empty());
    }
}
