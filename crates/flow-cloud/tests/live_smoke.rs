//! Live provider smoke tests (docs/cloud-stt.md §10, DoD 2). Each runs only
//! when `FLOW_CLOUD_LIVE_<PRESET>_KEY` is set (`OPENROUTER`, `ELEVENLABS`,
//! `GROQ`, `OPENAI`, `DEEPGRAM`, `ASSEMBLYAI`), optional
//! `FLOW_CLOUD_LIVE_<PRESET>_MODEL`; otherwise it prints a skip line and
//! passes. Audio: `jfk.wav` from `FLOW_MODELS_DIR` (or `models/`). Never
//! part of default CI: it sends audio to a third party and costs money.

use std::time::{Duration, Instant};

use flow_cloud::CloudBackend;
use flow_core::config::SessionConfig;
use flow_core::dictionary::{DictEntry, Dictionary};
use flow_router::{Backend, SecretString};

const PRESETS: [(&str, &str); 6] = [
    ("openrouter", "OPENROUTER"),
    ("elevenlabs", "ELEVENLABS"),
    ("groq", "GROQ"),
    ("openai", "OPENAI"),
    ("deepgram", "DEEPGRAM"),
    ("assemblyai", "ASSEMBLYAI"),
];

const REFERENCE: &str = "and so my fellow americans ask not what your country can do for you \
                         ask what you can do for your country";

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Word error rate (Levenshtein over words / reference length).
fn wer(reference: &str, hyp: &str) -> f64 {
    let (r, h) = (words(reference), words(hyp));
    let mut d: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut prev = d[0];
        d[0] = i;
        for j in 1..=h.len() {
            let cur = d[j];
            d[j] = if r[i - 1] == h[j - 1] {
                prev
            } else {
                1 + prev.min(d[j]).min(d[j - 1])
            };
            prev = cur;
        }
    }
    d[h.len()] as f64 / r.len().max(1) as f64
}

fn jfk() -> Option<Vec<i16>> {
    let dirs = [
        std::env::var("FLOW_MODELS_DIR").unwrap_or_default(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../models").to_string(),
    ];
    let p = dirs
        .iter()
        .filter(|d| !d.is_empty())
        .map(|d| std::path::Path::new(d).join("jfk.wav"))
        .find(|p| p.is_file())?;
    let mut r = hound::WavReader::open(p).ok()?;
    Some(r.samples::<i16>().map(|s| s.unwrap()).collect())
}

fn smoke(preset: &str, env: &str) {
    let Ok(key) = std::env::var(format!("FLOW_CLOUD_LIVE_{env}_KEY")) else {
        eprintln!("SKIP live {preset}: FLOW_CLOUD_LIVE_{env}_KEY not set");
        return;
    };
    let Some(pcm) = jfk() else {
        eprintln!("SKIP live {preset}: jfk.wav not found (FLOW_MODELS_DIR)");
        return;
    };
    let mut p = flow_cloud::presets()
        .into_iter()
        .find(|p| p.preset == preset)
        .unwrap();
    p.id = format!("{preset}-live");
    if let Ok(m) = std::env::var(format!("FLOW_CLOUD_LIVE_{env}_MODEL")) {
        p.model = m;
    }
    let b = CloudBackend::new(p.clone(), SecretString::new(key), None);
    let cfg = SessionConfig {
        dictionary: Dictionary {
            entries: vec![DictEntry::word("Meadowcroft")],
        },
        ..Default::default()
    };
    b.start_session(&cfg).unwrap();
    for c in pcm.chunks(1600) {
        b.push_audio(c);
    }
    let t = Instant::now();
    let r = b
        .commit(Instant::now() + Duration::from_secs(30))
        .unwrap_or_else(|e| panic!("{preset}: {e}"));
    let ms = t.elapsed().as_millis();
    let w = wer(REFERENCE, &r.asr_text);
    let info = b.last_call().unwrap();
    eprintln!(
        "LIVE {preset} model={} commit_ms={ms} upload+provider_ms={} bytes={} billed_ms={} cost_usd={:?} estimated={} wer={w:.3} text={:?}",
        p.model, info.latency_ms, info.bytes, info.billed_audio_ms, info.cost_usd, info.cost_estimated, r.asr_text
    );
    assert!(w <= 0.10, "{preset}: WER {w:.3} > 10 %: {:?}", r.asr_text);
}

#[test]
fn wer_helper() {
    assert_eq!(wer(REFERENCE, REFERENCE), 0.0);
    assert!((wer("a b c d", "a x c") - 0.5).abs() < 1e-9);
}

#[test]
fn live_openrouter() {
    smoke(PRESETS[0].0, PRESETS[0].1)
}
#[test]
fn live_elevenlabs() {
    smoke(PRESETS[1].0, PRESETS[1].1)
}
#[test]
fn live_groq() {
    smoke(PRESETS[2].0, PRESETS[2].1)
}
#[test]
fn live_openai() {
    smoke(PRESETS[3].0, PRESETS[3].1)
}
#[test]
fn live_deepgram() {
    smoke(PRESETS[4].0, PRESETS[4].1)
}
#[test]
fn live_assemblyai() {
    smoke(PRESETS[5].0, PRESETS[5].1)
}
