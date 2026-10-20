//! Real Parakeet-TDT model on real speech. Skips (passes with a note) when
//! the model dir is missing: `tools/fetch-models.sh --parakeet`, or set
//! FLOW_PARAKEET_DIR.

use std::path::PathBuf;
use std::time::Instant;

use flow_asr_sherpa::{ParakeetConfig, ParakeetEngine};

fn models() -> PathBuf {
    std::env::var_os("FLOW_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn model_dir() -> Option<PathBuf> {
    let d = std::env::var_os("FLOW_PARAKEET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| models().join("sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"));
    if d.join("tokens.txt").is_file() && models().join("jfk.wav").is_file() {
        Some(d)
    } else {
        eprintln!("SKIP: {} or models/jfk.wav missing", d.display());
        None
    }
}

fn read_wav(p: &std::path::Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(p).unwrap();
    assert_eq!(r.spec().sample_rate, 16_000);
    r.samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect()
}

#[test]
fn jfk_and_silence() {
    let Some(dir) = model_dir() else { return };
    let t = Instant::now();
    let eng = ParakeetEngine::load(&ParakeetConfig::new(&dir, 4)).expect("load");
    eprintln!("load: {} ms", t.elapsed().as_millis());
    assert!(eng.supports_hotwords());
    let pcm = read_wav(&models().join("jfk.wav"));
    let t = Instant::now();
    let text = eng.transcribe::<&str>(&pcm, &[]).unwrap();
    let ms = t.elapsed().as_millis();
    eprintln!("jfk: {ms} ms, RTF {:.3}: {text}", ms as f64 / 11_000.0);
    assert!(
        text.starts_with("And so, my fellow Americans, ask not what your country can do for you"),
        "{text}"
    );
    assert_eq!(eng.transcribe::<&str>(&[0.0; 32_000], &[]).unwrap(), "");
    assert_eq!(eng.transcribe::<&str>(&[], &[]).unwrap(), "");
    // A short burst is padded rather than rejected.
    eng.transcribe::<&str>(&pcm[..4_000], &[]).unwrap();
}

#[test]
fn bad_dir_is_a_load_error() {
    let e = ParakeetEngine::load(&ParakeetConfig::new("/nonexistent/parakeet", 1));
    assert!(e.is_err());
}
