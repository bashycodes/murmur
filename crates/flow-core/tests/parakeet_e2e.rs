//! End-to-end: real Parakeet-TDT model through the full FlowCore pipeline.
//! Skips (passes with a note) without the model: `tools/fetch-models.sh
//! --parakeet`, or FLOW_PARAKEET_DIR. The hotword case also needs the
//! LibriSpeech clip 5142-36377-0008.wav in FLOW_AUDIO_DIR (default models/).

use std::path::PathBuf;
use std::time::Instant;

use flow_core::config::{AsrBackend, ModelConfig, SessionConfig};
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_core::{audio, CommitStatus, FlowCore};

fn models() -> PathBuf {
    std::env::var_os("FLOW_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn load() -> Option<FlowCore> {
    let dir = std::env::var_os("FLOW_PARAKEET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| models().join("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"));
    if !dir.join("tokens.txt").is_file() || !models().join("jfk.wav").is_file() {
        eprintln!("SKIP: {} or jfk.wav missing", dir.display());
        return None;
    }
    let t = Instant::now();
    let cfg: ModelConfig = serde_json::from_value(serde_json::json!({
        "asr_engine": "parakeet",
        "asr_path": dir,
        "threads": 4,
    }))
    .unwrap();
    assert_eq!(cfg.asr_backend, AsrBackend::Parakeet);
    let core = FlowCore::load_models(&cfg).expect("load parakeet");
    eprintln!("parakeet load_models: {} ms", t.elapsed().as_millis());
    Some(core)
}

fn dictate(core: &FlowCore, wav: &std::path::Path, cfg: SessionConfig) -> flow_core::CommitResult {
    let pcm = audio::read_wav_16k_mono(wav).unwrap();
    core.start_session(cfg).unwrap();
    for c in pcm.chunks(640) {
        assert!(core.push_audio(c));
    }
    let r = core.commit();
    eprintln!(
        "audio {} ms | asr {} ms | {:?}: {}",
        r.timings.audio_ms, r.timings.asr_ms, r.status, r.final_text
    );
    r
}

#[test]
fn parakeet_jfk_end_to_end_with_dictionary_and_hotwords() {
    let Some(core) = load() else { return };
    let r = dictate(
        &core,
        &models().join("jfk.wav"),
        SessionConfig {
            dictionary: Dictionary {
                entries: vec![DictEntry::replacement(
                    "fellow americans",
                    "fellow Americans!",
                )],
            },
            ..Default::default()
        },
    );
    assert_eq!(r.status, CommitStatus::RawTranscript, "{r:?}");
    assert_eq!(r.language.as_deref(), Some("en"));
    assert!(
        r.asr_text
            .starts_with("And so, my fellow Americans, ask not what your country can do for you"),
        "{}",
        r.asr_text
    );
    assert!(
        r.final_text.contains("fellow Americans!"),
        "{}",
        r.final_text
    );
    // RTF guard, loose: the bench measured 0.12-0.14 on a quiet box; allow a
    // heavily shared one.
    assert!(r.timings.asr_ms < 11_000, "asr {} ms", r.timings.asr_ms);

    // Contextual biasing: the dictionary word reaches sherpa as a hotword.
    let clip = std::env::var_os("FLOW_AUDIO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(models)
        .join("5142-36377-0008.wav");
    if !clip.is_file() {
        eprintln!("SKIP hotword case: {} missing", clip.display());
        return;
    }
    let plain = dictate(&core, &clip, SessionConfig::default());
    assert!(plain.asr_text.contains("Meadocroft"), "{}", plain.asr_text);
    let biased = dictate(
        &core,
        &clip,
        SessionConfig {
            dictionary: Dictionary {
                entries: vec![DictEntry::word("Meadowcroft")],
            },
            ..Default::default()
        },
    );
    assert!(
        biased.asr_text.contains("Meadowcroft"),
        "{}",
        biased.asr_text
    );
}

#[test]
fn parakeet_silence_is_empty() {
    let Some(core) = load() else { return };
    core.start_session(SessionConfig::default()).unwrap();
    core.push_audio(&[0i16; 32_000]);
    assert_eq!(core.commit().status, CommitStatus::Empty);
}

#[test]
fn parakeet_bad_dir_fails_load() {
    let r = FlowCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Parakeet,
        asr_path: "/nonexistent/parakeet".into(),
        ..Default::default()
    });
    match r {
        Err(flow_core::CoreError::ModelNotFound { path }) => {
            assert_eq!(path, "/nonexistent/parakeet")
        }
        Err(e) => panic!("wrong error: {e:?}"),
        Ok(_) => panic!("loaded a missing model dir"),
    }
}
