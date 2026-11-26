//! Shared team dictionary merge vectors (`fixtures/team-dictionary/merge.json`).
//! The Android `TeamDictionaryVectorsTest` checks the same file against
//! `TeamDictionary.merge`, so both shells merge a server's team list into
//! the personal dictionary the same way.

use murmur_core::config::{FieldContext, SessionConfig};
use murmur_core::dictionary::{with_team, DictEntry, Dictionary};
use murmur_core::postprocess;
use serde_json::Value;

fn cases() -> Vec<Value> {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/team-dictionary/merge.json"
    );
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    v["cases"].as_array().unwrap().clone()
}

fn entries(v: &Value) -> Vec<DictEntry> {
    serde_json::from_value(v.clone()).unwrap()
}

#[test]
fn merge_vectors() {
    let all = cases();
    assert!(all.len() >= 5);
    for c in all {
        let got = with_team(&entries(&c["personal"]), &entries(&c["team"]));
        assert_eq!(got.entries, entries(&c["want"]), "{}", c["name"]);
    }
}

/// The merged list does what each list does on its own: replacements,
/// ASR vocabulary (hotwords and the Whisper prompt) and cleanup terms all
/// read `SessionConfig.dictionary`, and the personal spelling wins.
#[test]
fn merged_list_replaces_and_biases_with_personal_first() {
    let personal = [DictEntry::replacement("zephyr", "Zephyr")];
    let team = [
        DictEntry::replacement("zephyr", "ZephyrCo"),
        DictEntry::replacement("acme corp", "AcmeCorp"),
        DictEntry::word("Quillon"),
    ];
    let cfg = SessionConfig {
        dictionary: with_team(&personal, &team),
        ..SessionConfig::default()
    };
    let out = postprocess::apply(
        "zephyr met acme corp",
        "zephyr met acme corp",
        &cfg,
        &FieldContext::default(),
    );
    assert!(
        out.text.starts_with("Zephyr met AcmeCorp"),
        "{:?}",
        out.text
    );
    assert_eq!(
        cfg.dictionary.vocabulary(),
        vec!["Zephyr", "AcmeCorp", "Quillon"]
    );
    // Without a team list nothing changes.
    assert_eq!(
        with_team(&personal, &[]),
        Dictionary {
            entries: personal.to_vec()
        }
    );
}
