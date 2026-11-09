//! D22 equivalence bench (ignored; needs models + MURMUR_AUDIO_DIR): the
//! 2026-10-23 probe's long clips (19-62 s, 700 ms gaps), whole-clip vs
//! pause-segmented decoding through `MurmurCore`, real Parakeet and whisper
//! base. Gate: pooled segmented WER <= whole + 0.3 points per engine.
//! Also prints commit (tail) time with a paced push.
//!
//! MURMUR_AUDIO_DIR=… MURMUR_PARAKEET_DIR=… MURMUR_WHISPER_BENCH_MODEL=…/ggml-base-q5_1.bin \
//!   cargo test --release -p murmur-core --features whisper,parakeet,wav \
//!   --test segment_bench -- --ignored --nocapture

use std::path::PathBuf;
use std::time::{Duration, Instant};

use murmur_core::config::{ModelConfig, SessionConfig};
use murmur_core::MurmurCore;

mod common;
use common::long_clips;

fn engines() -> Vec<(&'static str, ModelConfig)> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("MURMUR_PARAKEET_DIR").map(PathBuf::from) {
        if d.join("tokens.txt").is_file() {
            v.push((
                "parakeet",
                serde_json::from_value(serde_json::json!({
                    "asr_engine": "parakeet", "asr_path": d, "threads": 4
                }))
                .unwrap(),
            ));
        }
    }
    if let Some(m) = std::env::var_os("MURMUR_WHISPER_BENCH_MODEL").map(PathBuf::from) {
        if m.is_file() {
            v.push((
                "whisper",
                ModelConfig {
                    asr_path: m.to_string_lossy().into(),
                    threads: 4,
                    ..Default::default()
                },
            ));
        }
    }
    v
}

/// Push at `speed`× real time (0 = at once), then commit.
fn dictate(
    core: &MurmurCore,
    pcm: &[i16],
    cfg: SessionConfig,
    speed: f32,
) -> murmur_core::CommitResult {
    core.start_session(SessionConfig {
        ring_capacity_samples: pcm.len() + 16_000,
        ..cfg
    })
    .unwrap();
    let t0 = Instant::now();
    for (i, c) in pcm.chunks(1600).enumerate() {
        core.push_audio(c);
        if speed > 0.0 {
            let due = Duration::from_secs_f32(0.1 * (i + 1) as f32 / speed);
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
    }
    core.commit()
}

#[test]
#[ignore]
fn segmented_wer_matches_whole_clip() {
    let Some(clips) = long_clips::audio_dir().and_then(|d| long_clips::build(&d)) else {
        eprintln!("SKIP: MURMUR_AUDIO_DIR not set");
        return;
    };
    let engines = engines();
    if engines.is_empty() {
        eprintln!("SKIP: set MURMUR_PARAKEET_DIR and/or MURMUR_WHISPER_BENCH_MODEL");
        return;
    }
    let seg = SessionConfig {
        segment_pause_ms: Some(800),
        ..Default::default()
    };
    for (name, model) in engines {
        let core = MurmurCore::load_models(&model).expect("load");
        let (mut ew, mut es, mut n) = (0, 0, 0);
        for clip in clips.iter().filter(|c| c.secs() >= 15.0) {
            let whole = dictate(&core, &clip.pcm, SessionConfig::default(), 0.0);
            let segm = dictate(&core, &clip.pcm, seg.clone(), 0.0);
            let (a, w) = long_clips::wer_counts(&clip.reference, &whole.asr_text);
            let (b, _) = long_clips::wer_counts(&clip.reference, &segm.asr_text);
            if std::env::var_os("MURMUR_BENCH_DUMP").is_some() && a != b {
                eprintln!(
                    "REF   {}\nWHOLE {}\nSEG   {}",
                    clip.reference, whole.asr_text, segm.asr_text
                );
            }
            ew += a;
            es += b;
            n += w;
            eprintln!(
                "{name} {:5.1} s: whole WER {:5.2}% asr {:5} ms | segmented {:5.2}% {} pieces",
                clip.secs(),
                100.0 * a as f32 / w as f32,
                whole.timings.asr_ms,
                100.0 * b as f32 / w as f32,
                segm.timings.segments
            );
        }
        let (pw, ps) = (100.0 * ew as f32 / n as f32, 100.0 * es as f32 / n as f32);
        eprintln!("{name}: pooled WER whole {pw:.2}% segmented {ps:.2}% ({n} words)");
        // Commit (release -> text) with a 4x real-time push, longest clips.
        for clip in clips.iter().filter(|c| c.secs() >= 25.0) {
            let w = dictate(&core, &clip.pcm, SessionConfig::default(), 4.0);
            let s = dictate(&core, &clip.pcm, seg.clone(), 4.0);
            eprintln!(
                "{name} {:5.1} s paced 4x: commit whole {} ms | segmented {} ms (tail asr {} ms, pre {} ms, wait {} ms)",
                clip.secs(),
                w.timings.total_ms,
                s.timings.total_ms,
                s.timings.asr_ms,
                s.timings.pre_asr_ms,
                s.timings.wait_ms
            );
        }
        assert!(
            ps <= pw + 0.3,
            "{name}: segmented {ps:.2}% vs whole {pw:.2}%"
        );
    }
}

/// Punctuated-token error of `hyp` against `reference` (exact tokens:
/// case and punctuation count), and the sentence ends ([.?!]) each has.
fn punct_counts(reference: &str, hyp: &str) -> (usize, usize, usize, usize) {
    let (r, h): (Vec<&str>, Vec<&str>) = (
        reference.split_whitespace().collect(),
        hyp.split_whitespace().collect(),
    );
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(rw != hw))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    let ends = |v: &[&str]| v.iter().filter(|w| w.ends_with(['.', '?', '!'])).count();
    (prev[h.len()], r.len(), ends(&r), ends(&h))
}

/// Agent W's regression: pieces punctuated alone turn every mid-sentence
/// pause into a sentence break. Compares the segmented output with the
/// whole-clip output (punctuation and case included) for three join
/// strategies (`MURMUR_SEGMENT_JOIN`): plain (no fix), smooth (text-only
/// heuristic) and bridge (decode around each cut; the product). Sets:
/// jfk at 800 ms; every manifest utterance >= 6 s cut at 300 ms pauses
/// with 2 s minimum pieces (forces mid-sentence cuts); the long probe
/// clips at 800 ms (cuts at real sentence ends, which must survive).
#[test]
#[ignore]
fn segmented_punctuation_matches_whole_clip() {
    let Some(audio) = long_clips::audio_dir() else {
        eprintln!("SKIP: MURMUR_AUDIO_DIR not set");
        return;
    };
    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(long_clips::manifest()).unwrap()).unwrap();
    let mut short: Vec<Vec<i16>> = m["utterances"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|u| u["dur_s"].as_f64().unwrap_or(0.0) >= 6.0)
        .filter_map(|u| {
            murmur_core::audio::read_wav_16k_mono(&audio.join(format!("{}.wav", u["id"].as_str()?)))
                .ok()
        })
        .collect();
    let jfk = std::env::var_os("MURMUR_MODELS_DIR")
        .map(|d| PathBuf::from(d).join("jfk.wav"))
        .and_then(|p| murmur_core::audio::read_wav_16k_mono(&p).ok());
    let long: Vec<Vec<i16>> = long_clips::build(&audio)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.secs() >= 15.0)
        .map(|c| c.pcm)
        .collect();
    let sets: Vec<(&str, Vec<Vec<i16>>, SessionConfig)> = vec![
        (
            "jfk@800",
            jfk.into_iter().collect(),
            SessionConfig {
                segment_pause_ms: Some(800),
                ..Default::default()
            },
        ),
        (
            "utterances@300/2s",
            std::mem::take(&mut short),
            SessionConfig {
                segment_pause_ms: Some(300),
                segment_min_ms: Some(2000),
                ..Default::default()
            },
        ),
        (
            "long@800",
            long,
            SessionConfig {
                segment_pause_ms: Some(800),
                ..Default::default()
            },
        ),
    ];
    for (name, model) in engines() {
        let core = MurmurCore::load_models(&model).expect("load");
        for (set, clips, cfg) in &sets {
            let wholes: Vec<String> = clips
                .iter()
                .map(|p| dictate(&core, p, SessionConfig::default(), 0.0).asr_text)
                .collect();
            for mode in ["plain", "smooth", "bridge"] {
                // SAFETY: this ignored bench is the only test in its binary
                // that reads the variable, and it runs alone.
                unsafe { std::env::set_var("MURMUR_SEGMENT_JOIN", mode) };
                let (mut e, mut n, mut exact, mut cuts) = (0, 0, 0, 0);
                let (mut ends_w, mut ends_s) = (0, 0);
                for (p, w) in clips.iter().zip(&wholes) {
                    let s = dictate(&core, p, cfg.clone(), 0.0);
                    cuts += s.timings.segments.saturating_sub(1);
                    let (a, b, c, d) = punct_counts(w, &s.asr_text);
                    e += a;
                    n += b;
                    ends_w += c;
                    ends_s += d;
                    exact += usize::from(*w == s.asr_text);
                }
                eprintln!(
                    "{name} {set:18} {mode:6}: punct-token err vs whole {:5.2}% | exact {exact}/{} | sentence ends whole {ends_w} seg {ends_s} | {cuts} cuts",
                    100.0 * e as f32 / n.max(1) as f32,
                    clips.len()
                );
            }
            unsafe { std::env::remove_var("MURMUR_SEGMENT_JOIN") };
        }
    }
}
