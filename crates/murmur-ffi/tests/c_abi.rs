//! Drive the C ABI exactly as a C/Tauri caller would (mock ASR backend).

use std::ffi::{CStr, CString};
use std::ptr;

use murmur_ffi::*;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

unsafe fn take(p: *mut std::os::raw::c_char) -> String {
    assert!(!p.is_null());
    let s = CStr::from_ptr(p).to_str().unwrap().to_string();
    murmur_string_free(p);
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
        let h = murmur_load_models(c(r#"{"asr_backend":{"mock":{"text":"hello btw"}}}"#).as_ptr());
        assert!(!h.is_null());
        let sess =
            c(r#"{"dictionary":[{"word":"btw","replacement":"by the way","isSnippet":false}]}"#);
        assert_eq!(murmur_start_session(h, sess.as_ptr()), 0);
        let pcm = tone(500);
        for ch in pcm.chunks(640) {
            assert_eq!(murmur_push_audio(h, ch.as_ptr(), ch.len()), 0);
        }
        assert_eq!(
            murmur_set_context(h, c(r#"{"app_id":"notepad"}"#).as_ptr()),
            0
        );
        let r: serde_json::Value = serde_json::from_str(&take(murmur_commit(h))).unwrap();
        assert_eq!(r["status"], "RAW_TRANSCRIPT");
        assert_eq!(r["final_text"], "hello by the way ");
        murmur_unload(h);
    }
}

#[test]
fn bad_json_reports_error() {
    unsafe {
        assert!(murmur_load_models(c("{not json").as_ptr()).is_null());
        let e = CStr::from_ptr(murmur_last_error()).to_str().unwrap();
        assert!(e.contains("config"), "{e}");
        assert_eq!(
            CStr::from_ptr(murmur_last_error_kind()).to_str(),
            Ok("bad_config")
        );
        let h = murmur_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        assert_eq!(murmur_start_session(h, c("[").as_ptr()), -1);
        murmur_unload(h);
    }
}

#[test]
fn null_handles_are_safe() {
    unsafe {
        assert_eq!(murmur_push_audio(ptr::null_mut(), ptr::null(), 0), -1);
        assert_eq!(murmur_start_session(ptr::null_mut(), ptr::null()), -1);
        murmur_cancel(ptr::null_mut());
        murmur_unload(ptr::null_mut());
        assert!(murmur_partial(ptr::null_mut()).is_null());
        let r: serde_json::Value =
            serde_json::from_str(&take(murmur_commit(ptr::null_mut()))).unwrap();
        assert_eq!(r["status"], "ERROR");
    }
}

#[test]
fn push_without_session_returns_1_and_cancel_works() {
    unsafe {
        let h = murmur_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        let pcm = tone(40);
        assert_eq!(murmur_push_audio(h, pcm.as_ptr(), pcm.len()), 1);
        murmur_start_session(h, c("{}").as_ptr());
        murmur_cancel(h);
        let r: serde_json::Value = serde_json::from_str(&take(murmur_commit(h))).unwrap();
        assert_eq!(r["status"], "ERROR");
        murmur_unload(h);
    }
}

#[test]
fn learn_edit_returns_json_candidates() {
    unsafe {
        let h = murmur_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        murmur_start_session(h, c("{}").as_ptr());
        let j = take(murmur_learn_edit(
            h,
            c("ask whisper now").as_ptr(),
            c("ask Zephyr now").as_ptr(),
        ));
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v[0]["corrected"], "Zephyr");
        murmur_unload(h);
    }
}

#[test]
fn whisper_without_feature_or_model_fails_cleanly() {
    unsafe {
        let h = murmur_load_models(
            c(r#"{"asr_backend":"whisper","asr_path":"/nonexistent.bin"}"#).as_ptr(),
        );
        assert!(h.is_null());
        assert!(!CStr::from_ptr(murmur_last_error())
            .to_str()
            .unwrap()
            .is_empty());
    }
}

#[cfg(feature = "whisper")]
#[test]
fn missing_model_kind_is_model_not_found() {
    unsafe {
        let h = murmur_load_models(
            c(r#"{"asr_backend":"whisper","asr_path":"/nonexistent.bin"}"#).as_ptr(),
        );
        assert!(h.is_null());
        assert_eq!(
            CStr::from_ptr(murmur_last_error_kind()).to_str(),
            Ok("model_not_found")
        );
        let e = CStr::from_ptr(murmur_last_error()).to_str().unwrap();
        assert_eq!(e, "model not found: /nonexistent.bin");
    }
}

#[test]
fn successful_load_clears_error_kind() {
    unsafe {
        assert!(murmur_load_models(c("{not json").as_ptr()).is_null());
        let h = murmur_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        assert!(!h.is_null());
        assert_eq!(CStr::from_ptr(murmur_last_error_kind()).to_str(), Ok(""));
        murmur_unload(h);
    }
}

/// D18 over the C ABI: without `cloud` every symbol says "not in this build";
/// with it, a router + key reaches the (mock) provider. The key is never echoed.
#[test]
fn cloud_symbols_over_c_abi() {
    unsafe {
        let caps = take(murmur_cloud_capabilities());
        let presets = take(murmur_cloud_presets());
        assert_eq!(
            caps.contains(r#""cloud":true"#),
            cfg!(feature = "cloud"),
            "{caps}"
        );
        assert_eq!(presets == "[]", !cfg!(feature = "cloud"), "{presets}");
        let key = c("sk-LEAKCHECK-cabi-0123456789");
        // D28: OpenRouter only; the mock is reached through the test base
        // URL override, never through config.
        #[cfg(feature = "cloud")]
        let srv = murmur_cloud::testing::MockServer::fixed(murmur_cloud::testing::Reply::json(
            200,
            serde_json::json!({"text": "and so my fellow americans", "usage": {"seconds": 3, "cost": 0.00001}}),
        ));
        #[cfg(feature = "cloud")]
        let _base = murmur_cloud::testing::override_base_url(&srv.url("/api/v1"));
        let p = c(r#"{"stt":{"model":"openai/whisper-large-v3-turbo"}}"#);
        let probe = take(murmur_probe_cloud(
            ptr::null_mut(),
            p.as_ptr(),
            key.as_ptr(),
        ));
        assert!(!probe.contains("LEAKCHECK"), "{probe}");
        let v: serde_json::Value = serde_json::from_str(&probe).unwrap();
        if cfg!(feature = "cloud") {
            assert_eq!(v["ok"], true, "{probe}");
            assert_eq!(v["transcript"], "and so my fellow americans");
            assert_eq!(v["provider"], "openrouter");
        } else {
            assert_eq!(v["error"], "cloud_not_configured", "{probe}");
        }
        // A legacy entry of a removed adapter is refused without a request.
        let groq = c(
            r#"{"id":"groq-1","adapter":"openai_compat","base_url":"https://api.groq.com/openai/v1","model":"m"}"#,
        );
        let refused = take(murmur_probe_cloud(
            ptr::null_mut(),
            groq.as_ptr(),
            key.as_ptr(),
        ));
        assert!(refused.contains("cloud_not_configured"), "{refused}");
        let bad = take(murmur_probe_cloud(
            ptr::null_mut(),
            c("{").as_ptr(),
            ptr::null(),
        ));
        assert!(bad.contains("cloud_not_configured"), "{bad}");

        let core = murmur_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        assert_eq!(
            murmur_set_cloud_key(core, c("openrouter").as_ptr(), key.as_ptr()),
            -1
        );
        let rt = murmur_open_router(
            c(r#"{"allow_server":false,"allow_client":true}"#).as_ptr(),
            core,
        );
        assert!(!rt.is_null());
        // A removed provider's key never enters the router.
        assert_eq!(murmur_set_cloud_key_for(rt, groq.as_ptr(), key.as_ptr()), 0);
        let none = take(murmur_probe_cloud(rt, p.as_ptr(), ptr::null()));
        assert!(none.contains("cloud_not_configured"), "{none}");
        // The OpenRouter key: the probe may use it (key NULL).
        assert_eq!(
            murmur_set_cloud_key(rt, c("openrouter").as_ptr(), key.as_ptr()),
            0
        );
        let stored = take(murmur_probe_cloud(rt, p.as_ptr(), ptr::null()));
        assert_eq!(
            stored.contains(r#""ok":true"#),
            cfg!(feature = "cloud"),
            "{stored}"
        );
        let ki = take(murmur_cloud_key_info(ptr::null_mut()));
        assert!(ki.contains("cloud_not_configured"), "{ki}");
        assert_eq!(murmur_set_cloud_usage(rt, c("not json").as_ptr()), -1);
        assert_eq!(
            CStr::from_ptr(murmur_last_error_kind()).to_str(),
            Ok("bad_config")
        );
        murmur_reset_cloud_usage(rt);
        let u: serde_json::Value = serde_json::from_str(&take(murmur_cloud_usage(rt))).unwrap();
        assert_eq!(u["providers"], serde_json::json!([]));
        assert!(murmur_cloud_usage(ptr::null_mut()).is_null());
        murmur_unload(rt);
        // Cloud-only: an engine only in a cloud build.
        let only = c(
            r#"{"allow_server":false,"allow_client":false,"allow_cloud":true,"order":["cloud"]}"#,
        );
        let h = murmur_open_router(only.as_ptr(), ptr::null_mut());
        assert_eq!(!h.is_null(), cfg!(feature = "cloud"));
        murmur_unload(h);
    }
}

/// Command Mode over the C ABI: a core with an on-device LLM rewrites the
/// selection (answer sanitised by murmur-core); without one the JSON says
/// `no_llm` and nothing is called.
#[test]
fn command_over_c_abi() {
    use murmur_core::cleanup::{CleanError, CleanPrompt, Cleaner};
    use std::sync::{Arc, Mutex};
    struct Llm(Mutex<Vec<String>>);
    impl Cleaner for Llm {
        fn clean(&self, p: &CleanPrompt, _d: std::time::Instant) -> Result<String, CleanError> {
            self.0.lock().unwrap().push(p.user.clone());
            Ok("Sure! Here's the shorter version:\n\"Short.\"".into())
        }
    }
    unsafe {
        let llm = Arc::new(Llm(Mutex::new(Vec::new())));
        let core = murmur_core::MurmurCore::from_engines(
            Arc::new(murmur_core::asr::MockAsr::new("x")),
            Some(llm.clone() as Arc<dyn Cleaner>),
        );
        let h = Box::into_raw(Box::new(MurmurHandle::Core(core)));
        let v: serde_json::Value = serde_json::from_str(&take(murmur_command(
            h,
            c("make it shorter").as_ptr(),
            c("A very long sentence.").as_ptr(),
        )))
        .unwrap();
        assert_eq!(v, serde_json::json!({"ok": true, "text": "Short."}));
        let sent = llm.0.lock().unwrap()[0].clone();
        assert!(sent.contains("make it shorter") && sent.contains("A very long sentence."));
        // Blank instruction: refused before the LLM.
        let v = take(murmur_command(h, c("  ").as_ptr(), ptr::null()));
        assert!(v.contains(r#""kind":"no_instruction""#), "{v}");
        assert_eq!(llm.0.lock().unwrap().len(), 1);
        murmur_unload(h);

        // No LLM at all (mock ASR only), core and router handles alike.
        let h = murmur_load_models(c(r#"{"asr_backend":{"mock":{"text":"x"}}}"#).as_ptr());
        let v = take(murmur_command(h, c("shorten").as_ptr(), c("x").as_ptr()));
        assert!(
            v.contains(r#""ok":false"#) && v.contains(r#""kind":"no_llm""#),
            "{v}"
        );
        let rt = murmur_open_router(c(r#"{"allow_client":true}"#).as_ptr(), h);
        assert!(!rt.is_null());
        let v = take(murmur_command(rt, c("shorten").as_ptr(), c("x").as_ptr()));
        assert!(v.contains(r#""kind":"no_llm""#), "{v}");
        murmur_unload(rt);
        assert!(murmur_command(ptr::null_mut(), c("x").as_ptr(), ptr::null()).is_null());
    }
}
