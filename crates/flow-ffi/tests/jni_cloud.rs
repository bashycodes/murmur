//! D18 host test: a real JVM drives the real cdylib (feature `cloud`) through
//! the JNI exports the Android shell uses, against flow-cloud's in-process mock
//! provider: no key → device; key → cloud (local pipeline applied, usage
//! counted); provider 401 → `auth_failed` + full replay into the device engine;
//! Test provider with and without a router; the key never appears in any
//! output. Skips when no `java` is on PATH.
#![cfg(all(feature = "jni", feature = "cloud"))]

use std::path::PathBuf;
use std::process::Command;

use flow_cloud::testing::{MockServer, Reply};

const KEY: &str = "sk-LEAKCHECK-jni-0123456789abcdef";

fn features() -> String {
    let mut f = vec!["jni", "cloud"];
    if cfg!(feature = "remote") {
        f.push("remote");
    }
    f.join(",")
}

fn json(line: &str) -> serde_json::Value {
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"))
}

#[test]
fn jvm_cloud_route_through_native_router() {
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

    let ok = MockServer::fixed(Reply::json(
        200,
        serde_json::json!({ "text": "cloud words btw" }),
    ));
    let bad = MockServer::fixed(Reply::json(
        401,
        serde_json::json!({ "error": { "message": "invalid key", "code": "invalid_api_key" } }),
    ));
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/java/dev/flowclone/core/JniCloud.java");
    let out = Command::new("java")
        .arg(format!("-Djava.library.path={}", lib_dir.display()))
        .arg("--enable-native-access=ALL-UNNAMED")
        .arg(&src)
        .arg(ok.url("/openai/v1"))
        .arg(bad.url("/openai/v1"))
        .arg(KEY)
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
    assert_eq!(caps["cloud"], true, "{caps}");
    assert_eq!(caps["adapters"].as_array().unwrap().len(), 5, "{caps}");
    let presets = json(l.next().unwrap());
    let ids: Vec<&str> = presets
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["preset"].as_str().unwrap())
        .collect();
    for want in [
        "openrouter",
        "elevenlabs",
        "groq",
        "openai",
        "deepgram",
        "assemblyai",
    ] {
        assert!(ids.contains(&want), "{ids:?}");
    }

    let probe = json(l.next().unwrap());
    assert_eq!(probe["ok"], true, "{probe}");
    assert_eq!(probe["engine"], "cloud");
    assert_eq!(probe["provider"], "groq-1");
    assert_eq!(probe["transcript"], "cloud words btw");
    let nokey = json(l.next().unwrap());
    assert_eq!(nokey["ok"], false);
    assert_eq!(nokey["error"], "cloud_not_configured", "{nokey}");

    assert_eq!(l.next().unwrap(), "KIND error", "core handle refuses a key");

    let first = json(l.next().unwrap());
    assert_eq!(first["route"]["engine"], "local", "{first}");
    assert_eq!(
        first["route"]["fallback"], "cloud_not_configured",
        "{first}"
    );
    assert_eq!(first["final_text"], "local words by the way ");

    let cloud = json(l.next().unwrap());
    assert_eq!(cloud["status"], "RAW_TRANSCRIPT", "{cloud}");
    assert_eq!(cloud["route"]["engine"], "cloud", "{cloud}");
    assert_eq!(cloud["route"]["provider"], "groq-1");
    assert_eq!(cloud["route"]["model"], "whisper-large-v3-turbo");
    assert!(
        cloud["route"]["billed_audio_ms"].as_u64().unwrap() > 0,
        "{cloud}"
    );
    assert!(cloud["route"]["cost_usd"].is_number(), "{cloud}");
    assert_eq!(
        cloud["route"]["cost_estimated"], true,
        "groq reports no cost"
    );
    assert_eq!(cloud["asr_text"], "cloud words btw");
    assert_eq!(
        cloud["final_text"], "cloud words by the way ",
        "C5: the dictionary ran locally on the provider's text"
    );

    let usage = json(l.next().unwrap());
    assert_eq!(usage["providers"][0]["provider_id"], "groq-1", "{usage}");
    assert_eq!(
        usage["providers"][0]["requests"], 1,
        "another month's counters were dropped: {usage}"
    );
    let status = json(l.next().unwrap());
    assert_eq!(status["last"]["engine"], "cloud", "{status}");
    assert_eq!(status["cloud_circuit"]["open"], false, "{status}");
    let stored = json(l.next().unwrap());
    assert_eq!(stored["ok"], true, "stored key used: {stored}");

    let fb = json(l.next().unwrap());
    assert_eq!(fb["route"]["engine"], "local", "{fb}");
    assert_eq!(fb["route"]["fallback"], "auth_failed", "{fb}");
    assert_eq!(fb["final_text"], "local words by the way ", "{fb}");
    assert_eq!(
        fb["timings"]["audio_ms"], cloud["timings"]["audio_ms"],
        "100 % of the audio replayed"
    );
    let open = json(l.next().unwrap());
    assert_eq!(open["cloud_circuit"]["open"], true, "{open}");
    assert_eq!(open["cloud_circuit"]["reason"], "auth_failed");
    assert_eq!(open["cloud_circuit"]["until_config_change"], true);
    let probe_bad = json(l.next().unwrap());
    assert_eq!(probe_bad["error"], "auth_failed", "{probe_bad}");

    let forgot = json(l.next().unwrap());
    assert_eq!(forgot["route"]["engine"], "local", "{forgot}");
    assert_eq!(forgot["route"]["fallback"], "cloud_not_configured");
    let reset = json(l.next().unwrap());
    assert_eq!(reset["providers"], serde_json::json!([]), "{reset}");
    assert_eq!(l.next().unwrap(), "KIND bad_config");

    let only = json(l.next().unwrap());
    assert_eq!(only["route"]["engine"], "cloud", "{only}");
    assert_eq!(only["final_text"], "cloud words by the way ");

    let want =
        serde_json::json!({"engine":"cloud","provider":"groq-1","model":"whisper-large-v3-turbo"});
    let info = json(l.next().unwrap());
    assert_eq!(info["route"], want, "{info}");
    let poll = json(l.next().unwrap());
    assert_eq!(
        poll["route"], want,
        "pill shows the provider while recording: {poll}"
    );
    let idle = json(l.next().unwrap());
    assert!(idle.get("route").is_none(), "{idle}");

    // What the provider saw: Bearer auth with the key, multipart audio, the
    // dictionary term as the Whisper prompt — and nothing else of ours.
    let reqs = ok.requests();
    assert!(reqs.len() >= 4, "{}", reqs.len());
    let r = reqs
        .iter()
        .find(|r| r.path().ends_with("/audio/transcriptions"))
        .unwrap();
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert_eq!(r.field("model").as_deref(), Some("whisper-large-v3-turbo"));
    assert!(r
        .parts()
        .iter()
        .any(|p| p.name == "file" && !p.data.is_empty()));
    assert_eq!(
        bad.requests().len(),
        2,
        "one dictation + one probe hit the 401 provider"
    );
}
