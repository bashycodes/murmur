//! D33 / H5: `load_language` in the model-config JSON makes the ffi load only
//! the model that language routes to (`FlowCore::load_models_for`), so a
//! RAM-tight shell (Android) keeps one ASR model resident.

use flow_ffi::{flow_push_audio, json_api, FlowHandle};

const HE: &str = r#""languages":{"he":{"asr_backend":{"mock":{"text":"shalom"}}}}"#;

fn dictate(h: &mut FlowHandle, session: &str) -> serde_json::Value {
    json_api::start_session(h, session).unwrap();
    let pcm: Vec<i16> = (0..8000)
        .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
        .collect();
    for ch in pcm.chunks(640) {
        // SAFETY: `h` is a live handle; `ch` outlives the call.
        assert_eq!(unsafe { flow_push_audio(h, ch.as_ptr(), ch.len()) }, 0);
    }
    serde_json::from_str(&json_api::commit(Some(h))).unwrap()
}

#[test]
fn load_language_loads_only_the_routed_model() {
    // The default model does not exist: loading everything fails, loading
    // only Hebrew's model does not touch it.
    let cfg = format!(r#"{{"asr_backend":"whisper","asr_path":"/nonexistent.bin",{HE}}}"#);
    assert!(json_api::load_models_kind(&cfg).is_err());
    let with = format!(
        r#"{{"asr_backend":"whisper","asr_path":"/nonexistent.bin",{HE},"load_language":"he"}}"#
    );
    let core = json_api::load_models_kind(&with).expect("only the Hebrew model is loaded");
    let mut h = FlowHandle::Core(core);
    // Every session decodes on the one loaded model.
    let r = dictate(&mut h, r#"{"languages":["he"]}"#);
    assert_eq!(r["final_text"], "shalom ", "{r}");
}

#[test]
fn load_language_without_a_route_loads_the_default_only() {
    let cfg = r#"{"asr_backend":{"mock":{"text":"hello"}},"asr_path":"","languages":{"he":{"asr_backend":"whisper","asr_path":"/nonexistent.bin"}},"load_language":"en"}"#;
    let core = json_api::load_models_kind(cfg).expect("the missing Hebrew model is not loaded");
    let mut h = FlowHandle::Core(core);
    assert_eq!(dictate(&mut h, "{}")["final_text"], "hello ");
}

#[test]
fn bad_load_language_is_a_config_error() {
    let (kind, msg) =
        json_api::load_models_kind(r#"{"asr_backend":{"mock":{"text":"x"}},"load_language":5}"#)
            .err()
            .unwrap();
    assert_eq!(kind, "bad_config", "{msg}");
    // null / absent = load everything, as before.
    assert!(json_api::load_models_kind(
        r#"{"asr_backend":{"mock":{"text":"x"}},"load_language":null}"#
    )
    .is_ok());
}

#[test]
fn local_loader_honours_load_language() {
    let mut h = json_api::open_router(
        r#"{"allow_server":false,"allow_client":true,"keep_local_warm":false}"#,
        None,
    )
    .unwrap();
    let cfg = format!(
        r#"[{{"asr_backend":"whisper","asr_path":"/nonexistent.bin",{HE},"load_language":"he"}}]"#
    );
    json_api::set_local_loader(&h, &cfg).unwrap();
    assert!(json_api::set_local_loader(&h, r#"[{"load_language":[]}]"#).is_err());
    json_api::set_local_loader(&h, &cfg).unwrap();
    let r = dictate(&mut h, r#"{"languages":["he"]}"#);
    assert_eq!(r["final_text"], "shalom ", "{r}");
}
