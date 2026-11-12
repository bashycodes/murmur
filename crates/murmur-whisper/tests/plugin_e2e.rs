//! The built plugin (this crate's cdylib, CPU build) through murmur-core's
//! loader, against the built-in engine on the same clip. Skips (passes
//! with a note) when the model or sample is missing; fetch them with
//! `tools/fetch-models.sh` (or point `MURMUR_MODELS_DIR` at them), or when
//! the cdylib was not built first (`cargo build -p murmur-whisper`).

use std::path::PathBuf;

use murmur_core::asr::{AsrEngine, AsrRequest};
use murmur_core::whisper::WhisperEngine;
use murmur_core::whisper_plugin::{probe_backend, PluginOptions, PluginWhisper};

fn models_dir() -> PathBuf {
    std::env::var_os("MURMUR_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

/// The cdylib cargo built next to the test binary's `deps/` folder.
fn plugin() -> PathBuf {
    let dir = std::env::current_exe().unwrap();
    let profile = dir.parent().unwrap().parent().unwrap();
    let name = if cfg!(windows) {
        "murmur_whisper.dll"
    } else if cfg!(target_os = "macos") {
        "libmurmur_whisper.dylib"
    } else {
        "libmurmur_whisper.so"
    };
    profile.join(name)
}

#[test]
fn plugin_transcribes_like_the_built_in_engine() {
    let d = models_dir();
    let (model, wav) = (d.join("ggml-tiny.en.bin"), d.join("jfk.wav"));
    if !model.exists() || !wav.exists() {
        eprintln!("SKIP: {} or {} missing", model.display(), wav.display());
        return;
    }
    let lib = plugin();
    if !lib.exists() {
        // `cargo test` builds the rlib only; `cargo build -p murmur-whisper`
        // (same profile) first builds the cdylib.
        eprintln!(
            "SKIP: {} not built (cargo build -p murmur-whisper)",
            lib.display()
        );
        return;
    }
    let lib = lib.display().to_string();

    let (_, b) = probe_backend(Some(&lib)).unwrap();
    assert_eq!(b.backend, "CPU", "{b:?}");

    let pcm: Vec<f32> = murmur_core::audio::read_wav_16k_mono(&wav)
        .unwrap()
        .iter()
        .map(|&s| s as f32 / 32768.0)
        .collect();
    let opts = PluginOptions {
        threads: 2,
        fit_ctx: true,
        vad_model: None,
    };
    let p = PluginWhisper::load(model.to_str().unwrap(), &opts, Some(&lib)).unwrap();
    assert!(!p.backend().is_gpu(), "{}", p.info);
    let req = AsrRequest {
        initial_prompt: "Americans".into(),
        ..AsrRequest::default()
    };
    let got = p.transcribe(&pcm, &req).unwrap();
    assert!(
        got.text
            .contains("ask not what your country can do for you"),
        "{got:?}"
    );
    assert_eq!(got.language.as_deref(), Some("en"));
    assert!(got.no_speech_prob.is_some());

    let built_in = WhisperEngine::load(model.to_str().unwrap(), 2)
        .unwrap()
        .with_fit_ctx(true);
    assert_eq!(built_in.transcribe(&pcm, &req).unwrap().text, got.text);

    // A set stop flag aborts the decode (an error or an empty result, never a hang).
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let stopped = AsrRequest {
        abort: murmur_core::asr::AbortFlag(Some(flag)),
        ..req
    };
    if let Ok(o) = p.transcribe(&pcm, &stopped) {
        assert!(!o.text.contains("country can do for you"), "{o:?}");
    }
}

/// D84: `ModelConfig::whisper_lib` makes MurmurCore load whisper models
/// through the plugin (murmur-server's CUDA image), and the load says where
/// the model runs. The CPU build here finds no GPU: on the CPU, no error.
#[test]
fn murmur_core_loads_whisper_through_the_configured_plugin() {
    use murmur_core::config::{AsrBackend, ModelConfig, SessionConfig};
    use murmur_core::{CommitStatus, MurmurCore};
    let model = models_dir().join("ggml-tiny.en.bin");
    let lib = plugin();
    if !model.exists() || !lib.exists() {
        eprintln!(
            "SKIP: {} or {} missing (cargo build -p murmur-whisper)",
            model.display(),
            lib.display()
        );
        return;
    }
    let core = MurmurCore::load_models(&ModelConfig {
        asr_backend: AsrBackend::Whisper,
        asr_path: model.to_string_lossy().into(),
        threads: 2,
        whisper_lib: Some(lib.display().to_string()),
        ..Default::default()
    })
    .unwrap();
    let asr = &core.load_timings().models[0];
    assert_eq!(
        (asr.device.as_deref(), asr.gpu_error.as_deref()),
        (Some("CPU"), None),
        "{asr:?}"
    );
    // A synthetic tone decodes without an error (the words do not matter).
    core.start_session(SessionConfig::default()).unwrap();
    let tone: Vec<i16> = (0..16_000 * 2)
        .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 8_000.0) as i16)
        .collect();
    core.push_audio(&tone);
    let r = core.commit();
    assert_ne!(r.status, CommitStatus::Error, "{r:?}");
}
