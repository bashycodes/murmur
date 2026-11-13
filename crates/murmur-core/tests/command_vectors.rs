//! The Command Mode outcome vectors shared with the Android shell
//! (`fixtures/command/command_results.json`, also decoded by the Kotlin
//! `CommandVectorsTest`) must match what `command::result_json` emits, so
//! the JNI wire shape and the pill texts can't drift from the Kotlin side
//! unnoticed. Every `CommandError` kind and the answer to a question are
//! covered.

use murmur_core::command::{result_json, CommandError, CommandOutput};
use serde_json::Value;

fn fixture() -> Value {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/command/command_results.json"
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn input(v: &Value) -> Result<CommandOutput, CommandError> {
    if let Some(t) = v.get("ok") {
        return Ok(CommandOutput::Text(t.as_str().unwrap().to_string()));
    }
    if let Some(a) = v.get("answer") {
        return Ok(CommandOutput::Answer(a.as_str().unwrap().to_string()));
    }
    let num = |k: &str| v[k].as_u64().unwrap();
    Err(match v["err"].as_str().unwrap() {
        "no_instruction" => CommandError::NoInstruction,
        "selection_too_long" => CommandError::SelectionTooLong {
            max: num("max") as usize,
        },
        "no_llm" => CommandError::NoLlm,
        "selection_not_allowed" => CommandError::SelectionNotAllowed {
            to: v.get("to").map(|t| t.as_str().unwrap().into()),
        },
        "timeout" => CommandError::Timeout { secs: num("secs") },
        "empty" => CommandError::Empty,
        "language_changed" => CommandError::LanguageChanged,
        "backend" => CommandError::Backend(v["detail"].as_str().unwrap().into()),
        other => panic!("unknown kind {other}"),
    })
}

#[test]
fn result_json_matches_the_shared_vectors() {
    let f = fixture();
    let cases = f["cases"].as_array().unwrap();
    let mut kinds = std::collections::BTreeSet::new();
    let mut answers = 0;
    for c in cases {
        let r = input(&c["input"]);
        match &r {
            Err(e) => {
                kinds.insert(e.kind());
            }
            Ok(CommandOutput::Answer(_)) => answers += 1,
            Ok(CommandOutput::Text(_)) => {}
        }
        assert_eq!(result_json(&r), c["json"], "{}", c["input"]);
    }
    assert_eq!(kinds.len(), 8, "every CommandError kind has a vector");
    assert!(answers > 0, "an answer has a vector");
}

#[test]
fn the_max_selection_vector_is_the_real_limit() {
    let f = fixture();
    let max = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|c| c["input"].get("max").and_then(Value::as_u64))
        .unwrap();
    assert_eq!(max as usize, murmur_core::command::MAX_SELECTION_CHARS);
}
