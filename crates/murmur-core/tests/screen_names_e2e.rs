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
//! Screens with no English `said` (the Hebrew ones) get no `*_full` rows.
//!
//! The request carries the words as `screen_terms` too, as `engine.rs`
//! `asr_request` does, so they get the screen hotword score
//! (`SCREEN_HOTWORDS_SCORE`, or `MURMUR_SCREEN_HOTWORDS_SCORE` for a
//! sweep). `MURMUR_SCREEN_ENGINE=sherpa_online` decodes with the streaming
//! zipformer instead of Parakeet (`MURMUR_SCREEN_ASR` = its folder,
//! language `MURMUR_SCREEN_LANG`, default `he`).
//!
//! The dictionary side (2026-11-25): `dictionary` gives the screen's name
//! candidates as dictionary words instead (no screen), `dict30` a
//! 30-word dictionary of invented words unrelated to any clip (false
//! insertions on the bench), and `candidates_dict30` both the screen's
//! candidates and that dictionary. `MURMUR_PARAKEET_HOTWORDS_SCORE` sets
//! Parakeet's base hotword score for a comparison.
//!
//! Each `<mode>_ms` is that decode's wall time (2026-11-26, option A latency).
//!
//! `cargo test -p murmur-core --features wav,parakeet --release --test screen_names_e2e -- --nocapture`

use std::io::Write;
use std::path::Path;

use murmur_core::asr::{build_initial_prompt, dedupe_terms, AsrEngine, AsrRequest};
use murmur_core::audio;
use murmur_core::dictionary::{DictEntry, Dictionary};
use murmur_core::parakeet::ParakeetAsr;
use murmur_core::screen_names::name_candidates;
use murmur_core::sherpa_online::SherpaOnlineAsr;
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

/// Invented words, none in any clip or screen: a dictionary that should
/// change nothing.
const UNRELATED: [&str; 30] = [
    "Quorvel",
    "Tamsik",
    "Brannoway",
    "Elvessa",
    "Drovik",
    "Fennimore Q",
    "Hallistrade",
    "Ixtapel",
    "Jorvane",
    "Kelloptic",
    "Lunderhall",
    "Mavrix",
    "Nostrel",
    "Ophandry",
    "Pellucine",
    "Quillith",
    "Rastenbur",
    "Sovrane",
    "Thessaly Pike",
    "Ulvaren",
    "Vintrell",
    "Wexmoor",
    "Yarrowcap",
    "Zenthic",
    "Abrovel",
    "Caskadine",
    "Dulcimar",
    "Embervale",
    "Glistane",
    "Horvath Lumen",
];

/// The request `engine.rs` `asr_request` builds for this dictionary and
/// these screen names.
fn request(dict: &Dictionary, names: &[String]) -> AsrRequest {
    let mut seen = Vec::new();
    AsrRequest {
        initial_prompt: build_initial_prompt(dict, names),
        dictionary_terms: dedupe_terms(dict.vocabulary(), &mut seen),
        screen_terms: dedupe_terms(names.iter().map(String::as_str), &mut seen),
        ..Default::default()
    }
}

fn dictionary(words: impl IntoIterator<Item = impl AsRef<str>>) -> Dictionary {
    Dictionary {
        entries: words
            .into_iter()
            .map(|w| DictEntry::word(w.as_ref()))
            .collect(),
    }
}

/// The phone's cap on a capture (`ScreenWords.MAX_CHARS`).
const PHONE_MAX_CHARS: usize = 2000;

/// `own` below older messages: the bubbles (entries of more than 3 words)
/// of the other English screens, from the next one on, while the whole
/// stays within [`PHONE_MAX_CHARS`]. Empty for a non-English screen.
fn full_capture(screens: &[Value], id: &Value, own: &[String]) -> Vec<String> {
    let own_chars: usize = own.iter().map(|e| e.chars().count()).sum();
    let english: Vec<&Value> = screens
        .iter()
        .filter(|s| !s["said"].as_array().unwrap().is_empty())
        .collect();
    let Some(at) = english.iter().position(|s| &s["id"] == id) else {
        return Vec::new();
    };
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
    let engine: Box<dyn AsrEngine> = match std::env::var("MURMUR_SCREEN_ENGINE").as_deref() {
        Ok("sherpa_online") => {
            let lang = std::env::var("MURMUR_SCREEN_LANG").unwrap_or_else(|_| "he".into());
            Box::new(SherpaOnlineAsr::load(&asr, threads, Some(&lang)).expect("load zipformer"))
        }
        _ => Box::new(ParakeetAsr::load(&asr, threads).expect("load parakeet")),
    };
    let screens = screens();
    let unrelated = dictionary(UNRELATED);
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
        // As the session does for quiet speech: the Hebrew FLEURS bench
        // clips sit below the engines' silence gate (no-op for the rest).
        let pcm = murmur_core::vad::asr_level(&pcm).into_owned();
        let mut rec = json!({"id": row["id"], "set": row["set"]});
        let none = Dictionary::default();
        let cands = name_candidates(&bubbles);
        let modes = [
            ("none", request(&none, &[])),
            ("bubbles", request(&none, &bubbles)),
            ("candidates", request(&none, &cands)),
            ("bubbles_full", request(&none, &full)),
            ("candidates_full", request(&none, &name_candidates(&full))),
            ("dictionary", request(&dictionary(&cands), &[])),
            ("dict30", request(&unrelated, &[])),
            ("candidates_dict30", request(&unrelated, &cands)),
        ];
        for (mode, req) in modes {
            if mode.ends_with("_full") && full.is_empty() {
                continue;
            }
            let t0 = std::time::Instant::now();
            let text = engine.transcribe(&pcm, &req).unwrap().text;
            rec[format!("{mode}_ms")] = json!(t0.elapsed().as_millis() as u64);
            rec[mode] = json!(text);
            rec[format!("{mode}_hotwords")] = json!(murmur_core::asr::hotword_phrases(&req).len());
        }
        eprintln!("{rec}");
        writeln!(out, "{rec}").unwrap();
    }
}
