//! End-to-end: the real streaming zipformer (ivrit-ai/zipformer2-66m-
//! streaming-he int8, `asr_backend: "sherpa_online"`) through MurmurCore.
//! Skips (passes with a note) unless both exist:
//! MURMUR_ZIPFORMER_HE_DIR (default models/zipformer2-66m-streaming-he-int8,
//! then ~/.cache/murmur-models/zipformer2-66m-streaming-he-int8) and
//! MURMUR_HE_WAV (default models/he.wav, then the FLEURS he dev clip
//! 10012360550181770926.wav under ~/.cache/murmur-models/
//! fleurs-he-sample/raw/). The hotword case expects that clip's
//! "הארכיפלג" (greedy writes "הארכי פלג"). Model pin:
//! docs/findings/2026-10-30-hebrew-model-pins.md.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use murmur_core::config::{AsrBackend, LanguageModel, ModelConfig, SessionConfig};
use murmur_core::dictionary::{DictEntry, Dictionary};
use murmur_core::{audio, CommitResult, CommitStatus, MurmurCore};

const HOTWORD: &str = "הארכיפלג";

fn models() -> PathBuf {
    std::env::var_os("MURMUR_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn cache() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/murmur-models"))
}

/// The env var, else the first candidate that exists.
fn pick(var: &str, candidates: Vec<PathBuf>, ok: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(var) {
        return Some(PathBuf::from(p));
    }
    candidates.into_iter().find(|p| ok(p))
}

fn inputs() -> Option<(PathBuf, PathBuf)> {
    let zip = "zipformer2-66m-streaming-he-int8";
    let dir = pick(
        "MURMUR_ZIPFORMER_HE_DIR",
        [Some(models().join(zip)), cache().map(|c| c.join(zip))]
            .into_iter()
            .flatten()
            .collect(),
        |p| p.join("tokens.txt").is_file(),
    );
    let wav = pick(
        "MURMUR_HE_WAV",
        [
            Some(models().join("he.wav")),
            cache().map(|c| c.join("fleurs-he-sample/raw/10012360550181770926.wav")),
        ]
        .into_iter()
        .flatten()
        .collect(),
        |p| p.is_file(),
    );
    match (dir, wav) {
        (Some(d), Some(w)) if d.join("tokens.txt").is_file() && w.is_file() => Some((d, w)),
        (d, w) => {
            eprintln!("SKIP: zipformer dir {d:?} or Hebrew wav {w:?} missing");
            None
        }
    }
}

/// The clip scaled to peak 0.5: some FLEURS he clips are recorded below
/// the engines' silence gate; dictation audio is louder.
fn clip(wav: &Path) -> Vec<i16> {
    let pcm = audio::read_wav_16k_mono(wav).unwrap();
    let peak = pcm
        .iter()
        .map(|s| (*s as i32).abs())
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    pcm.iter()
        .map(|s| (*s as f32 * 16_384.0 / peak) as i16)
        .collect()
}

fn dictate(core: &MurmurCore, pcm: &[i16], cfg: SessionConfig) -> CommitResult {
    core.start_session(cfg).unwrap();
    for c in pcm.chunks(640) {
        assert!(core.push_audio(c));
    }
    let r = core.commit();
    eprintln!(
        "audio {} ms | asr {} ms | {:?} [{:?}]: {}",
        r.timings.audio_ms, r.timings.asr_ms, r.status, r.language, r.final_text
    );
    r
}

fn hebrew(lang: &str, words: &[&str]) -> SessionConfig {
    SessionConfig {
        languages: vec![lang.into()],
        dictionary: Dictionary {
            entries: words.iter().map(|w| DictEntry::word(w)).collect(),
        },
        ..Default::default()
    }
}

fn assert_hebrew(text: &str) {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let he = text
        .chars()
        .filter(|c| ('\u{05D0}'..='\u{05EA}').contains(c))
        .count();
    assert!(letters > 10 && he * 10 >= letters * 9, "not Hebrew: {text}");
}

#[test]
fn zipformer_he_route_through_murmur_core() {
    let Some((dir, wav)) = inputs() else { return };
    // English default (mock) + the zipformer as the Hebrew route.
    let cfg: ModelConfig = serde_json::from_value(serde_json::json!({
        "asr_backend": {"mock": {"text": "english words"}},
        "threads": 4,
        "languages": {"he": {"asr_backend": "sherpa_online", "asr_path": dir}},
    }))
    .unwrap();
    assert_eq!(cfg.languages["he"].asr_backend, AsrBackend::SherpaOnline);
    let t = Instant::now();
    let core = MurmurCore::load_models(&cfg).expect("load zipformer");
    eprintln!("load_models: {} ms", t.elapsed().as_millis());
    let pcm = clip(&wav);

    let r = dictate(&core, &pcm, hebrew("he-IL", &[]));
    assert_ne!(r.status, CommitStatus::Error, "{r:?}");
    assert_ne!(r.status, CommitStatus::Empty, "{r:?}");
    assert_eq!(r.language.as_deref(), Some("he"));
    assert_hebrew(&r.asr_text);
    assert_hebrew(&r.final_text);
    // RTF measured 0.05; generous for a shared box.
    assert!(
        (r.timings.asr_ms as f64) < r.timings.audio_ms as f64 * 0.5,
        "asr {} ms",
        r.timings.asr_ms
    );

    // Dictionary words reach the zipformer as hotwords.
    if wav
        .file_name()
        .is_some_and(|f| f == "10012360550181770926.wav")
    {
        assert!(!r.asr_text.contains(HOTWORD), "{}", r.asr_text);
        let b = dictate(&core, &pcm, hebrew("he", &[HOTWORD]));
        assert!(b.asr_text.contains(HOTWORD), "{}", b.asr_text);
    }

    // Auto-detect and English stay on the default model.
    let e = dictate(&core, &pcm, SessionConfig::default());
    assert_eq!(e.asr_text, "english words");

    // Silence is empty.
    core.start_session(hebrew("he", &[])).unwrap();
    core.push_audio(&[0i16; 32_000]);
    assert_eq!(core.commit().status, CommitStatus::Empty);
}

/// FLEURS he_il dev 10043827527204102024.wav (13.3 s). Its reference
/// starts "הפארק הלאומי קרוגר (KNP) נמצא ...", but the reader starts at
/// "נמצא" (~1.1 s, after ~1 s of quiet breath) and says "הפארק הלאומי
/// קרוגר" near the end (~9.5-11 s), before "קרוקודייל". The 2026-10-31
/// server e2e scored that as "first 3 words dropped";
/// docs/findings/2026-10-31-zipformer-first-words.md.
fn kruger_clip() -> Option<PathBuf> {
    pick(
        "MURMUR_HE_KRUGER_WAV",
        cache()
            .map(|c| c.join("fleurs-he-sample/raw/10043827527204102024.wav"))
            .into_iter()
            .collect(),
        |p| p.is_file(),
    )
    .filter(|p| p.is_file())
}

#[test]
fn zipformer_keeps_every_spoken_word_of_the_kruger_clip() {
    let (Some((dir, _)), Some(wav)) = (inputs(), kruger_clip()) else {
        eprintln!("SKIP: zipformer or FLEURS he 10043827527204102024.wav missing");
        return;
    };
    let pcm = clip(&wav);
    // VAD keeps the lead-in: the trimmed span starts before the first word.
    let f: Vec<f32> = pcm.iter().map(|s| *s as f32 / 32768.0).collect();
    let sp = murmur_core::vad::detect(&f).expect("speech");
    assert!(sp.start <= 16_000, "VAD start {} samples", sp.start);

    let cfg = ModelConfig {
        asr_backend: AsrBackend::SherpaOnline,
        asr_path: dir.display().to_string(),
        threads: 4,
        ..Default::default()
    };
    let core = MurmurCore::load_models(&cfg).expect("load zipformer");
    let r = dictate(&core, &pcm, hebrew("he", &[]));
    let t = &r.asr_text;
    // Spoken order: "נמצא בחלק ..." first, "... הלאומי קרוגר קרוקודייל" last.
    let first = t
        .find("נמצא")
        .unwrap_or_else(|| panic!("first word lost: {t}"));
    let kruger = t
        .find("הלאומי קרוגר")
        .unwrap_or_else(|| panic!("park name lost: {t}"));
    let croc = t
        .find("קרוקודייל")
        .unwrap_or_else(|| panic!("last word lost: {t}"));
    assert!(first < kruger && kruger < croc, "{t}");
    assert!(t.split_whitespace().count() >= 20, "{t}");
}

#[test]
fn zipformer_as_the_default_model_reports_the_forced_language() {
    let Some((dir, wav)) = inputs() else { return };
    let cfg = ModelConfig {
        asr_backend: AsrBackend::SherpaOnline,
        asr_path: dir.display().to_string(),
        threads: 4,
        ..Default::default()
    };
    let core = MurmurCore::load_models(&cfg).expect("load zipformer");
    let r = dictate(&core, &clip(&wav), hebrew("iw", &[]));
    assert_eq!(r.language.as_deref(), Some("he"));
    assert_hebrew(&r.asr_text);
}

#[test]
fn sherpa_online_bad_dir_fails_load() {
    let cfg = ModelConfig {
        asr_backend: AsrBackend::Mock { text: "x".into() },
        languages: BTreeMap::from([(
            "he".to_string(),
            LanguageModel {
                asr_backend: AsrBackend::SherpaOnline,
                asr_path: "/nonexistent/zipformer".into(),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    match MurmurCore::load_models(&cfg) {
        Err(murmur_core::CoreError::ModelNotFound { path }) => {
            assert_eq!(path, "/nonexistent/zipformer")
        }
        Err(e) => panic!("wrong error: {e:?}"),
        Ok(_) => panic!("loaded a missing model dir"),
    }
}
