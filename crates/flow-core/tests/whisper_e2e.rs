//! End-to-end: real whisper.cpp model + real speech through the full
//! FlowCore pipeline. Skips (passes with a note) when the model or sample
//! is missing; fetch them with `tools/fetch-models.sh`.

use std::path::PathBuf;

use flow_core::config::{AsrBackend, ModelConfig, SessionConfig};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{audio, CommitStatus, FlowCore};

fn models_dir() -> PathBuf {
    std::env::var_os("FLOW_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn fixture() -> Option<(PathBuf, PathBuf)> {
    let d = models_dir();
    let model =
        d.join(std::env::var("FLOW_TEST_MODEL").unwrap_or_else(|_| "ggml-tiny.en.bin".into()));
    let wav = d.join("jfk.wav");
    if model.exists() && wav.exists() {
        Some((model, wav))
    } else {
        eprintln!(
            "SKIP: {} or {} missing (run tools/fetch-models.sh)",
            model.display(),
            wav.display()
        );
        None
    }
}

#[test]
fn jfk_transcribes_end_to_end() {
    let Some((model, wav)) = fixture() else {
        return;
    };
    let core = FlowCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Whisper,
        asr_path: model.to_string_lossy().into(),
        threads: std::env::var("FLOW_THREADS")
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(4),
        ..Default::default()
    })
    .expect("load model");
    let pcm = audio::read_wav_16k_mono(&wav).unwrap();
    core.start_session(SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::replacement(
                "fellow americans",
                "fellow Americans!",
            )],
        },
        ..Default::default()
    })
    .unwrap();
    for c in pcm.chunks(640) {
        core.push_audio(c);
    }
    let r = core.commit();
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap());
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    let lower = r.asr_text.to_lowercase();
    assert!(
        lower.contains("ask not what your country can do for you"),
        "{}",
        r.asr_text
    );
    assert!(
        r.final_text.contains("fellow Americans!"),
        "dictionary applied: {}",
        r.final_text
    );
    assert_eq!(r.language.as_deref(), Some("en"));
    assert!(r.timings.audio_ms > 10_000);
}
