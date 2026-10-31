//! D28 host test: a real JVM drives the real cdylib (feature `cloud`) through
//! the JNI exports the Android shell uses, offline: capabilities + the single
//! preset, the schema-1 → 2 migration (report + notice), key aliasing (a
//! removed provider's key is ignored), catalogue search from the baked
//! snapshot, cleanup toggle / consent gate / model selection, and that the
//! key never appears in any output. The network paths (OpenRouter mock via
//! `flow_cloud::testing::override_base_url`, which a separate JVM process
//! cannot use) are in flow-contract-tests. Skips when no `java` is on PATH.
#![cfg(all(feature = "jni", feature = "cloud"))]

use std::path::PathBuf;
use std::process::Command;

const KEY: &str = "sk-LEAKCHECK-jni-0123456789abcdef";

fn features() -> String {
    let mut f = vec!["jni", "cloud"];
    if cfg!(feature = "remote") {
        f.push("remote");
    }
    // Same set as jni_smoke / jni_library build, so none undoes another's cdylib.
    if cfg!(feature = "library") {
        f.push("library");
    }
    f.join(",")
}

fn json(line: &str) -> serde_json::Value {
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"))
}

#[test]
fn jvm_openrouter_flow_through_native_router() {
    if Command::new("java").arg("-version").output().is_err() {
        eprintln!("SKIP: no java on PATH");
        return;
    }
    let st = Command::new(env!("CARGO"))
        .args(["build", "-q", "-p", "flow-ffi", "--lib", "--features"])
        .arg(features())
        .status()
        .unwrap();
    assert!(st.success());
    let exe = std::env::current_exe().unwrap();
    let lib_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
    let cache = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("jni_cloud_cache");
    std::fs::create_dir_all(&cache).unwrap();
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/java/dev/flowclone/core/JniCloud.java");
    let out = Command::new("java")
        .arg(format!("-Djava.library.path={}", lib_dir.display()))
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg(&src)
        .arg(KEY)
        .arg(&cache)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{stdout}\n{stderr}");
    assert!(
        !stdout.contains(KEY) && !stderr.contains(KEY),
        "C3: the key leaked into JNI output"
    );
    let mut l = stdout.lines();

    let caps = json(l.next().unwrap());
    assert_eq!(
        caps,
        serde_json::json!({"cloud": true, "adapters": ["openrouter"]})
    );
    let presets = json(l.next().unwrap());
    let presets = presets.as_array().unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0]["id"], "openrouter");
    assert_eq!(presets[0]["base_url"], "https://openrouter.ai/api/v1");

    let mig = json(l.next().unwrap());
    let rep = &mig["report"];
    assert_eq!(rep["migrated"], true, "{mig}");
    assert_eq!(rep["key_from"], "openrouter-1");
    assert_eq!(rep["key_to"], "openrouter");
    assert_eq!(rep["delete_keys"], serde_json::json!(["groq-1"]));
    assert_eq!(rep["cloud_disabled"], true);
    assert_eq!(rep["model_to"], "openai/whisper-large-v3-turbo");
    assert!(rep["notice"].as_str().unwrap().contains("Groq was removed"));
    assert_eq!(mig["config"]["cloud"]["schema"], 2);
    assert_eq!(
        mig["config"]["cloud"]["openrouter"]["stt"]["consent_host"],
        "openrouter.ai"
    );
    assert!(mig["config"]["cloud"].get("providers").is_none());
    assert_eq!(l.next().unwrap(), "KIND bad_config");

    let stt = json(l.next().unwrap());
    assert!(!stt.as_array().unwrap().is_empty(), "baked STT rows: {stt}");
    assert!(stt.to_string().contains("whisper"), "{stt}");
    let cl = json(l.next().unwrap());
    assert!(cl.to_string().contains("llama"), "{cl}");
    assert_eq!(l.next().unwrap(), "[]");

    let notice = l.next().unwrap();
    assert!(
        notice.starts_with("Cloud speech-to-text now uses OpenRouter only."),
        "{notice}"
    );
    assert_eq!(l.next().unwrap(), "NOTICE null");
    let ki = json(l.next().unwrap());
    assert_eq!(
        ki["error"], "cloud_not_configured",
        "removed provider's key ignored: {ki}"
    );

    let first = json(l.next().unwrap());
    assert_eq!(first["route"]["engine"], "local", "{first}");
    assert_eq!(first["final_text"], "local words by the way ");

    let st = json(l.next().unwrap());
    assert_eq!(st["supported"], true, "{st}");
    assert_eq!(st["enabled"], true);
    assert_eq!(st["consent"], false);
    assert_eq!(st["blocked"], "cloud_cleanup_not_configured");
    assert_eq!(st["timeout_ms"], 2500);
    assert_eq!(l.next().unwrap(), "KIND bad_config", "timeout out of range");
    assert_eq!(l.next().unwrap(), "KIND bad_config", "bad model id");
    let st = json(l.next().unwrap());
    assert_eq!(st["model"], "meta-llama/llama-3.1-8b-instruct");
    let d = json(l.next().unwrap());
    assert_eq!(d["route"]["engine"], "local", "{d}");
    assert!(
        d["route"].get("llm_engine").is_none(),
        "no consent, no cloud cleanup: {d}"
    );
    assert!(d.get("llm").is_none(), "{d}");
    assert_eq!(
        d["fallback_reason"],
        "llm_error: cloud_cleanup_not_configured"
    );
    let rs = json(l.next().unwrap());
    assert_eq!(rs["cleanup_circuit"]["open"], false, "{rs}");
}
