//! D22 equivalence bench (ignored; needs models + FLOW_AUDIO_DIR): the
//! 2026-10-23 probe's long clips (19-62 s, 700 ms gaps), whole-clip vs
//! pause-segmented decoding through `FlowCore`, real Parakeet and whisper
//! base. Gate: pooled segmented WER <= whole + 0.3 points per engine.
//! Also prints commit (tail) time with a paced push.
//!
//! FLOW_AUDIO_DIR=… FLOW_PARAKEET_DIR=… FLOW_WHISPER_BENCH_MODEL=…/ggml-base-q5_1.bin \
//!   cargo test --release -p flow-core --features whisper,parakeet,wav \
//!   --test segment_bench -- --ignored --nocapture

use std::path::PathBuf;
use std::time::{Duration, Instant};

use flow_core::config::{ModelConfig, SessionConfig};
use flow_core::FlowCore;

mod common;
use common::long_clips;

fn engines() -> Vec<(&'static str, ModelConfig)> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("FLOW_PARAKEET_DIR").map(PathBuf::from) {
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
    if let Some(m) = std::env::var_os("FLOW_WHISPER_BENCH_MODEL").map(PathBuf::from) {
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
    core: &FlowCore,
    pcm: &[i16],
    cfg: SessionConfig,
    speed: f32,
) -> flow_core::CommitResult {
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
        eprintln!("SKIP: FLOW_AUDIO_DIR not set");
        return;
    };
    let engines = engines();
    if engines.is_empty() {
        eprintln!("SKIP: set FLOW_PARAKEET_DIR and/or FLOW_WHISPER_BENCH_MODEL");
        return;
    }
    let seg = SessionConfig {
        segment_pause_ms: Some(800),
        ..Default::default()
    };
    for (name, model) in engines {
        let core = FlowCore::load_models(&model).expect("load");
        let (mut ew, mut es, mut n) = (0, 0, 0);
        for clip in clips.iter().filter(|c| c.secs() >= 15.0) {
            let whole = dictate(&core, &clip.pcm, SessionConfig::default(), 0.0);
            let segm = dictate(&core, &clip.pcm, seg.clone(), 0.0);
            let (a, w) = long_clips::wer_counts(&clip.reference, &whole.asr_text);
            let (b, _) = long_clips::wer_counts(&clip.reference, &segm.asr_text);
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
