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
