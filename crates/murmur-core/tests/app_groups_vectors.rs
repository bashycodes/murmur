//! Shared app-group vectors (`fixtures/app-groups/vectors.json`, D81).
//! The Android `AppGroupsVectorsTest` checks the same file against
//! `PolicyLists`, so both sides read `data/app-groups.json` the same way.

use murmur_core::app_groups;
use murmur_core::config::Platform;
use murmur_core::postprocess::apps;
use serde_json::Value;

fn vectors() -> Vec<Value> {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/app-groups/vectors.json"
    );
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    v["vectors"].as_array().unwrap().clone()
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

#[test]
fn app_group_vectors() {
    let mut fails = Vec::new();
    for v in vectors() {
        let (app, host) = (s(&v, "app"), s(&v, "host"));
        let (want, got) = if let Some(bucket) = v.get("bucket").and_then(Value::as_str) {
            let platform = match s(&v, "platform") {
                "android" => Platform::Android,
                "windows" => Platform::Windows,
                p => panic!("bucket vector with unknown platform {p:?}: {v}"),
            };
            let got = apps::classify(platform, app, host).bucket.as_str();
            (Value::from(bucket), Value::from(got))
        } else {
            let g = match s(&v, "platform") {
                "android" => app_groups::android(s(&v, "group")),
                _ => app_groups::windows(s(&v, "group")),
            };
            (v["expect"].clone(), Value::from(g.matches(app, host)))
        };
        if want != got {
            fails.push(format!("{v}: got {got}"));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
