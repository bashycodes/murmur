//! Real streaming zipformer (ivrit-ai/zipformer2-66m-streaming-he int8) on
//! real Hebrew speech. Skips (passes with a note) unless both exist:
//! MURMUR_ZIPFORMER_HE_DIR (default models/zipformer2-66m-streaming-he-int8)
//! and MURMUR_HE_WAV (16 kHz mono wav, int16 or float, e.g. a FLEURS he_il
//! dev clip; default models/he.wav). Optional MURMUR_HE_HOTWORD: a phrase
//! the beam decode must then contain (e.g. FLEURS he dev
//! 10012360550181770926.wav with "הארכיפלג": greedy writes "הארכי פלג").
//! Model pin:
//! docs/findings/2026-10-30-hebrew-model-pins.md.

use std::path::PathBuf;
use std::time::Instant;

use murmur_asr_sherpa::{OnlineConfig, OnlineEngine, Search, SherpaTranscriber};

fn models() -> PathBuf {
    std::env::var_os("MURMUR_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn inputs() -> Option<(PathBuf, PathBuf)> {
    let d = std::env::var_os("MURMUR_ZIPFORMER_HE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| models().join("zipformer2-66m-streaming-he-int8"));
    let w = std::env::var_os("MURMUR_HE_WAV")
        .map(PathBuf::from)
        .unwrap_or_else(|| models().join("he.wav"));
    if d.join("tokens.txt").is_file() && w.is_file() {
        Some((d, w))
    } else {
        eprintln!("SKIP: {} or {} missing", d.display(), w.display());
        None
    }
}

fn read_wav(p: &std::path::Path) -> Vec<f32> {
    let mut r = hound::WavReader::open(p).unwrap();
    let spec = r.spec();
    assert_eq!((spec.sample_rate, spec.channels), (16_000, 1));
    match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => r
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
    }
}

fn hebrew_letters(t: &str) -> usize {
    t.chars()
        .filter(|c| ('\u{05D0}'..='\u{05EA}').contains(c))
        .count()
}

#[test]
fn hebrew_clip_greedy_and_beam() {
    let Some((dir, wav)) = inputs() else { return };
    let t = Instant::now();
    let eng = OnlineEngine::load(&OnlineConfig::new(&dir, 4, Some("he"))).expect("load");
    eprintln!("load: {} ms", t.elapsed().as_millis());
    assert!(eng.supports_hotwords() && eng.has_greedy());
    assert_eq!(SherpaTranscriber::language(&eng), Some("he"));
    let mut pcm = read_wav(&wav);
    // Some FLEURS he clips are recorded at RMS < 0.001, below the silence
    // gate; dictation audio is louder. Scale to peak 0.5.
    let peak = pcm.iter().fold(0f32, |m, x| m.max(x.abs()));
    pcm.iter_mut().for_each(|x| *x *= 0.5 / peak.max(1e-6));
    let dur_s = pcm.len() as f64 / 16_000.0;
    let hot: Vec<String> = std::env::var("MURMUR_HE_HOTWORD").into_iter().collect();
    for (search, hw) in [(Search::Greedy, &vec![]), (Search::Beam, &hot)] {
        let t = Instant::now();
        let text = eng.transcribe_with(&pcm, hw, search).unwrap();
        let s = t.elapsed().as_secs_f64();
        eprintln!(
            "{search:?} {hw:?}: {dur_s:.1} s audio, {:.0} ms, RTF {:.3}: {text}",
            s * 1e3,
            s / dur_s
        );
        let letters = text.chars().filter(|c| c.is_alphabetic()).count();
        assert!(letters > 10, "{text}");
        assert!(
            hebrew_letters(&text) * 10 >= letters * 9,
            "not Hebrew script: {text}"
        );
        // Measured 0.05 on a loaded 4-thread i7-6700; generous for CI noise.
        assert!(s / dur_s < 0.5, "RTF {}", s / dur_s);
        if search == Search::Beam {
            for h in hw {
                assert!(text.contains(h.as_str()), "hotword {h} missing: {text}");
            }
        }
    }
    assert_eq!(eng.transcribe::<&str>(&[0.0; 32_000], &[]).unwrap(), "");
    assert_eq!(eng.transcribe::<&str>(&[], &[]).unwrap(), "");
    // A short burst decodes (tail padding covers the encoder's context).
    eng.transcribe::<&str>(&pcm[..4_000], &[]).unwrap();
}

#[test]
fn bad_dir_is_a_load_error() {
    assert!(OnlineEngine::load(&OnlineConfig::new("/nonexistent/zip", 1, None)).is_err());
}
