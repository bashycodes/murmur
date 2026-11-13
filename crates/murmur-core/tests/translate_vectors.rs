//! D90: the translation outcome vectors shared with the Android shell
//! (`fixtures/translate/translate_results.json`, also decoded by the Kotlin
//! `TranslateVectorsTest`) must match what `translate::result_json` emits,
//! so the JNI wire shape and the pill texts can't drift from the Kotlin side
//! unnoticed. Every `TranslateError` kind is covered.

use murmur_core::translate::{result_json, TranslateError};
use serde_json::Value;

fn fixture() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/translate/translate_results.json"
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn input(v: &Value) -> Result<String, TranslateError> {
    if let Some(t) = v.get("ok") {
        return Ok(t.as_str().unwrap().to_string());
    }
    let num = |k: &str| v[k].as_u64().unwrap();
    Err(match v["err"].as_str().unwrap() {
        "no_text" => TranslateError::NoText,
        "text_too_long" => TranslateError::TextTooLong {
            max: num("max") as usize,
        },
        "no_target" => TranslateError::NoTarget,
        "no_llm" => TranslateError::NoLlm,
        "timeout" => TranslateError::Timeout { secs: num("secs") },
        "empty" => TranslateError::Empty,
        "not_translated" => TranslateError::NotTranslated,
        "backend" => TranslateError::Backend(v["detail"].as_str().unwrap().into()),
        other => panic!("unknown kind {other}"),
    })
}

#[test]
fn result_json_matches_the_shared_vectors() {
    let f = fixture();
    let mut kinds = std::collections::BTreeSet::new();
    for c in f["cases"].as_array().unwrap() {
        let r = input(&c["input"]);
        if let Err(e) = &r {
            kinds.insert(e.kind());
        }
        assert_eq!(result_json(&r), c["json"], "{}", c["input"]);
    }
    assert_eq!(kinds.len(), 8, "every TranslateError kind has a vector");
}

#[test]
fn the_max_text_vector_is_the_real_limit() {
    let f = fixture();
    let max = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|c| c["input"].get("max").and_then(Value::as_u64))
        .unwrap();
    assert_eq!(max as usize, murmur_core::translate::MAX_TEXT_CHARS);
}
