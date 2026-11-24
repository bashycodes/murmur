//! Auto-detect accuracy on a he / en / mixed manifest (2026-11-24 probe,
//! `tools/probes/prototypes/lang_detect_2026_11_24/`). Env-gated: skips
//! (passes with a note) unless `MURMUR_LANG_MANIFEST` (manifest.jsonl from
//! the probe's `prepare.py`), `MURMUR_LID_MODEL` (multilingual ggml, e.g.
//! `ggml-tiny-q5_1.bin`) and `MURMUR_LANG_OUT` (a JSONL file to write) are
//! set. Audio and models are never committed.
//!
//! * `lang_detect_route`: every clip through `MurmurCore` (auto-detect, a
//!   Hebrew route, language ID), one row per clip with the language the
//!   result came back in. Decoders are mocks unless `MURMUR_LANG_EN_ASR`
//!   (a Parakeet folder) and `MURMUR_LANG_HE_ASR` (a zipformer folder) are
//!   set; then the rows carry real text for WER.
//! * `lang_detect_scores` (`MURMUR_LANG_SCORES=1`): the identifier's raw
//!   en/he scores for each 3 s window of each clip's speech, for comparing
//!   routing rules offline (the probe's `score.py`).
//!
//! `cargo test -p murmur-core --features whisper,wav,parakeet --release --test lang_detect_e2e -- --nocapture --test-threads 1`

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use murmur_core::config::{AsrBackend, LanguageModel, ModelConfig, SessionConfig};
use murmur_core::lid::{self, LanguageId, WhisperLid};
use murmur_core::{audio, vad, MurmurCore};
use serde_json::{json, Value};

const EN_MOCK: &str = "english";
const HE_MOCK: &str = "עברית";

struct Env {
    manifest: Vec<Value>,
    lid: String,
    out: std::fs::File,
}

fn env() -> Option<Env> {
    let (Ok(m), Ok(lid), Ok(out)) = (
        std::env::var("MURMUR_LANG_MANIFEST"),
        std::env::var("MURMUR_LID_MODEL"),
        std::env::var("MURMUR_LANG_OUT"),
    ) else {
        eprintln!("SKIP: set MURMUR_LANG_MANIFEST, MURMUR_LID_MODEL, MURMUR_LANG_OUT");
        return None;
    };
    let sets = std::env::var("MURMUR_LANG_SETS").ok();
    let manifest = std::fs::read_to_string(m)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|r| {
            sets.as_deref()
                .is_none_or(|s| s.split(',').any(|s| s == r["set"]))
        })
        .collect();
    Some(Env {
        manifest,
        lid,
        out: std::fs::File::create(out).unwrap(),
    })
}

fn threads() -> usize {
    std::env::var("MURMUR_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(4)
}

fn pcm16(row: &Value) -> Vec<i16> {
    audio::read_wav_16k_mono(Path::new(row["path"].as_str().unwrap())).unwrap()
}

#[test]
fn lang_detect_route() {
    if std::env::var("MURMUR_LANG_SCORES").is_ok() {
        return;
    }
    let Some(mut e) = env() else { return };
    let real = match (
        std::env::var("MURMUR_LANG_EN_ASR"),
        std::env::var("MURMUR_LANG_HE_ASR"),
    ) {
        (Ok(en), Ok(he)) => Some((en, he)),
        _ => None,
    };
    let (default, he) = match &real {
        Some((en, he)) => (
            (AsrBackend::Parakeet, en.clone()),
            (AsrBackend::SherpaOnline, he.clone()),
        ),
        None => (
            (
                AsrBackend::Mock {
                    text: EN_MOCK.into(),
                },
                String::new(),
            ),
            (
                AsrBackend::Mock {
                    text: HE_MOCK.into(),
                },
                String::new(),
            ),
        ),
    };
    let core = MurmurCore::load_models(&ModelConfig {
        asr_backend: default.0,
        asr_path: default.1,
        threads: threads(),
        languages: BTreeMap::from([(
            "he".to_string(),
            LanguageModel {
                asr_backend: he.0,
                asr_path: he.1,
                ..Default::default()
            },
        )]),
        lid_path: Some(e.lid.clone()),
        ..Default::default()
    })
    .unwrap();
    let (mut ok, mut n) = (0, 0);
    for row in &e.manifest {
        let pcm = pcm16(row);
        core.start_session(SessionConfig {
            segment_pause_ms: Some(800),
            ..Default::default()
        })
        .unwrap();
        for c in pcm.chunks(320) {
            core.push_audio(c);
        }
        let t = Instant::now();
        let r = core.commit();
        let got = match (&real, r.language.as_deref()) {
            (Some(_), Some(l)) => l.to_string(),
            _ if r.asr_text.starts_with(HE_MOCK) => "he".into(),
            _ if r.asr_text.starts_with(EN_MOCK) => "en".into(),
            _ => "none".into(),
        };
        n += 1;
        if got == row["expect"] {
            ok += 1;
        }
        writeln!(
            e.out,
            "{}",
            json!({
                "id": row["id"], "set": row["set"], "expect": row["expect"], "got": got,
                "text": r.asr_text, "lid_ms": r.timings.lid_ms,
                "commit_ms": t.elapsed().as_millis() as u64,
                "empty_reason": r.empty_reason,
            })
        )
        .unwrap();
    }
    println!("routed as expected: {ok}/{n}");
}

#[test]
fn lang_detect_scores() {
    if std::env::var("MURMUR_LANG_SCORES").is_err() {
        return;
    }
    let Some(mut e) = env() else { return };
    let lid = WhisperLid::load(&e.lid, threads(), true).unwrap();
    let cands = vec!["en".to_string(), "he".to_string()];
    for row in &e.manifest {
        let pcm: Vec<f32> = pcm16(row).iter().map(|&s| s as f32 / 32768.0).collect();
        let Some(sp) = vad::detect(&pcm) else {
            writeln!(e.out, "{}", json!({"id": row["id"], "windows": []})).unwrap();
            continue;
        };
        // The engine's windows (D106): the first from the speech start, then
        // every further window that holds speech, and a long enough tail.
        // All of them, without the engine's skip of undecidable ones.
        let mut windows = Vec::new();
        let mut next = Some(lid::first_window(sp.start..sp.end));
        while let Some(r) = next {
            next = lid::window_after(r.end, sp.end, true);
            if r.start > sp.start && vad::detect(&pcm[r.clone()]).is_none() {
                continue;
            }
            let x = vad::asr_level(&pcm[r.clone()]);
            let sc = lid.scores(&x, &cands).unwrap();
            let p = |l: &str| sc.iter().find(|(c, _)| c == l).map_or(0.0, |(_, p)| *p);
            windows.push(json!([p("en"), p("he"), r.len() as f32 / 16000.0]));
        }
        writeln!(
            e.out,
            "{}",
            json!({"id": row["id"], "set": row["set"], "expect": row["expect"],
                   "speech_s": (sp.end - sp.start) as f32 / 16000.0, "windows": windows})
        )
        .unwrap();
    }
}
