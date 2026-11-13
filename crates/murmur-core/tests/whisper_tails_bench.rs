//! What reaches the hallucination guard: raw whisper.cpp output on windows
//! with little or no speech (ignored by default; minutes per model). Used to
//! measure decoder settings such as `entropy_thold` / `logprob_thold`
//! (`docs/findings/2026-11-13-whisper-decoder-thresholds.md`). Run:
//!   MURMUR_BENCH_AUDIO=<dir of the 40 LibriSpeech wavs> MURMUR_BENCH_OUT=out.jsonl \
//!   cargo test --release -p murmur-core --features whisper,wav --test whisper_tails_bench -- --ignored --nocapture
//! Env: MURMUR_MODELS_DIR (models), MURMUR_BENCH_MODELS="ggml-tiny.en.bin,…".
//!
//! Cases, one JSON line each (score with
//! `tools/probes/prototypes/whisper_thresholds_2026_11_13/score.py`):
//! * `noise`: synthetic white and brown noise, 3 levels × 3 lengths × 2
//!   seeds, no speech. Decoded by the engine directly (no VAD), as with
//!   VAD off or a VAD false positive.
//! * `blip`: 250 ms cut from the middle of each clip (a syllable, not a
//!   word) inside 3 s of quiet noise: a breath or key-press tail that a VAD
//!   might pass. Engine direct; any text here is a hallucination or a guess.
//! * `tail`: each clip of the 40-clip set followed by 4 s of quiet noise
//!   (the "mostly silent window" where stock tails appear), engine direct.
//! * `quiet`: each clip scaled to about −60 dBFS, through `MurmurCore` with
//!   VAD on (the D16 quiet-speech path: relative bar + `asr_level`).
//!
//! Per line: the text, Whisper's no-speech probability, and what
//! `hallucination.rs` makes of it (`stock`: the whole text is a stock
//! phrase or credit; `tail`: a trailing stock sentence after real text;
//! `leak`: non-empty text the guard would let through with VAD off).

use std::io::Write;
use std::path::PathBuf;

use murmur_core::asr::{AsrEngine, AsrRequest};
use murmur_core::config::{AsrBackend, ModelConfig, SessionConfig};
use murmur_core::hallucination::{is_hallucination, strip_trailing_stock, Evidence};
use murmur_core::whisper::WhisperEngine;
use murmur_core::{audio, MurmurCore};

/// xorshift white noise, peak `amp` (full scale 1.0).
fn white(n: usize, amp: f32, seed: u32) -> Vec<f32> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

/// Leaky-integrated white noise (rumble, room tone), rescaled to the same
/// RMS as white noise of peak `amp`.
fn brown(n: usize, amp: f32, seed: u32) -> Vec<f32> {
    let w = white(n, 1.0, seed);
    let mut y = 0.0f32;
    let v: Vec<f32> = w
        .iter()
        .map(|s| {
            y = 0.98 * y + s;
            y
        })
        .collect();
    let gain = rms(&white(n, amp, seed)) / rms(&v).max(1e-9);
    v.iter().map(|s| s * gain).collect()
}

fn rms(p: &[f32]) -> f32 {
    (p.iter().map(|s| s * s).sum::<f32>() / p.len().max(1) as f32).sqrt()
}

fn verdict(text: &str, no_speech_prob: Option<f32>) -> serde_json::Value {
    let t = text.trim();
    let stock = !t.is_empty()
        && is_hallucination(
            t,
            Evidence {
                speech_ms: Some(0),
                no_speech_prob: None,
            },
        );
    let caught_vad_off = is_hallucination(
        t,
        Evidence {
            speech_ms: None,
            no_speech_prob,
        },
    );
    serde_json::json!({
        "stock": stock,
        "tail": strip_trailing_stock(t).is_some(),
        "leak": !t.is_empty() && !caught_vad_off,
    })
}

#[test]
#[ignore]
fn whisper_tails() {
    let models_dir = PathBuf::from(std::env::var("MURMUR_MODELS_DIR").expect("MURMUR_MODELS_DIR"));
    let audio_dir = PathBuf::from(std::env::var("MURMUR_BENCH_AUDIO").expect("MURMUR_BENCH_AUDIO"));
    let out_path = std::env::var("MURMUR_BENCH_OUT").expect("MURMUR_BENCH_OUT");
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tools/probes/bench/asr_manifest_extended.json"
    ))
    .unwrap();
    let clips: Vec<(String, String, Vec<f32>)> = manifest["utterances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            let id = u["id"].as_str().unwrap().to_string();
            let pcm = audio::read_wav_16k_mono(&audio_dir.join(format!("{id}.wav")))
                .unwrap()
                .iter()
                .map(|&s| s as f32 / 32768.0)
                .collect();
            (id, u["ref"].as_str().unwrap().to_string(), pcm)
        })
        .collect();
    let models = std::env::var("MURMUR_BENCH_MODELS").unwrap_or_else(|_| "ggml-tiny.en.bin".into());
    let mut out = std::fs::File::create(&out_path).unwrap();
    let en = AsrRequest {
        language: Some("en".into()),
        ..Default::default()
    };
    for m in models.split(',') {
        let path = models_dir.join(m);
        let eng = WhisperEngine::load(path.to_str().unwrap(), 4)
            .unwrap()
            .with_fit_ctx(true);
        let mut emit = |case: &str,
                        id: &str,
                        r: &str,
                        text: &str,
                        nsp: Option<f32>,
                        extra: serde_json::Value| {
            let mut line = serde_json::json!({"model": m, "case": case, "id": id, "ref": r,
                "hyp": text, "no_speech_prob": nsp, "v": verdict(text, nsp)});
            if let (Some(o), Some(e)) = (line.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            writeln!(out, "{line}").unwrap();
        };

        for (kind, gen) in [
            ("white", white as fn(usize, f32, u32) -> Vec<f32>),
            ("brown", brown),
        ] {
            for amp in [0.006f32, 0.012, 0.024] {
                for secs in [1.5f32, 3.0, 6.0] {
                    for seed in [1u32, 2] {
                        let pcm = gen((secs * 16_000.0) as usize, amp, seed);
                        let o = eng.transcribe(&pcm, &en).unwrap();
                        let id = format!("{kind}-{amp}-{secs}s-{seed}");
                        emit(
                            "noise",
                            &id,
                            "",
                            &o.text,
                            o.no_speech_prob,
                            serde_json::json!({"rms": rms(&pcm)}),
                        );
                    }
                }
            }
        }

        for (i, (id, _, pcm)) in clips.iter().enumerate() {
            let mut v = white(48_000, 0.004, 11 + i as u32);
            let mid = pcm.len() / 2;
            for (k, s) in pcm[mid..mid + 4_000].iter().enumerate() {
                v[16_000 + k] += s;
            }
            let o = eng.transcribe(&v, &en).unwrap();
            emit(
                "blip",
                id,
                "",
                &o.text,
                o.no_speech_prob,
                serde_json::json!({}),
            );
        }

        for (id, r, pcm) in &clips {
            let mut v = pcm.clone();
            v.extend(white(64_000, 0.004, 7));
            let o = eng.transcribe(&v, &en).unwrap();
            emit(
                "tail",
                id,
                r,
                &o.text,
                o.no_speech_prob,
                serde_json::json!({}),
            );
        }
        drop(eng);

        let core = MurmurCore::load_models(&ModelConfig {
            asr_backend: AsrBackend::Whisper,
            asr_path: path.to_string_lossy().into(),
            threads: 4,
            ..Default::default()
        })
        .unwrap();
        for (id, r, pcm) in &clips {
            // ≈ −60 dBFS RMS: 30 PCM16.
            let g = 30.0 / 32768.0 / rms(pcm);
            let q: Vec<i16> = pcm
                .iter()
                .map(|s| (s * g * 32768.0).round() as i16)
                .collect();
            core.start_session(SessionConfig {
                vad: true,
                languages: vec!["en".into()],
                ..Default::default()
            })
            .unwrap();
            core.push_audio(&q);
            let res = core.commit();
            emit(
                "quiet",
                id,
                r,
                &res.asr_text,
                None,
                serde_json::json!({"status": format!("{:?}", res.status), "empty_reason": res.empty_reason}),
            );
        }
        eprintln!("{m}: done");
    }
}
