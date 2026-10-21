//! VAD / hallucination-guard measurement on real whisper.cpp (ignored by
//! default; it takes a minute). Run:
//!   cargo test --release -p flow-core --features whisper,wav --test vad_bench -- --ignored --nocapture
//! Env: FLOW_BENCH_MODELS="ggml-tiny.en.bin,ggml-base.en.bin", FLOW_BENCH_REPS=2.
//!
//! Cases: jfk.wav as is; jfk padded with 3 s of room noise each side; 3 s
//! of room noise alone (RMS ≈ 0.005, above the old whole-buffer gate).
//! Each runs with VAD on and off; the min ASR time over the reps is kept.

use std::path::PathBuf;

use flow_core::config::{AsrBackend, ModelConfig, SessionConfig};
use flow_core::{audio, FlowCore};

fn models_dir() -> PathBuf {
    std::env::var_os("FLOW_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn noise(n: usize, amp: f32, seed: u32) -> Vec<i16> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            ((x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp * 32767.0) as i16
        })
        .collect()
}

#[test]
#[ignore]
fn vad_saves_asr_time() {
    let d = models_dir();
    let Ok(jfk) = audio::read_wav_16k_mono(&d.join("jfk.wav")) else {
        eprintln!("SKIP: no jfk.wav");
        return;
    };
    let reps: usize = std::env::var("FLOW_BENCH_REPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let models = std::env::var("FLOW_BENCH_MODELS")
        .unwrap_or_else(|_| "ggml-tiny.en.bin,ggml-base.en.bin".into());
    let pad = noise(48_000, 0.004, 1);
    let padded = [pad.clone(), jfk.clone(), noise(48_000, 0.004, 2)].concat();
    let room = noise(48_000, 0.009, 3);
    let cases: [(&str, &[i16]); 3] = [
        ("jfk 11.0 s", &jfk),
        ("3 s noise + jfk + 3 s noise", &padded),
        ("3 s room noise only", &room),
    ];
    for m in models.split(',') {
        let path = d.join(m);
        if !path.exists() {
            eprintln!("SKIP: {}", path.display());
            continue;
        }
        let core = FlowCore::load_models(&ModelConfig {
            asr_backend: AsrBackend::Whisper,
            asr_path: path.to_string_lossy().into(),
            ..Default::default()
        })
        .unwrap();
        for (name, pcm) in cases {
            for vad in [false, true] {
                let mut best = u64::MAX;
                let mut last = None;
                for _ in 0..reps {
                    core.start_session(SessionConfig {
                        vad,
                        languages: vec!["en".into()],
                        ..Default::default()
                    })
                    .unwrap();
                    core.push_audio(pcm);
                    let r = core.commit();
                    best = best.min(r.timings.asr_ms);
                    last = Some(r);
                }
                let r = last.unwrap();
                eprintln!(
                    "{m} | {name:28} | vad {:3} | asr {best:5} ms | asr audio {:5} ms | {:?} {:?} | {:?}",
                    if vad { "on" } else { "off" },
                    r.timings.asr_audio_ms,
                    r.status,
                    r.empty_reason,
                    r.asr_text
                );
            }
        }
    }
}
