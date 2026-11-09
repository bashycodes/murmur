//! End-to-end: real whisper.cpp model + real speech through the full
//! MurmurCore pipeline. Skips (passes with a note) when the model or sample
//! is missing; fetch them with `tools/fetch-models.sh`.

use std::path::PathBuf;

use murmur_core::config::{AsrBackend, ModelConfig, SessionConfig};
use murmur_core::dictionary::{DictEntry, Dictionary};
use murmur_core::{audio, CommitStatus, MurmurCore};

fn models_dir() -> PathBuf {
    std::env::var_os("MURMUR_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn fixture() -> Option<(PathBuf, PathBuf)> {
    let d = models_dir();
    let model =
        d.join(std::env::var("MURMUR_TEST_MODEL").unwrap_or_else(|_| "ggml-tiny.en.bin".into()));
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
    let core = MurmurCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Whisper,
        asr_path: model.to_string_lossy().into(),
        threads: std::env::var("MURMUR_THREADS")
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

#[test]
fn missing_silero_model_is_model_not_found() {
    let Some((model, _)) = fixture() else {
        return;
    };
    let r = MurmurCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Whisper,
        asr_path: model.to_string_lossy().into(),
        vad_model_path: Some("/nonexistent/ggml-silero.bin".into()),
        ..Default::default()
    });
    match r {
        Err(murmur_core::CoreError::ModelNotFound { path }) => {
            assert_eq!(path, "/nonexistent/ggml-silero.bin")
        }
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("loaded"),
    }
}

/// Runs only when a Silero model is present (`MURMUR_SILERO_MODEL` or
/// `models/ggml-silero-v5.1.2.bin`; never downloaded by tests).
#[test]
fn silero_vad_keeps_the_speech_if_model_present() {
    let Some((model, wav)) = fixture() else {
        return;
    };
    let silero = std::env::var_os("MURMUR_SILERO_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| models_dir().join("ggml-silero-v5.1.2.bin"));
    if !silero.exists() {
        eprintln!("SKIP: {} missing", silero.display());
        return;
    }
    let core = MurmurCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Whisper,
        asr_path: model.to_string_lossy().into(),
        vad_model_path: Some(silero.to_string_lossy().into()),
        ..Default::default()
    })
    .unwrap();
    core.start_session(SessionConfig::default()).unwrap();
    core.push_audio(&audio::read_wav_16k_mono(&wav).unwrap());
    let r = core.commit();
    assert!(r.asr_text.to_lowercase().contains("ask not"), "{r:?}");
}

/// D20: the encoder window sized to the clip gives the same words as the
/// full 30 s window, on the whole clip and on a short (padded) piece.
#[test]
fn fit_ctx_matches_the_full_window() {
    use murmur_core::asr::{AsrEngine, AsrRequest};
    use murmur_core::whisper::WhisperEngine;
    let Some((model, wav)) = fixture() else {
        return;
    };
    let pcm: Vec<f32> = audio::read_wav_16k_mono(&wav)
        .unwrap()
        .iter()
        .map(|&s| s as f32 / 32768.0)
        .collect();
    let words = |s: &str| {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let req = AsrRequest::default();
    for clip in [&pcm[..], &pcm[..16_000 * 3]] {
        let full = WhisperEngine::load(model.to_str().unwrap(), 4)
            .unwrap()
            .with_fit_ctx(false)
            .transcribe(clip, &req)
            .unwrap()
            .text;
        let fit = WhisperEngine::load(model.to_str().unwrap(), 4)
            .unwrap()
            .with_fit_ctx(true);
        let text = fit.transcribe(clip, &req).unwrap().text;
        eprintln!(
            "full {full:?} | fit {text:?} | re-decodes {}",
            fit.redecodes()
        );
        assert_eq!(words(&text), words(&full));
    }
}

/// Regression (agent W): segmented pieces must not add sentence breaks at
/// mid-sentence pauses; jfk segmented == whole clip, punctuation included.
#[test]
fn segmented_jfk_equals_whole_clip() {
    let Some((model, wav)) = fixture() else {
        return;
    };
    let core = MurmurCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Whisper,
        asr_path: model.to_string_lossy().into(),
        threads: 4,
        ..Default::default()
    })
    .unwrap();
    let pcm = audio::read_wav_16k_mono(&wav).unwrap();
    let run = |cfg: SessionConfig| {
        core.start_session(cfg).unwrap();
        for c in pcm.chunks(640) {
            core.push_audio(c);
        }
        core.commit()
    };
    let whole = run(SessionConfig::default());
    let seg = run(SessionConfig {
        segment_pause_ms: Some(800),
        ..Default::default()
    });
    eprintln!("whole: {}\nseg:   {}", whole.asr_text, seg.asr_text);
    assert!(seg.timings.segments >= 2, "no cut: {:?}", seg.timings);
    assert_eq!(seg.final_text, whole.final_text);
}
