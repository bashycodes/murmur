//! On-screen words as Parakeet hotwords (2026-11-24 probe,
//! `tools/probes/prototypes/screen_names_2026_11_24/`). Env-gated: skips
//! (passes with a note) unless `MURMUR_SCREEN_MANIFEST` (the probe's
//! manifest.jsonl), `MURMUR_SCREEN_ASR` (a Parakeet folder) and
//! `MURMUR_SCREEN_OUT` (a JSONL file to write) are set. Audio and models
//! are never committed.
//!
//! Each clip is decoded with the hotwords the core would build
//! (`build_initial_prompt` → `hotword_phrases`) from its chat screen
//! (`fixtures/screen-names/chat-screens.json`): `none` (no screen words),
//! `bubbles` (the screen as the phone sends it, what the core used before
//! `screen_names`) and `candidates` (`name_candidates` of it). The fixture
//! screens are shorter than the prompt budget (900 chars), so `*_full`
//! repeats both with a full capture: older messages (the other screens'
//! bubbles, names and all) above the screen, up to the phone's 2000-char
//! cap (`ScreenWords.MAX_CHARS`), as a chat scrolled further would give.
//!
//! `cargo test -p murmur-core --features wav,parakeet --release --test screen_names_e2e -- --nocapture`

use std::io::Write;
use std::path::Path;

use murmur_core::asr::{build_initial_prompt, AsrEngine, AsrRequest};
use murmur_core::audio;
use murmur_core::dictionary::Dictionary;
use murmur_core::parakeet::ParakeetAsr;
use murmur_core::screen_names::name_candidates;
use serde_json::{json, Value};

fn screens() -> Vec<Value> {
    let raw = include_str!("../fixtures/screen-names/chat-screens.json");
    let v: Value = serde_json::from_str(raw).unwrap();
    v["screens"].as_array().unwrap().clone()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

/// The phone's cap on a capture (`ScreenWords.MAX_CHARS`).
const PHONE_MAX_CHARS: usize = 2000;

/// `own` below older messages: the bubbles (entries of more than 3 words)
/// of the other English screens, from the next one on, while the whole
/// stays within [`PHONE_MAX_CHARS`].
fn full_capture(screens: &[Value], id: &Value, own: &[String]) -> Vec<String> {
    let own_chars: usize = own.iter().map(|e| e.chars().count()).sum();
    let english: Vec<&Value> = screens
        .iter()
        .filter(|s| !s["said"].as_array().unwrap().is_empty())
        .collect();
    let at = english.iter().position(|s| &s["id"] == id).unwrap();
    let mut older = Vec::new();
    let mut chars = own_chars;
    'fill: for k in 1..english.len() {
        for e in strings(&english[(at + k) % english.len()]["screen"]) {
            if e.split_whitespace().count() <= 3 {
                continue;
            }
            if chars + e.chars().count() > PHONE_MAX_CHARS {
                break 'fill;
            }
            chars += e.chars().count();
            older.push(e);
        }
    }
    older.extend(own.iter().cloned());
    older
}

#[test]
fn screen_words_as_hotwords() {
    let (Ok(m), Ok(asr), Ok(out)) = (
        std::env::var("MURMUR_SCREEN_MANIFEST"),
        std::env::var("MURMUR_SCREEN_ASR"),
        std::env::var("MURMUR_SCREEN_OUT"),
    ) else {
        eprintln!("SKIP: set MURMUR_SCREEN_MANIFEST, MURMUR_SCREEN_ASR, MURMUR_SCREEN_OUT");
        return;
    };
    let threads = std::env::var("MURMUR_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(4);
    let engine = ParakeetAsr::load(&asr, threads).expect("load parakeet");
    let screens = screens();
    let mut out = std::fs::File::create(out).unwrap();
    for line in std::fs::read_to_string(m).unwrap().lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let screen = screens
            .iter()
            .find(|s| s["id"] == row["screen"])
            .expect("screen id");
        let bubbles = strings(&screen["screen"]);
        let full = full_capture(&screens, &screen["id"], &bubbles);
        let pcm16 = audio::read_wav_16k_mono(Path::new(row["path"].as_str().unwrap())).unwrap();
        let pcm: Vec<f32> = pcm16.iter().map(|&s| s as f32 / 32768.0).collect();
        let mut rec = json!({"id": row["id"], "set": row["set"]});
        for (mode, words) in [
            ("none", Vec::new()),
            ("bubbles", bubbles.clone()),
            ("candidates", name_candidates(&bubbles)),
            ("bubbles_full", full.clone()),
            ("candidates_full", name_candidates(&full)),
        ] {
            let req = AsrRequest {
                initial_prompt: build_initial_prompt(&Dictionary::default(), &words),
                ..Default::default()
            };
            let text = engine.transcribe(&pcm, &req).unwrap().text;
            rec[mode] = json!(text);
            rec[format!("{mode}_hotwords")] = json!(murmur_core::asr::hotword_phrases(&req).len());
        }
        eprintln!("{rec}");
        writeln!(out, "{rec}").unwrap();
    }
}
