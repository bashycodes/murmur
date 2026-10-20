//! flow-cli is the end-to-end consumer: WAV in -> CommitResult JSON out.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flow-cli"))
}

fn write_wav(path: &PathBuf, secs: f32, rate: u32, channels: u16) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..(secs * rate as f32) as usize {
        let s = ((i as f32 * 0.05).sin() * 8000.0) as i16;
        for _ in 0..channels {
            w.write_sample(s).unwrap();
        }
    }
    w.finalize().unwrap();
}

fn tmp(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    d.join(name)
}

#[test]
fn mock_backend_outputs_commit_json() {
    let wav = tmp("tone.wav");
    write_wav(&wav, 1.5, 48_000, 2);
    let out = bin()
        .args(["transcribe", "--mock", "hello world", "--json"])
        .arg(&wav)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "RAW_TRANSCRIPT");
    assert_eq!(v["asr_text"], "hello world");
    assert_eq!(v["timings"]["audio_ms"], 1500);
}

#[test]
fn dictionary_file_is_applied() {
    let wav = tmp("tone2.wav");
    write_wav(&wav, 1.0, 16_000, 1);
    let dict = tmp("dict.json");
    std::fs::write(
        &dict,
        r#"[{"word":"btw","replacement":"by the way","isSnippet":false}]"#,
    )
    .unwrap();
    let out = bin()
        .args(["transcribe", "--mock", "ok btw", "--json", "--dict"])
        .arg(&dict)
        .arg(&wav)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["final_text"].as_str().unwrap().contains("ok by the way"),
        "{v}"
    );
}

#[test]
fn plain_output_is_final_text() {
    let wav = tmp("tone3.wav");
    write_wav(&wav, 1.0, 16_000, 1);
    let out = bin()
        .args(["transcribe", "--mock", "Hi there."])
        .arg(&wav)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "Hi there.");
}

#[test]
fn usage_error_exits_nonzero() {
    let out = bin().arg("bogus").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}

#[test]
fn real_model_on_jfk_if_present() {
    let m = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let (model, wav) = (m.join("ggml-tiny.en.bin"), m.join("jfk.wav"));
    if !model.exists() || !wav.exists() {
        eprintln!("SKIP: model/sample missing (tools/fetch-models.sh)");
        return;
    }
    let out = bin()
        .args(["transcribe", "--json", "--model"])
        .arg(&model)
        .arg(&wav)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["asr_text"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("ask not what your country"),
        "{v}"
    );
}

#[test]
fn clean_needs_llm_and_text() {
    let out = bin().args(["clean", "hello"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}

#[test]
fn clean_with_real_llm_if_present() {
    let model = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../models/qwen2.5-0.5b-instruct-q4_k_m.gguf");
    // `cargo test` builds the plugin into deps/ without copying it up.
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_flow-cli"));
    let plugin = exe.parent().unwrap().join("deps/libflow_llm.so");
    if !model.is_file() || !plugin.is_file() {
        eprintln!("SKIP: LLM model or plugin missing (tools/fetch-models.sh --llm)");
        return;
    }
    let session = tmp("llm-session.json");
    std::fs::write(&session, r#"{"llm_timeout_ms":30000}"#).unwrap();
    let out = bin()
        .args(["clean", "--json", "--llm"])
        .arg(&model)
        .arg("--llm-lib")
        .arg(&plugin)
        .arg("--session")
        .arg(&session)
        .args([
            "i i think the the problem is in the cache layer",
            "hello there",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    for v in &lines {
        assert_eq!(v["result"]["status"], "FORMATTED", "{v}");
        assert!(v["llm"]["gen_tokens"].as_u64().unwrap() > 0, "{v}");
        // Warm KV prefix: only the tail of the prompt is prefilled.
        assert!(
            v["llm"]["prefill_tokens"].as_u64().unwrap() * 4
                < v["llm"]["prompt_tokens"].as_u64().unwrap(),
            "{v}"
        );
    }
}

/// whisper.cpp (linked) and llama.cpp (plugin) bring different ggml builds;
/// both must work in one process for the real pipeline.
#[test]
fn whisper_then_llm_in_one_process_if_present() {
    let m = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
    let (asr, llm, wav) = (
        m.join("ggml-tiny.en.bin"),
        m.join("qwen2.5-0.5b-instruct-q4_k_m.gguf"),
        m.join("jfk.wav"),
    );
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_flow-cli"));
    let plugin = exe.parent().unwrap().join("deps/libflow_llm.so");
    if ![&asr, &llm, &wav, &plugin].iter().all(|p| p.is_file()) {
        eprintln!("SKIP: whisper/LLM model, sample or plugin missing");
        return;
    }
    let session = tmp("llm-session2.json");
    std::fs::write(&session, r#"{"llm_timeout_ms":30000}"#).unwrap();
    let out = bin()
        .args(["transcribe", "--json", "--model"])
        .arg(&asr)
        .arg("--llm")
        .arg(&llm)
        .arg("--llm-lib")
        .arg(&plugin)
        .arg("--session")
        .arg(&session)
        .arg(&wav)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "FORMATTED", "{v}");
    assert!(v["timings"]["llm_ms"].as_u64().unwrap() > 0);
    let t = v["final_text"].as_str().unwrap().to_lowercase();
    assert!(t.contains("ask not what your country"), "{v}");
}
