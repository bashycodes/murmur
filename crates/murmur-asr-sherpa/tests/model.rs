//! Real Parakeet-TDT model on real speech. Skips (passes with a note) when
//! the model dir is missing: `tools/fetch-models.sh --parakeet`, or set
//! FLOW_PARAKEET_DIR.

use std::path::PathBuf;
use std::time::Instant;

use flow_asr_sherpa::{ParakeetConfig, ParakeetEngine, Search};

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

/// Default load (hotwords on): beam + greedy recognizers; no hotwords →
/// greedy, hotwords → beam. The unified-en model dir (survey D23
/// candidate) loads the same way when FLOW_PARAKEET_UNIFIED_DIR is set.
#[test]
fn greedy_without_hotwords_beam_with() {
    let Some(dir) = model_dir() else { return };
    let eng = ParakeetEngine::load(&ParakeetConfig::new(&dir, 4)).expect("load");
    assert!(eng.supports_hotwords() && eng.has_greedy());
    assert_eq!(eng.default_search::<&str>(&[]), Search::Greedy);
    assert_eq!(eng.default_search(&["Kubernetes"]), Search::Beam);
    let pcm = read_wav(&models().join("jfk.wav"));
    for s in [Search::Greedy, Search::Beam] {
        let t = eng.transcribe_with::<&str>(&pcm, &[], s).unwrap();
        assert!(
            t.contains("ask not what your country can do for you"),
            "{s:?}: {t}"
        );
    }
    let mut cfg = ParakeetConfig::new(&dir, 4);
    cfg.greedy_fallback = false;
    let beam_only = ParakeetEngine::load(&cfg).unwrap();
    assert!(!beam_only.has_greedy());
    assert_eq!(beam_only.default_search::<&str>(&[]), Search::Beam);
    if let Some(u) = std::env::var_os("FLOW_PARAKEET_UNIFIED_DIR") {
        let eng = ParakeetEngine::load(&ParakeetConfig::new(PathBuf::from(u), 4)).expect("unified");
        let t = eng.transcribe::<&str>(&pcm, &[]).unwrap();
        assert!(
            t.contains("ask not what your country can do for you"),
            "{t}"
        );
    }
}

/// Parakeet-v3-he int8 (krut42/voice-parakeet-he-int8, pinned in
/// docs/findings/2026-10-30-hebrew-model-pins.md) on the unchanged
/// `nemo_transducer` path. Runs when FLOW_PARAKEET_HE_DIR and FLOW_HE_WAV
/// (16 kHz mono Hebrew wav, int16 or float) are set.
#[test]
fn parakeet_he_loads_on_nemo_path() {
    let (Some(dir), Some(wav)) = (
        std::env::var_os("FLOW_PARAKEET_HE_DIR"),
        std::env::var_os("FLOW_HE_WAV"),
    ) else {
        eprintln!("SKIP: FLOW_PARAKEET_HE_DIR / FLOW_HE_WAV unset");
        return;
    };
    let mut r = hound::WavReader::open(&wav).unwrap();
    let mut pcm: Vec<f32> = match r.spec().sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => r
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
    };
    let peak = pcm.iter().fold(0f32, |m, x| m.max(x.abs()));
    pcm.iter_mut().for_each(|x| *x *= 0.5 / peak.max(1e-6));
    let t = Instant::now();
    let eng = ParakeetEngine::load(&ParakeetConfig::new(PathBuf::from(dir), 4)).expect("load");
    eprintln!("load: {} ms", t.elapsed().as_millis());
    assert!(eng.supports_hotwords());
    for s in [Search::Greedy, Search::Beam] {
        let t = Instant::now();
        let text = eng.transcribe_with::<&str>(&pcm, &[], s).unwrap();
        let secs = t.elapsed().as_secs_f64();
        eprintln!(
            "{s:?}: RTF {:.3}: {text}",
            secs * 16_000.0 / pcm.len() as f64
        );
        let he = text
            .chars()
            .filter(|c| ('\u{05D0}'..='\u{05EA}').contains(c))
            .count();
        assert!(he > 10, "{text}");
    }
}
