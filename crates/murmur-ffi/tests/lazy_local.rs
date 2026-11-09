//! D24: a router opened without a local engine loads one on demand
//! (`json_api::set_local_loader`, JNI `nativeSetLocalLoader`), trying the
//! model configs in order.

use std::ffi::CStr;

use flow_ffi::*;

unsafe fn take(p: *mut std::os::raw::c_char) -> String {
    assert!(!p.is_null());
    let s = CStr::from_ptr(p).to_str().unwrap().to_string();
    flow_string_free(p);
    s
}

#[test]
fn cold_router_loads_the_first_config_that_works_and_dictates() {
    let h = json_api::open_router(
        r#"{"allow_server":false,"allow_client":true,"keep_local_warm":false}"#,
        None,
    )
    .unwrap();
    assert!(json_api::set_local_loader(&h, "{").is_err());
    json_api::set_local_loader(
        &h,
        r#"[{"asr_backend":"whisper","asr_path":"/nonexistent.bin"},{"asr_backend":{"mock":{"text":"hello there"}}}]"#,
    )
    .unwrap();
    let info: serde_json::Value =
        serde_json::from_str(&json_api::start_session_info(&h, "{}", false).unwrap()).unwrap();
    assert_eq!(info["route"]["engine"], "local");
    let h = Box::into_raw(Box::new(h));
    unsafe {
        let pcm: Vec<i16> = (0..8000)
            .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
            .collect();
        for ch in pcm.chunks(640) {
            assert_eq!(flow_push_audio(h, ch.as_ptr(), ch.len()), 0);
        }
        let r: serde_json::Value = serde_json::from_str(&take(flow_commit(h))).unwrap();
        assert_eq!(r["status"], "RAW_TRANSCRIPT", "{r}");
        assert_eq!(r["final_text"], "hello there ");
        let st: serde_json::Value = serde_json::from_str(&take(flow_route_status(h))).unwrap();
        assert_eq!(st["last_diag"]["local_cold"], true, "{st}");
        assert!(st["last_diag"]["local_load_ms"].is_u64());
        flow_unload(h);
    }
}

#[test]
fn loader_failure_says_why() {
    let h = json_api::open_router(r#"{"allow_server":false,"allow_client":true}"#, None).unwrap();
    json_api::set_local_loader(
        &h,
        r#"[{"asr_backend":"whisper","asr_path":"/nonexistent.bin"}]"#,
    )
    .unwrap();
    json_api::start_session(&h, "{}").unwrap();
    let r: serde_json::Value = serde_json::from_str(&json_api::commit(Some(&h))).unwrap();
    assert_eq!(r["status"], "ERROR");
    let e = r["error"].as_str().unwrap();
    assert!(e.contains("phone model failed to load"), "{e}");
    assert!(!json_api::prewarm(&h), "no server route");
}
