//! Shared model-fit vectors (`fixtures/model-fit/vectors.json`, D117). The
//! Android `ModelFitVectorsTest` checks the same file against its Kotlin
//! twin (`ModelFit.kt`), so the phone and the PC read one table.
//! `MURMUR_BLESS=1` rewrites the expected values from this side.

use murmur_core::model_fit::{fit, recommend, Machine, Model};
use serde_json::{json, Value};

const PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/model-fit/vectors.json"
);

fn machine(v: &Value) -> Machine {
    Machine {
        total_ram: v["ram"].as_u64().unwrap(),
        free_disk: v["free"].as_u64(),
        gpu_llm: v["gpu_llm"].as_bool().unwrap(),
        gpu_whisper: v["gpu_whisper"].as_bool().unwrap(),
    }
}

#[test]
fn model_fit_vectors() {
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(PATH).unwrap()).unwrap();
    let bless = std::env::var_os("MURMUR_BLESS").is_some();
    let mut fails = Vec::new();
    for v in doc["recommend"].as_array_mut().unwrap() {
        let p = recommend(&machine(v));
        let got = json!({
            "speech": p.speech.key(),
            "cleanup": p.cleanup.map(Model::key),
            "hebrew": p.hebrew.key(),
        });
        if bless {
            v["expect"] = got;
        } else if v["expect"] != got {
            fails.push(format!("{v}: got {got}"));
        }
    }
    for v in doc["fit"].as_array_mut().unwrap() {
        let model = Model::from_key(v["model"].as_str().unwrap()).expect("known model key");
        let f = fit(
            model,
            v["size"].as_u64().unwrap(),
            v["installed"].as_bool().unwrap(),
            &machine(v),
        );
        let got = json!({ "fits": f.fits, "reasons": f.reasons });
        if bless {
            v["expect"] = got;
        } else if v["expect"] != got {
            fails.push(format!("{v}: got {got}"));
        }
    }
    if bless {
        let mut text = serde_json::to_string_pretty(&doc).unwrap();
        text.push('\n');
        std::fs::write(PATH, text).unwrap();
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
    // Every model key appears in a fit vector.
    for m in Model::ALL {
        assert!(
            doc["fit"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["model"] == m.key()),
            "{m:?} has no fit vector"
        );
    }
}
